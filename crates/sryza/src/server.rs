use crate::{
    authentication::ServerIdentity,
    config, configuration,
    protocol::{self, take_string, take_varint, GAME_VERSION, PROTOCOL_VERSION},
    scheduler::TickScheduler,
};
use anyhow::{ensure, Context, Result};
use rand::RngCore;
use serde_json::json;
use std::{net::IpAddr, path::Path, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    task::JoinSet,
};
use tracing::{info, warn};

const STATUS_STATE: i32 = 1;
const LOGIN_STATE: i32 = 2;

pub async fn run(root: &Path) -> Result<()> {
    let cfg = config::load(root)?;
    let identity = if cfg.server.online_mode {
        Some(Arc::new(ServerIdentity::load_or_create(
            &root.join("config").join("server-key.pk8"),
        )?))
    } else {
        None
    };
    let listener = TcpListener::bind(cfg.network.listen).await?;
    let slots = Arc::new(Semaphore::new(cfg.resources.max_packet_queue as usize));
    info!(address = %cfg.network.listen, version = GAME_VERSION, "SryzaMC accepting status, ping, and guarded login connections");
    tokio::spawn(TickScheduler::new(cfg.performance.target_tps).run_clock());
    let mut connections = JoinSet::new();
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                info!("shutdown signal received; stopping listener");
                break;
            }
            result = listener.accept() => {
                let (stream, peer) = result?;
                let permit = match slots.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        warn!(%peer, "connection queue full");
                        continue;
                    }
                };
                let config = cfg.clone();
                let identity = identity.clone();
                connections.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle(stream, &config, identity.as_deref(), Some(peer.ip())).await {
                        warn!(%peer, %error, "connection closed");
                    }
                });
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                if let Err(error) = result {
                    warn!(%error, "connection task failed");
                }
            }
        }
    }
    connections.shutdown().await;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("register SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn handle(
    mut stream: TcpStream,
    config: &config::Config,
    identity: Option<&ServerIdentity>,
    remote_ip: Option<IpAddr>,
) -> Result<()> {
    let max = config.network.max_packet_bytes;
    let handshake = protocol::read_frame(&mut stream, max)
        .await
        .context("read handshake frame")?;
    let mut cursor = 0;
    ensure!(
        take_varint(&handshake, &mut cursor)? == 0,
        "expected handshake packet"
    );
    let client_protocol = take_varint(&handshake, &mut cursor)?;
    let _server_address = take_string(&handshake, &mut cursor, 255)?;
    let port_end = cursor
        .checked_add(2)
        .context("handshake port offset overflow")?;
    let port = u16::from_be_bytes(
        handshake
            .get(cursor..port_end)
            .context("truncated handshake port")?
            .try_into()?,
    );
    cursor = port_end;
    let next_state = take_varint(&handshake, &mut cursor)?;
    ensure!(
        cursor == handshake.len(),
        "unexpected trailing handshake bytes"
    );
    ensure!(port != 0, "client sent an empty server port");

    match next_state {
        STATUS_STATE => status_session(&mut stream, config, max).await,
        LOGIN_STATE => {
            login_session(
                &mut stream,
                config,
                max,
                client_protocol,
                identity,
                remote_ip,
            )
            .await
        }
        other => anyhow::bail!("unsupported handshake intent {other}"),
    }
}

