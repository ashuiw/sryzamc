use crate::protocol::{GameProfile, ProfileProperty};
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::net::IpAddr;

const SESSION_URL: &str = "https://sessionserver.mojang.com/session/minecraft/hasJoined";

#[derive(Debug, Deserialize)]
struct SessionProperty {
    name: String,
    value: String,
    #[serde(default)]
    signature: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JoinedResponse {
    id: String,
    name: String,
    #[serde(default)]
    properties: Vec<SessionProperty>,
}

/// Minecraft's signed hexadecimal SHA-1 digest used by the session server.
pub fn minecraft_server_hash(server_id: &[u8], secret: &[u8], public_key_der: &[u8]) -> String {
    let mut hash = Sha1::new();
    hash.update(server_id);
    hash.update(secret);
    hash.update(public_key_der);
    signed_hex(&hash.finalize())
}

fn signed_hex(digest: &[u8]) -> String {
    if digest.is_empty() {
        return "0".into();
    }
    let negative = digest[0] & 0x80 != 0;
    let mut magnitude = digest.to_vec();
    if negative {
        for byte in &mut magnitude {
            *byte = !*byte;
        }
        for byte in magnitude.iter_mut().rev() {
            let (value, carry) = byte.overflowing_add(1);
            *byte = value;
            if !carry {
                break;
            }
        }
    }
    let first = magnitude
        .iter()
        .position(|b| *b != 0)
        .unwrap_or(magnitude.len());
    let mut digits = String::new();
    for byte in &magnitude[first..] {
        use std::fmt::Write as _;
        if digits.is_empty() {
            write!(&mut digits, "{byte:x}").expect("writing to String cannot fail");
        } else {
            write!(&mut digits, "{byte:02x}").expect("writing to String cannot fail");
        }
    }
    if digits.is_empty() {
        digits.push('0');
    }
    if negative {
        format!("-{digits}")
    } else {
        digits
    }
}

/// Verify a client that has completed the encrypted session join against Mojang.
/// This function must only be called after decrypting and validating the encryption response.
pub async fn verify_joined_profile(
    client: &reqwest::Client,
    username: &str,
    server_hash: &str,
    client_ip: Option<IpAddr>,
) -> Result<GameProfile> {
    ensure!(
        !username.is_empty() && username.chars().count() <= 16,
        "invalid player name"
    );
    let mut request = client
        .get(SESSION_URL)
        .query(&[("username", username), ("serverId", server_hash)]);
    if let Some(ip) = client_ip {
        request = request.query(&[("ip", ip.to_string())]);
    }
    let response = request
        .send()
        .await
        .context("request Mojang session verification")?;
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        bail!("Mojang session verification rejected the player");
    }
    let response = response
        .error_for_status()
        .context("Mojang session verification failed")?;
    let joined: JoinedResponse = response
        .json()
        .await
        .context("parse Mojang session profile")?;
    ensure!(
        joined.name.eq_ignore_ascii_case(username),
        "Mojang returned a different player name"
    );
    ensure!(
        joined.properties.len() <= 16,
        "Mojang returned too many profile properties"
    );
    let uuid_text = if joined.id.len() == 32 {
        format!(
            "{}-{}-{}-{}-{}",
            &joined.id[0..8],
            &joined.id[8..12],
            &joined.id[12..16],
            &joined.id[16..20],
            &joined.id[20..32]
        )
    } else {
        joined.id
    };
    let uuid =
        uuid::Uuid::parse_str(&uuid_text).context("invalid UUID from Mojang session service")?;
    Ok(GameProfile {
        uuid: *uuid.as_bytes(),
        username: joined.name,
        properties: joined
            .properties
            .into_iter()
            .map(|property| ProfileProperty {
                name: property.name,
                value: property.value,
                signature: property.signature,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_hash_uses_twos_complement_hex() {
        assert_eq!(signed_hex(&[0, 0, 0xab]), "ab");
        assert_eq!(signed_hex(&[0xff, 0xff, 0x80]), "-80");
        assert_eq!(signed_hex(&[0, 0, 0]), "0");
    }

    #[test]
    fn session_hash_covers_all_protocol_inputs() {
        assert_ne!(
            minecraft_server_hash(b"", &[1, 2], &[3]),
            minecraft_server_hash(b"", &[1, 2], &[4])
        );
    }
}
