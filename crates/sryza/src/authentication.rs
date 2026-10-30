use crate::protocol::{GameProfile, ProfileProperty};
use anyhow::{bail, ensure, Context, Result};
use rand::rngs::OsRng;
use rsa::{
    pkcs1v15::Pkcs1v15Encrypt,
    pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey},
    RsaPrivateKey, RsaPublicKey,
};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::net::IpAddr;

const SESSION_URL: &str = "https://sessionserver.mojang.com/session/minecraft/hasJoined";

pub struct ServerIdentity {
    private_key: RsaPrivateKey,
    public_key_der: Vec<u8>,
    http: reqwest::Client,
    session_url: String,
}

impl ServerIdentity {
    /// Load the persistent Minecraft RSA identity or create it on first startup.
    pub fn load_or_create(path: &std::path::Path) -> Result<Self> {
        Self::load_or_create_at(path, SESSION_URL)
    }

    pub fn load_or_create_at(
        path: &std::path::Path,
        session_url: impl Into<String>,
    ) -> Result<Self> {
        let private_key = if path.exists() {
            RsaPrivateKey::from_pkcs8_der(&std::fs::read(path).context("read server RSA key")?)
                .context("decode server RSA key")?
        } else {
            let key = RsaPrivateKey::new(&mut OsRng, 1024).context("generate Minecraft RSA key")?;
            let der = key.to_pkcs8_der().context("encode server RSA key")?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, der.as_bytes()).context("persist server RSA key")?;
            key
        };
        let public_key_der = RsaPublicKey::from(&private_key)
            .to_public_key_der()
            .context("encode server public key")?
            .as_bytes()
            .to_vec();
        Ok(Self {
            private_key,
            public_key_der,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()?,
            session_url: session_url.into(),
        })
    }

    pub fn encryption_request(&self, verify_token: &[u8]) -> Result<Vec<u8>> {
        let mut packet = crate::protocol::encode_varint(1);
        crate::protocol::put_string("", &mut packet)?;
        put_byte_array(&self.public_key_der, &mut packet)?;
        put_byte_array(verify_token, &mut packet)?;
        packet.push(1); // shouldAuthenticate: vanilla client performs session join
        Ok(packet)
    }

    pub fn decrypt_encryption_response(
        &self,
        packet: &[u8],
        expected_token: &[u8],
    ) -> Result<[u8; 16]> {
        let mut offset = 0;
        ensure!(
            crate::protocol::take_varint(packet, &mut offset)? == 1,
            "expected Encryption Response"
        );
        let encrypted_secret = take_byte_array(packet, &mut offset, 512)?;
        let encrypted_token = take_byte_array(packet, &mut offset, 512)?;
        ensure!(
            offset == packet.len(),
            "unexpected trailing Encryption Response bytes"
        );
        let secret = self
            .private_key
            .decrypt(Pkcs1v15Encrypt, &encrypted_secret)
            .context("decrypt shared secret")?;
        let token = self
            .private_key
            .decrypt(Pkcs1v15Encrypt, &encrypted_token)
            .context("decrypt verify token")?;
        use subtle::ConstantTimeEq as _;
        ensure!(
            token.len() == expected_token.len()
                && bool::from(token.as_slice().ct_eq(expected_token)),
            "invalid encryption verify token"
        );
        ensure!(secret.len() == 16, "invalid AES shared secret length");
        Ok(secret.try_into().expect("validated 16-byte secret"))
    }

    pub async fn verify(
        &self,
        username: &str,
        shared_secret: &[u8],
        ip: Option<IpAddr>,
    ) -> Result<GameProfile> {
        let hash = minecraft_server_hash(b"", shared_secret, &self.public_key_der);
        verify_joined_profile_at(&self.http, &self.session_url, username, &hash, ip).await
    }
}

fn put_byte_array(bytes: &[u8], out: &mut Vec<u8>) -> Result<()> {
    out.extend(crate::protocol::encode_varint(i32::try_from(bytes.len())?));
    out.extend_from_slice(bytes);
    Ok(())
}

fn take_byte_array(data: &[u8], offset: &mut usize, max: usize) -> Result<Vec<u8>> {
    let length = crate::protocol::take_varint(data, offset)?;
    ensure!(
        length >= 0 && length as usize <= max,
        "invalid byte array length"
    );
    let end = offset
        .checked_add(length as usize)
        .context("byte array length overflow")?;
    let bytes = data
        .get(*offset..end)
        .context("truncated byte array")?
        .to_vec();
    *offset = end;
    Ok(bytes)
}

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

async fn verify_joined_profile_at(
    client: &reqwest::Client,
    session_url: &str,
    username: &str,
    server_hash: &str,
    client_ip: Option<IpAddr>,
) -> Result<GameProfile> {
    ensure!(
        !username.is_empty() && username.chars().count() <= 16,
        "invalid player name"
    );
    let mut request = client
        .get(session_url)
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
    let uuid_text = if joined.id.len() == 32 && joined.id.is_ascii() {
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
