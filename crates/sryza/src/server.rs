use crate::{config, protocol::{self, take_string, take_varint, PROTOCOL_VERSION, GAME_VERSION}, scheduler::TickScheduler};
use anyhow::{ensure, Context, Result};
use serde_json::json;
use std::{path::Path, sync::Arc};
use tokio::{net::{TcpListener, TcpStream}, sync::Semaphore};
use tracing::{info, warn};

const STATUS_STATE: i32 = 1;
const LOGIN_STATE: i32 = 2;

pub async fn run(root: &Path) -> Result<()> {
    let cfg = config::load(root)?;
    let listener = TcpListener::bind(cfg.network.listen).await?;
    let slots = Arc::new(Semaphore::new(cfg.resources.max_packet_queue as usize));
    info!(address = %cfg.network.listen, version = GAME_VERSION, "SryzaMC accepting Minecraft status and ping connections");
    tokio::spawn(TickScheduler::new(cfg.performance.target_tps).run_clock());
    loop {
        let (stream, peer) = listener.accept().await?;
        let permit = match slots.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => { warn!(%peer, "connection queue full"); continue; }
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
    let handshake = protocol::read_frame(&mut stream, max).await.context("read handshake frame")?;
    let mut cursor = 0;
    ensure!(take_varint(&handshake, &mut cursor)? == 0, "expected handshake packet");
    let client_protocol = take_varint(&handshake, &mut cursor)?;
    let _server_address = take_string(&handshake, &mut cursor, 255)?;
    let port_end = cursor.checked_add(2).context("handshake port offset overflow")?;
    let port = u16::from_be_bytes(handshake.get(cursor..port_end).context("truncated handshake port")?.try_into()?);
    cursor = port_end;
    let next_state = take_varint(&handshake, &mut cursor)?;
    ensure!(cursor == handshake.len(), "unexpected trailing handshake bytes");
    ensure!(port != 0, "client sent an empty server port");

    match next_state {
        STATUS_STATE => status_session(&mut stream, config, max).await,
        LOGIN_STATE => {
            warn!(client_protocol, "login requested but login is not implemented");
            Ok(())
        }
        other => anyhow::bail!("unsupported handshake intent {other}"),
    }
}

async fn status_session(stream: &mut TcpStream, config: &config::Config, max: usize) -> Result<()> {
    let request = protocol::read_frame(stream, max).await.context("read status request")?;
    let mut cursor = 0;
    ensure!(take_varint(&request, &mut cursor)? == 0 && cursor == request.len(), "invalid status request");

    let response = json!({
        "version": { "name": GAME_VERSION, "protocol": PROTOCOL_VERSION },
        "players": { "max": config.server.max_players, "online": 0, "sample": [] },
        "description": { "text": config.server.motd }
    });
    let json = serde_json::to_vec(&response)?;
    let mut packet = protocol::encode_varint(0);
    protocol::put_string(std::str::from_utf8(&json)?, &mut packet)?;
    protocol::write_frame(stream, &packet).await.context("write status response")?;

    let ping = protocol::read_frame(stream, max).await.context("read status ping")?;
    ensure!(ping.len() == 9, "invalid status ping length");
    let (packet_id, used) = protocol::decode_varint(&ping)?;
    ensure!(packet_id == 1 && used == 1, "expected status ping packet");
    let mut pong = protocol::encode_varint(1);
    pong.extend_from_slice(&ping[used..]);
    protocol::write_frame(stream, &pong).await.context("write status pong")
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
        protocol::write_frame(&mut client, &handshake).await.unwrap();
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
}
