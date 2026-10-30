use crate::{
    config,
    protocol::{self, take_string, take_varint, GAME_VERSION, PROTOCOL_VERSION},
    scheduler::TickScheduler,
};
use anyhow::{ensure, Context, Result};
use serde_json::json;
use std::{path::Path, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};
use tracing::{info, warn};

const STATUS_STATE: i32 = 1;
const LOGIN_STATE: i32 = 2;

pub async fn run(root: &Path) -> Result<()> {
    let cfg = config::load(root)?;
    let listener = TcpListener::bind(cfg.network.listen).await?;
    let slots = Arc::new(Semaphore::new(cfg.resources.max_packet_queue as usize));
    info!(address = %cfg.network.listen, version = GAME_VERSION, "SryzaMC accepting status, ping, and guarded login connections");
    tokio::spawn(TickScheduler::new(cfg.performance.target_tps).run_clock());
    loop {
        let (stream, peer) = listener.accept().await?;
        let permit = match slots.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                warn!(%peer, "connection queue full");
                continue;
            }
        };
        let config = cfg.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(error) = handle(stream, &config).await {
                warn!(%peer, %error, "connection closed");
            }
        });
    }
}

async fn handle(mut stream: TcpStream, config: &config::Config) -> Result<()> {
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
        LOGIN_STATE => login_session(&mut stream, config, max, client_protocol).await,
        other => anyhow::bail!("unsupported handshake intent {other}"),
    }
}

async fn login_session(
    stream: &mut TcpStream,
    config: &config::Config,
    max: usize,
    client_protocol: i32,
) -> Result<()> {
    ensure!(
        client_protocol == PROTOCOL_VERSION,
        "login protocol mismatch: expected {PROTOCOL_VERSION}, got {client_protocol}"
    );
    let start_packet = protocol::read_frame(stream, max)
        .await
        .context("read Login Start")?;
    let login = protocol::LoginStart::decode(&start_packet)?;

    if config.server.online_mode {
        send_disconnect(
            stream,
            0,
            "Online authentication is not implemented yet; no login was accepted.",
        )
        .await?;
        warn!(player = %login.username, "rejected login because online authentication is unavailable");
        return Ok(());
    }

    let profile = protocol::GameProfile {
        uuid: protocol::offline_uuid(&login.username),
        username: login.username.clone(),
        properties: Vec::new(),
    };
    let session_id = *uuid::Uuid::new_v4().as_bytes();
    let threshold = config.network.compression_threshold;
    let compression_enabled = threshold >= 0;

    if compression_enabled {
        let mut set_compression = protocol::encode_varint(3);
        set_compression.extend(protocol::encode_varint(threshold));
        protocol::write_frame(stream, &set_compression)
            .await
            .context("send Set Compression")?;
    }

    let login_success = protocol::encode_login_success(&profile, &session_id)?;
    if compression_enabled {
        protocol::write_compressed_frame(stream, &login_success, threshold)
            .await
            .context("send Login Success")?;
    } else {
        protocol::write_frame(stream, &login_success)
            .await
            .context("send Login Success")?;
    }

    let acknowledgement = if compression_enabled {
        protocol::read_compressed_frame(stream, max, threshold)
            .await
            .context("read Login Acknowledged")?
    } else {
        protocol::read_frame(stream, max)
            .await
            .context("read Login Acknowledged")?
    };
    let mut offset = 0;
    ensure!(
        protocol::take_varint(&acknowledgement, &mut offset)? == 3
            && offset == acknowledgement.len(),
        "expected Login Acknowledged"
    );

    // Do not leave a client hanging or claim Play support before registry synchronization exists.
    let mut config_disconnect = protocol::encode_varint(2);
    protocol::put_string(
        &json!({ "text": "Login completed in offline mode, but Configuration registry synchronization is not implemented yet." }).to_string(),
        &mut config_disconnect,
    )?;
    if compression_enabled {
        protocol::write_compressed_frame(stream, &config_disconnect, threshold)
            .await
            .context("send Configuration Disconnect")?;
    } else {
        protocol::write_frame(stream, &config_disconnect)
            .await
            .context("send Configuration Disconnect")?;
    }
    info!(player = %profile.username, "completed offline Login state; configuration is not implemented");
    Ok(())
}

async fn send_disconnect(stream: &mut TcpStream, packet_id: i32, reason: &str) -> Result<()> {
    let mut packet = protocol::encode_varint(packet_id);
    protocol::put_string(&json!({ "text": reason }).to_string(), &mut packet)?;
    protocol::write_frame(stream, &packet).await
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
            let (mut socket, _) = listener.accept().await.unwrap();
            handle(socket, &server_config).await.unwrap();
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
    async fn offline_login_uses_compression_then_disconnects_at_unimplemented_configuration() {
        let mut config = config::Config::default();
        config.server.online_mode = false;
        config.network.compression_threshold = 1;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_config = config.clone();
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            handle(socket, &server_config).await.unwrap();
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
        let disconnect = protocol::read_compressed_frame(&mut client, 4096, 1)
            .await
            .unwrap();
        let (disconnect_id, mut offset) = protocol::decode_varint(&disconnect).unwrap();
        assert_eq!(disconnect_id, 2);
        let reason = protocol::take_string(&disconnect, &mut offset, 2048).unwrap();
        assert!(reason.contains("Configuration registry synchronization is not implemented"));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn online_mode_rejects_login_instead_of_accepting_an_unauthenticated_profile() {
        let config = config::Config::default();
        assert!(config.server.online_mode);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_config = config.clone();
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            handle(socket, &server_config).await.unwrap();
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

        let disconnect = protocol::read_frame(&mut client, 4096).await.unwrap();
        let (id, mut offset) = protocol::decode_varint(&disconnect).unwrap();
        assert_eq!(id, 0, "expected login disconnect, never Login Success");
        let reason = protocol::take_string(&disconnect, &mut offset, 2048).unwrap();
        assert!(reason.contains("Online authentication is not implemented"));
        task.await.unwrap();
    }
}