async fn login_session(
    stream: &mut TcpStream,
    config: &config::Config,
    max: usize,
    client_protocol: i32,
    identity: Option<&ServerIdentity>,
    remote_ip: Option<IpAddr>,
) -> Result<()> {
    ensure!(
        client_protocol == PROTOCOL_VERSION,
        "login protocol mismatch: expected {PROTOCOL_VERSION}, got {client_protocol}"
    );
    let start_packet = tokio::time::timeout(
        Duration::from_secs(config.security.auth_timeout_seconds.max(1) as u64),
        protocol::read_frame(stream, max),
    )
    .await
    .context("Login Start timed out")?
    .context("read Login Start")?;
    let login = protocol::LoginStart::decode(&start_packet)?;

    let mut send_cipher = None;
    let mut receive_cipher = None;
    let profile = if config.server.online_mode {
        let identity = identity.context("online mode requires a persistent RSA identity")?;
        let mut verify_token = [0u8; 4];
        rand::rngs::OsRng.fill_bytes(&mut verify_token);
        let request = identity.encryption_request(&verify_token)?;
        protocol::write_frame(stream, &request)
            .await
            .context("send Encryption Request")?;
        let key_response = tokio::time::timeout(
            Duration::from_secs(config.security.auth_timeout_seconds.max(1) as u64),
            protocol::read_frame(stream, max),
        )
        .await
        .context("Encryption Response timed out")?
        .context("read Encryption Response")?;
        let secret = identity.decrypt_encryption_response(&key_response, &verify_token)?;
        send_cipher = Some(protocol::MinecraftCipher::new(&secret)?);
        receive_cipher = Some(protocol::MinecraftCipher::new(&secret)?);
        match identity.verify(&login.username, &secret, remote_ip).await {
            Ok(profile) => profile,
            Err(error) => {
                let mut packet = protocol::encode_varint(0);
                protocol::put_string(
                    &json!({"text":"Minecraft account verification failed."}).to_string(),
                    &mut packet,
                )?;
                protocol::write_wire_frame(stream, &packet, send_cipher.as_mut().unwrap()).await?;
                return Err(error).context("verify authenticated Minecraft account");
            }
        }
    } else {
        protocol::GameProfile {
            uuid: protocol::offline_uuid(&login.username),
            username: login.username.clone(),
            properties: Vec::new(),
        }
    };
    let session_id = *uuid::Uuid::new_v4().as_bytes();
    let threshold = config.network.compression_threshold;
    let compression_enabled = threshold >= 0;

    if compression_enabled {
        let mut set_compression = protocol::encode_varint(3);
        set_compression.extend(protocol::encode_varint(threshold));
        write_connection_packet(stream, &set_compression, send_cipher.as_mut(), None)
            .await
            .context("send Set Compression")?;
    }

    let login_success = protocol::encode_login_success(&profile, &session_id)?;
    write_connection_packet(
        stream,
        &login_success,
        send_cipher.as_mut(),
        compression_enabled.then_some(threshold),
    )
    .await
    .context("send Login Success")?;

    let acknowledgement = read_connection_packet(
        stream,
        max,
        receive_cipher.as_mut(),
        compression_enabled.then_some(threshold),
    )
    .await
    .context("read Login Acknowledged")?;
    let mut offset = 0;
    ensure!(
        protocol::take_varint(&acknowledgement, &mut offset)? == 3
            && offset == acknowledgement.len(),
        "expected Login Acknowledged"
    );

    configuration_session(
        stream,
        max,
        config.security.auth_timeout_seconds,
        send_cipher.as_mut(),
        receive_cipher.as_mut(),
        compression_enabled.then_some(threshold),
    )
    .await?;
    info!(player = %profile.username, online_mode = config.server.online_mode, "completed Configuration state; Play world spawning is not implemented");
    Ok(())
}

