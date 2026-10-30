use anyhow::{bail, ensure, Result};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use md5::{Digest, Md5};
use std::io::{Read, Write};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_VERSION: i32 = 777;
pub const GAME_VERSION: &str = "26.3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginStart {
    pub username: String,
    pub client_uuid: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameProfile {
    pub uuid: [u8; 16],
    pub username: String,
    pub properties: Vec<ProfileProperty>,
}

impl LoginStart {
    pub fn decode(packet: &[u8]) -> Result<Self> {
        let mut offset = 0;
        ensure!(
            take_varint(packet, &mut offset)? == 0,
            "expected Login Start packet"
        );
        let username = take_string(packet, &mut offset, 64)?;
        ensure!(
            !username.is_empty() && username.chars().count() <= 16,
            "invalid player name"
        );
        let end = offset
            .checked_add(16)
            .ok_or_else(|| anyhow::anyhow!("UUID length overflow"))?;
        let client_uuid = packet
            .get(offset..end)
            .ok_or_else(|| anyhow::anyhow!("truncated Login Start UUID"))?
            .try_into()?;
        ensure!(end == packet.len(), "unexpected trailing Login Start bytes");
        Ok(Self {
            username,
            client_uuid,
        })
    }
}

/// Mojang's offline-mode UUID: name-based MD5 with UUID v3 and RFC 4122 variant bits.
pub fn offline_uuid(username: &str) -> [u8; 16] {
    let mut digest = Md5::digest(format!("OfflinePlayer:{username}").as_bytes()).to_vec();
    digest[6] = (digest[6] & 0x0f) | 0x30;
    digest[8] = (digest[8] & 0x3f) | 0x80;
    digest.try_into().expect("MD5 digest is 16 bytes")
}

pub fn encode_login_success(profile: &GameProfile, session_id: &[u8; 16]) -> Result<Vec<u8>> {
    ensure!(
        profile.properties.len() <= 16,
        "too many profile properties"
    );
    let mut packet = encode_varint(2);
    packet.extend_from_slice(&profile.uuid);
    put_string(&profile.username, &mut packet)?;
    packet.extend(encode_varint(i32::try_from(profile.properties.len())?));
    for property in &profile.properties {
        put_string(&property.name, &mut packet)?;
        put_string(&property.value, &mut packet)?;
        match &property.signature {
            Some(signature) => {
                packet.push(1);
                put_string(signature, &mut packet)?;
            }
            None => packet.push(0),
        }
    }
    packet.extend_from_slice(session_id);
    Ok(packet)
}

pub fn decode_varint(bytes: &[u8]) -> Result<(i32, usize)> {
    let mut result = 0i32;
    for (i, byte) in bytes.iter().copied().take(5).enumerate() {
        if i == 4 && byte & 0xf0 != 0 {
            bail!("VarInt exceeds 32 bits");
        }
        result |= i32::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok((result, i + 1));
        }
    }
    bail!("incomplete or overlong VarInt")
}

pub fn encode_varint(mut value: i32) -> Vec<u8> {
    let mut out = Vec::with_capacity(5);
    loop {
        if value & !0x7f == 0 {
            out.push(value as u8);
            return out;
        }
        out.push(((value & 0x7f) | 0x80) as u8);
        value = ((value as u32) >> 7) as i32;
    }
}

pub fn take_varint(data: &[u8], offset: &mut usize) -> Result<i32> {
    let (value, used) = decode_varint(data.get(*offset..).unwrap_or_default())?;
    *offset += used;
    Ok(value)
}

pub fn take_string(data: &[u8], offset: &mut usize, max_bytes: usize) -> Result<String> {
    let length = take_varint(data, offset)?;
    ensure!(
        length >= 0 && length as usize <= max_bytes,
        "invalid string length"
    );
    let end = offset
        .checked_add(length as usize)
        .ok_or_else(|| anyhow::anyhow!("string length overflow"))?;
    let bytes = data
        .get(*offset..end)
        .ok_or_else(|| anyhow::anyhow!("truncated string"))?;
    *offset = end;
    Ok(std::str::from_utf8(bytes)?.to_owned())
}

