use anyhow::{bail, ensure, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_VERSION: i32 = 777;
pub const GAME_VERSION: &str = "26.3";

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
    ensure!(length >= 0 && length as usize <= max_bytes, "invalid string length");
    let end = offset.checked_add(length as usize).ok_or_else(|| anyhow::anyhow!("string length overflow"))?;
    let bytes = data.get(*offset..end).ok_or_else(|| anyhow::anyhow!("truncated string"))?;
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
        if byte & 0x80 == 0 { break; }
    }
    let (length, used) = decode_varint(&prefix)?;
    ensure!(used == prefix.len() && length >= 0 && length as usize <= max, "invalid or oversized frame");
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
}