#[allow(clippy::needless_option_as_deref, clippy::option_as_ref_deref)]
async fn configuration_session(
    stream: &mut TcpStream,
    max: usize,
    timeout_seconds: u32,
    send_cipher: Option<&mut protocol::MinecraftCipher>,
    receive_cipher: Option<&mut protocol::MinecraftCipher>,
    compression: Option<i32>,
) -> Result<()> {
    let mut send_cipher = send_cipher;
    let mut receive_cipher = receive_cipher;
    write_connection_packet(
        stream,
        &configuration::select_known_packs(&[])?,
        send_cipher.as_mut().map(|cipher| &mut **cipher),
        compression,
    )
    .await
    .context("send Select Known Packs")?;
    let known_packs = tokio::time::timeout(
        Duration::from_secs(timeout_seconds.max(1) as u64),
        read_connection_packet(
            stream,
            max,
            receive_cipher.as_mut().map(|cipher| &mut **cipher),
            compression,
        ),
    )
    .await
    .context("Select Known Packs response timed out")??;
    let _client_packs = configuration::validate_known_packs_response(&known_packs)?;

    let keep_alive_id = rand::rngs::OsRng.next_u64() as i64;
    write_connection_packet(
        stream,
        &configuration::keep_alive(keep_alive_id),
        send_cipher.as_deref_mut(),
        compression,
    )
    .await
    .context("send Configuration Keep Alive")?;
    let keep_alive_response = tokio::time::timeout(
        Duration::from_secs(timeout_seconds.max(1) as u64),
        read_connection_packet(stream, max, receive_cipher.as_deref_mut(), compression),
    )
    .await
    .context("Configuration Keep Alive timed out")??;
    configuration::validate_keep_alive(&keep_alive_response, keep_alive_id)
        .context("validate Configuration Keep Alive")?;

    write_connection_packet(
        stream,
        &configuration::update_enabled_features(&["minecraft:vanilla"])?,
        send_cipher.as_deref_mut(),
        compression,
    )
    .await
    .context("send enabled feature flags")?;
    write_connection_packet(
        stream,
        &configuration::update_tags_empty(),
        send_cipher.as_deref_mut(),
        compression,
    )
    .await
    .context("send empty tag update")?;
    write_connection_packet(
        stream,
        &configuration::finish_configuration(),
        send_cipher.as_deref_mut(),
        compression,
    )
    .await
    .context("send Finish Configuration")?;
    let finish_ack = tokio::time::timeout(
        Duration::from_secs(timeout_seconds.max(1) as u64),
        read_connection_packet(stream, max, receive_cipher.as_deref_mut(), compression),
    )
    .await
    .context("Finish Configuration acknowledgement timed out")??;
    configuration::validate_finish_acknowledgement(&finish_ack)
        .context("validate Finish Configuration acknowledgement")?;
    Ok(())
}

async fn read_connection_packet(
    stream: &mut TcpStream,
    max: usize,
    cipher: Option<&mut protocol::MinecraftCipher>,
    compression: Option<i32>,
) -> Result<Vec<u8>> {
    let frame = match cipher {
        Some(cipher) => protocol::read_wire_frame(stream, max, cipher).await?,
        None => protocol::read_frame(stream, max).await?,
    };
    match compression {
        Some(threshold) => protocol::decode_compressed_payload(&frame, max, threshold),
        None => Ok(frame),
    }
}

async fn write_connection_packet(
    stream: &mut TcpStream,
    packet: &[u8],
    cipher: Option<&mut protocol::MinecraftCipher>,
    compression: Option<i32>,
) -> Result<()> {
    let frame_payload = match compression {
        Some(threshold) => protocol::encode_compressed_payload(packet, threshold)?,
        None => packet.to_vec(),
    };
    match cipher {
        Some(cipher) => protocol::write_wire_frame(stream, &frame_payload, cipher).await,
        None => protocol::write_frame(stream, &frame_payload).await,
    }
}