pub fn put_string(value: &str, out: &mut Vec<u8>) -> Result<()> {
    let length = i32::try_from(value.len())?;
    out.extend(encode_varint(length));
    out.extend(value.as_bytes());
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(stream: &mut R, max: usize) -> Result<Vec<u8>> {
    let mut prefix = Vec::with_capacity(5);
    for _ in 0..5 {
        let byte = stream.read_u8().await?;
        prefix.push(byte);
        if byte & 0x80 == 0 {
            break;
        }
    }
    let (length, used) = decode_varint(&prefix)?;
    ensure!(
        used == prefix.len() && length >= 0 && length as usize <= max,
        "invalid or oversized frame"
    );
    let mut frame = vec![0; length as usize];
    stream.read_exact(&mut frame).await?;
    Ok(frame)
}

pub async fn write_frame<W: AsyncWrite + Unpin>(stream: &mut W, payload: &[u8]) -> Result<()> {
    let length = i32::try_from(payload.len())?;
    stream.write_all(&encode_varint(length)).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

/// Read one Minecraft frame after Set Compression has taken effect.
/// Both the outer frame and advertised decompressed length are bounded by `max`.
pub async fn read_compressed_frame<R: AsyncRead + Unpin>(
    stream: &mut R,
    max: usize,
    threshold: i32,
) -> Result<Vec<u8>> {
    let frame = read_frame(stream, max).await?;
    let mut offset = 0;
    let data_length = take_varint(&frame, &mut offset)?;
    ensure!(
        data_length >= 0 && data_length as usize <= max,
        "invalid decompressed frame length"
    );
    let body = &frame[offset..];
    if data_length == 0 {
        ensure!(
            threshold < 0 || body.len() < threshold as usize,
            "uncompressed packet exceeds compression threshold"
        );
        return Ok(body.to_vec());
    }
    ensure!(
        threshold < 0 || data_length >= threshold,
        "compressed packet is below compression threshold"
    );
    let mut decoder = ZlibDecoder::new(body);
    let mut decoded = Vec::with_capacity(data_length as usize);
    decoder
        .take(data_length as u64 + 1)
        .read_to_end(&mut decoded)?;
    ensure!(
        decoded.len() == data_length as usize,
        "compressed packet length mismatch"
    );
    Ok(decoded)
}

/// Write one packet using the negotiated Minecraft compression framing.
pub async fn write_compressed_frame<W: AsyncWrite + Unpin>(
    stream: &mut W,
    packet: &[u8],
    threshold: i32,
) -> Result<()> {
    let mut framed = Vec::new();
    if threshold >= 0 && packet.len() >= threshold as usize {
        framed.extend(encode_varint(i32::try_from(packet.len())?));
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(packet)?;
        framed.extend(encoder.finish()?);
    } else {
        framed.push(0);
        framed.extend_from_slice(packet);
    }
    write_frame(stream, &framed).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip() {
        for value in [0, 1, 127, 128, 300, i32::MAX, -1] {
            let encoded = encode_varint(value);
            assert_eq!(decode_varint(&encoded).unwrap(), (value, encoded.len()));
        }
    }

    #[test]
    fn rejects_incomplete_and_oversized_varints() {
        assert!(decode_varint(&[0x80; 5]).is_err());
        assert!(decode_varint(&[0xff, 0xff, 0xff, 0xff, 0x10]).is_err());
    }

    #[test]
    fn strings_are_length_prefixed_and_bounded() {
        let mut bytes = Vec::new();
        put_string("hello", &mut bytes).unwrap();
        let mut offset = 0;
        assert_eq!(take_string(&bytes, &mut offset, 16).unwrap(), "hello");
        let mut bounded_offset = 0;
        assert!(take_string(&bytes, &mut bounded_offset, 2).is_err());
    }

    #[test]
    fn parses_login_start_and_derives_vanilla_offline_uuid() {
        let mut packet = encode_varint(0);
        put_string("Notch", &mut packet).unwrap();
        let client_uuid = [0x12; 16];
        packet.extend(client_uuid);
        assert_eq!(
            LoginStart::decode(&packet).unwrap(),
            LoginStart {
                username: "Notch".into(),
                client_uuid
            }
        );
        assert_eq!(
            offline_uuid("Notch"),
            [
                0xb5, 0x0a, 0xd3, 0x85, 0x82, 0x9d, 0x31, 0x41, 0xa2, 0x16, 0x7e, 0x7d, 0x75, 0x39,
                0xba, 0x7f,
            ]
        );
        packet.push(0);
        assert!(LoginStart::decode(&packet).is_err());
    }

    #[test]
    fn login_success_encodes_profile_and_session_uuid() {
        let profile = GameProfile {
            uuid: [0x11; 16],
            username: "TestPlayer".into(),
            properties: vec![ProfileProperty {
                name: "textures".into(),
                value: "base64-data".into(),
                signature: Some("signature".into()),
            }],
        };
        let session = [0x22; 16];
        let packet = encode_login_success(&profile, &session).unwrap();
        assert_eq!(decode_varint(&packet).unwrap(), (2, 1));
        assert!(packet.ends_with(&session));
    }

    #[tokio::test]
    async fn compression_framing_round_trips_both_paths() {
        let (mut left, mut right) = tokio::io::duplex(4096);
        let packet = vec![42; 300];
        let expected = packet.clone();
        let writer = tokio::spawn(async move {
            write_compressed_frame(&mut left, &packet, 64)
                .await
                .unwrap();
        });
        assert_eq!(
            read_compressed_frame(&mut right, 2048, 64).await.unwrap(),
            expected
        );
        writer.await.unwrap();

        let (mut left, mut right) = tokio::io::duplex(4096);
        let packet = vec![7; 12];
        let expected = packet.clone();
        let writer = tokio::spawn(async move {
            write_compressed_frame(&mut left, &packet, 64)
                .await
                .unwrap();
        });
        assert_eq!(
            read_compressed_frame(&mut right, 2048, 64).await.unwrap(),
            expected
        );
        writer.await.unwrap();
    }
}