async fn status_session(stream: &mut TcpStream, config: &config::Config, max: usize) -> Result<()> {
    let request = protocol::read_frame(stream, max)
        .await
        .context("read status request")?;
    let mut cursor = 0;
    ensure!(
        take_varint(&request, &mut cursor)? == 0 && cursor == request.len(),
        "invalid status request"
    );

    let response = json!({
        "version": { "name": GAME_VERSION, "protocol": PROTOCOL_VERSION },
        "players": { "max": config.server.max_players, "online": 0, "sample": [] },
        "description": { "text": config.server.motd }
    });
    let json = serde_json::to_vec(&response)?;
    let mut packet = protocol::encode_varint(0);
    protocol::put_string(std::str::from_utf8(&json)?, &mut packet)?;
    protocol::write_frame(stream, &packet)
        .await
        .context("write status response")?;

    let ping = protocol::read_frame(stream, max)
        .await
        .context("read status ping")?;
    ensure!(ping.len() == 9, "invalid status ping length");
    let (packet_id, used) = protocol::decode_varint(&ping)?;
    ensure!(packet_id == 1 && used == 1, "expected status ping packet");
    let mut pong = protocol::encode_varint(1);
    pong.extend_from_slice(&ping[used..]);
    protocol::write_frame(stream, &pong)
        .await
        .context("write status pong")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn answers_status_and_echoes_ping() {
        let config = config::Config::default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_config = config.clone();
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            handle(socket, &server_config, None, None).await.unwrap();
        });
        let mut client = TcpStream::connect(address).await.unwrap();

        let request = protocol::encode_varint(0);
        let mut handshake = protocol::encode_varint(0);
        handshake.extend(protocol::encode_varint(PROTOCOL_VERSION));
        protocol::put_string("localhost", &mut handshake).unwrap();
        handshake.extend(25565u16.to_be_bytes());
        handshake.extend(protocol::encode_varint(STATUS_STATE));
        protocol::write_frame(&mut client, &handshake)
            .await
            .unwrap();
        protocol::write_frame(&mut client, &request).await.unwrap();

        let response = protocol::read_frame(&mut client, 4096).await.unwrap();
        let (id, used) = protocol::decode_varint(&response).unwrap();
        assert_eq!(id, 0);
        let mut offset = used;
        let text = protocol::take_string(&response, &mut offset, 2048).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["version"]["protocol"], PROTOCOL_VERSION);
        assert_eq!(parsed["players"]["max"], 20);

        let token = 0x0102030405060708i64;
        let mut ping = protocol::encode_varint(1);
        ping.extend(token.to_be_bytes());
        protocol::write_frame(&mut client, &ping).await.unwrap();
        let pong = protocol::read_frame(&mut client, 4096).await.unwrap();
        assert_eq!(pong, ping);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn offline_login_completes_configuration_state() {
        let mut config = config::Config::default();
        config.server.online_mode = false;
        config.network.compression_threshold = 1;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_config = config.clone();
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            handle(socket, &server_config, None, None).await.unwrap();
        });
        let mut client = TcpStream::connect(address).await.unwrap();

        let mut handshake = protocol::encode_varint(0);
        handshake.extend(protocol::encode_varint(PROTOCOL_VERSION));
        protocol::put_string("localhost", &mut handshake).unwrap();
        handshake.extend(25565u16.to_be_bytes());
        handshake.extend(protocol::encode_varint(LOGIN_STATE));
        protocol::write_frame(&mut client, &handshake)
            .await
            .unwrap();

        let mut login_start = protocol::encode_varint(0);
        protocol::put_string("Notch", &mut login_start).unwrap();
        login_start.extend([0u8; 16]);
        protocol::write_frame(&mut client, &login_start)
            .await
            .unwrap();

        let compression = protocol::read_frame(&mut client, 4096).await.unwrap();
        assert_eq!(protocol::decode_varint(&compression).unwrap(), (3, 1));
        assert_eq!(protocol::decode_varint(&compression[1..]).unwrap(), (1, 1));

        let success = protocol::read_compressed_frame(&mut client, 4096, 1)
            .await
            .unwrap();
        let (success_id, mut offset) = protocol::decode_varint(&success).unwrap();
        assert_eq!(success_id, 2);
        let profile_id: [u8; 16] = success[offset..offset + 16].try_into().unwrap();
        offset += 16;
        assert_eq!(
            protocol::take_string(&success, &mut offset, 64).unwrap(),
            "Notch"
        );
        assert_eq!(protocol::take_varint(&success, &mut offset).unwrap(), 0);
        assert_eq!(profile_id, protocol::offline_uuid("Notch"));
        assert_eq!(
            success.len() - offset,
            16,
            "Login Success must end with session UUID"
        );

        let ack = protocol::encode_varint(3);
        protocol::write_compressed_frame(&mut client, &ack, 1)
            .await
            .unwrap();
        assert_configuration_packets(&mut client, None, None, 1).await;
        task.await.unwrap();
    }

    #[tokio::test]
    async fn online_login_verifies_mojang_profile_and_encrypts_connection() {
        use rsa::{pkcs1v15::Pkcs1v15Encrypt, pkcs8::DecodePublicKey, RsaPublicKey};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mock_auth = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let auth_address = mock_auth.local_addr().unwrap();
        let auth_task = tokio::spawn(async move {
            let (mut socket, _) = mock_auth.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            let body =
                r#"{"id":"00000000000000000000000000000001","name":"Notch","properties":[]}"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let key_path =
            std::env::temp_dir().join(format!("sryzamc-test-{}.pk8", uuid::Uuid::new_v4()));
        let identity = ServerIdentity::load_or_create_at(
            &key_path,
            format!("http://{auth_address}/hasJoined"),
        )
        .unwrap();
        let mut config = config::Config::default();
        config.network.compression_threshold = 1;
        assert!(config.server.online_mode);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_config = config.clone();
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            handle(socket, &server_config, Some(&identity), None)
                .await
                .unwrap();
        });
        let mut client = TcpStream::connect(address).await.unwrap();
        let mut handshake = protocol::encode_varint(0);
        handshake.extend(protocol::encode_varint(PROTOCOL_VERSION));
        protocol::put_string("localhost", &mut handshake).unwrap();
        handshake.extend(25565u16.to_be_bytes());
        handshake.extend(protocol::encode_varint(LOGIN_STATE));
        protocol::write_frame(&mut client, &handshake)
            .await
            .unwrap();
        let mut login_start = protocol::encode_varint(0);
        protocol::put_string("Notch", &mut login_start).unwrap();
        login_start.extend([0u8; 16]);
        protocol::write_frame(&mut client, &login_start)
            .await
            .unwrap();

        let encryption_request = protocol::read_frame(&mut client, 4096).await.unwrap();
        let mut offset = 0;
        assert_eq!(
            protocol::take_varint(&encryption_request, &mut offset).unwrap(),
            1
        );
        assert_eq!(
            protocol::take_string(&encryption_request, &mut offset, 20).unwrap(),
            ""
        );
        let public_key_der = take_test_bytes(&encryption_request, &mut offset);
        let verify_token = take_test_bytes(&encryption_request, &mut offset);
        assert_eq!(encryption_request[offset], 1);
        let public_key = RsaPublicKey::from_public_key_der(&public_key_der).unwrap();
        let secret = [0x33; 16];
        let mut key_response = protocol::encode_varint(1);
        put_test_bytes(
            &public_key
                .encrypt(&mut rand::rngs::OsRng, Pkcs1v15Encrypt, &secret)
                .unwrap(),
            &mut key_response,
        );
        put_test_bytes(
            &public_key
                .encrypt(&mut rand::rngs::OsRng, Pkcs1v15Encrypt, &verify_token)
                .unwrap(),
            &mut key_response,
        );
        protocol::write_frame(&mut client, &key_response)
            .await
            .unwrap();

        let mut incoming_cipher = protocol::MinecraftCipher::new(&secret).unwrap();
        let mut outgoing_cipher = protocol::MinecraftCipher::new(&secret).unwrap();
        let set_compression = protocol::read_wire_frame(&mut client, 4096, &mut incoming_cipher)
            .await
            .unwrap();
        let (packet_id, mut offset) = protocol::decode_varint(&set_compression).unwrap();
        assert_eq!(packet_id, 3);
        assert_eq!(
            protocol::take_varint(&set_compression, &mut offset).unwrap(),
            1
        );

        let login_success =
            protocol::read_compressed_wire_frame(&mut client, 4096, 1, &mut incoming_cipher)
                .await
                .unwrap();
        let (packet_id, mut offset) = protocol::decode_varint(&login_success).unwrap();
        assert_eq!(packet_id, 2);
        let profile_uuid: [u8; 16] = login_success[offset..offset + 16].try_into().unwrap();
        offset += 16;
        assert_eq!(
            protocol::take_string(&login_success, &mut offset, 64).unwrap(),
            "Notch"
        );
        assert_eq!(
            profile_uuid,
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001")
                .unwrap()
                .into_bytes()
        );
        let ack = protocol::encode_varint(3);
        protocol::write_compressed_wire_frame(&mut client, &ack, 1, &mut outgoing_cipher)
            .await
            .unwrap();
        assert_configuration_packets(
            &mut client,
            Some(&mut incoming_cipher),
            Some(&mut outgoing_cipher),
            1,
        )
        .await;
        task.await.unwrap();
        auth_task.await.unwrap();
        std::fs::remove_file(key_path).unwrap();
    }

    fn take_test_bytes(data: &[u8], offset: &mut usize) -> Vec<u8> {
        let len = protocol::take_varint(data, offset).unwrap() as usize;
        let bytes = data[*offset..*offset + len].to_vec();
        *offset += len;
        bytes
    }

    fn put_test_bytes(bytes: &[u8], out: &mut Vec<u8>) {
        out.extend(protocol::encode_varint(bytes.len() as i32));
        out.extend_from_slice(bytes);
    }

    async fn assert_configuration_packets(
        client: &mut TcpStream,
        incoming_cipher: Option<&mut protocol::MinecraftCipher>,
        outgoing_cipher: Option<&mut protocol::MinecraftCipher>,
        compression: i32,
    ) {
        let mut incoming_cipher = incoming_cipher;
        let mut outgoing_cipher = outgoing_cipher;
        let selected = match incoming_cipher.as_deref_mut() {
            Some(cipher) => protocol::read_compressed_wire_frame(client, 4096, compression, cipher)
                .await
                .unwrap(),
            None => protocol::read_compressed_frame(client, 4096, compression)
                .await
                .unwrap(),
        };
        let mut offset = 0;
        assert_eq!(protocol::take_varint(&selected, &mut offset).unwrap(), 15);
        assert_eq!(protocol::take_varint(&selected, &mut offset).unwrap(), 0);
        let known_packs = vec![7, 0];
        match outgoing_cipher.as_deref_mut() {
            Some(cipher) => {
                protocol::write_compressed_wire_frame(client, &known_packs, compression, cipher)
                    .await
                    .unwrap()
            }
            None => protocol::write_compressed_frame(client, &known_packs, compression)
                .await
                .unwrap(),
        }

        let keep_alive = match incoming_cipher.as_deref_mut() {
            Some(cipher) => protocol::read_compressed_wire_frame(client, 4096, compression, cipher)
                .await
                .unwrap(),
            None => protocol::read_compressed_frame(client, 4096, compression)
                .await
                .unwrap(),
        };
        let (keep_alive_id, mut offset) = protocol::decode_varint(&keep_alive).unwrap();
        assert_eq!(keep_alive_id, 4);
        let token_bytes: [u8; 8] = keep_alive[offset..offset + 8].try_into().unwrap();
        offset += 8;
        assert_eq!(offset, keep_alive.len());
        let mut keep_alive_response = protocol::encode_varint(4);
        keep_alive_response.extend(token_bytes);
        match outgoing_cipher.as_deref_mut() {
            Some(cipher) => protocol::write_compressed_wire_frame(
                client,
                &keep_alive_response,
                compression,
                cipher,
            )
            .await
            .unwrap(),
            None => protocol::write_compressed_frame(client, &keep_alive_response, compression)
                .await
                .unwrap(),
        }

        let features = match incoming_cipher.as_deref_mut() {
            Some(cipher) => protocol::read_compressed_wire_frame(client, 4096, compression, cipher)
                .await
                .unwrap(),
            None => protocol::read_compressed_frame(client, 4096, compression)
                .await
                .unwrap(),
        };
        let mut offset = 0;
        assert_eq!(protocol::take_varint(&features, &mut offset).unwrap(), 13);
        assert_eq!(protocol::take_varint(&features, &mut offset).unwrap(), 1);
        assert_eq!(
            protocol::take_string(&features, &mut offset, 64).unwrap(),
            "minecraft:vanilla"
        );

        let tags = match incoming_cipher.as_deref_mut() {
            Some(cipher) => protocol::read_compressed_wire_frame(client, 4096, compression, cipher)
                .await
                .unwrap(),
            None => protocol::read_compressed_frame(client, 4096, compression)
                .await
                .unwrap(),
        };
        assert_eq!(tags, [14, 0]);
        let finish = match incoming_cipher {
            Some(cipher) => protocol::read_compressed_wire_frame(client, 4096, compression, cipher)
                .await
                .unwrap(),
            None => protocol::read_compressed_frame(client, 4096, compression)
                .await
                .unwrap(),
        };
        assert_eq!(finish, [3]);
        match outgoing_cipher {
            Some(cipher) => {
                protocol::write_compressed_wire_frame(client, &[3], compression, cipher)
                    .await
                    .unwrap()
            }
            None => protocol::write_compressed_frame(client, &[3], compression)
                .await
                .unwrap(),
        }
    }
}
