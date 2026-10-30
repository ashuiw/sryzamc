//! Minecraft Configuration-state packet sequencing for protocol 777.
//!
//! The server currently uses the client's vanilla registries and therefore sends no
//! Registry Data packets. The protocol permits this: omitted synchronized registries
//! retain their client-side values. Pack and tag formats live here rather than in the
//! connection state machine so they can grow independently.

use crate::protocol::{encode_varint, put_string, take_string, take_varint};
use anyhow::{ensure, Context, Result};

pub const CLIENTBOUND_FINISH_CONFIGURATION: i32 = 3;
pub const CLIENTBOUND_KEEP_ALIVE: i32 = 4;
pub const CLIENTBOUND_SELECT_KNOWN_PACKS: i32 = 15;
pub const CLIENTBOUND_UPDATE_ENABLED_FEATURES: i32 = 13;
pub const CLIENTBOUND_UPDATE_TAGS: i32 = 14;
pub const SERVERBOUND_FINISH_CONFIGURATION: i32 = 3;
pub const SERVERBOUND_KEEP_ALIVE: i32 = 4;
pub const SERVERBOUND_SELECT_KNOWN_PACKS: i32 = 7;

/// Encode Select Known Packs with the server's installed pack list.
/// SryzaMC currently has no data packs, so an empty list is correct.
pub fn select_known_packs(packs: &[(&str, &str, &str)]) -> Result<Vec<u8>> {
    let mut packet = encode_varint(CLIENTBOUND_SELECT_KNOWN_PACKS);
    packet.extend(encode_varint(i32::try_from(packs.len())?));
    for (namespace, id, version) in packs {
        put_string(namespace, &mut packet)?;
        put_string(id, &mut packet)?;
        put_string(version, &mut packet)?;
    }
    Ok(packet)
}

/// Validate the client's response to Select Known Packs.
pub fn validate_known_packs_response(packet: &[u8]) -> Result<Vec<(String, String, String)>> {
    let mut offset = 0;
    ensure!(
        take_varint(packet, &mut offset)? == SERVERBOUND_SELECT_KNOWN_PACKS,
        "expected Select Known Packs response"
    );
    let count = take_varint(packet, &mut offset)?;
    ensure!((0..=256).contains(&count), "invalid known-pack count");
    let mut packs = Vec::with_capacity(count as usize);
    for _ in 0..count {
        packs.push((
            take_string(packet, &mut offset, 64)?,
            take_string(packet, &mut offset, 256)?,
            take_string(packet, &mut offset, 64)?,
        ));
    }
    ensure!(
        offset == packet.len(),
        "trailing data in known-pack response"
    );
    Ok(packs)
}

pub fn update_enabled_features(features: &[&str]) -> Result<Vec<u8>> {
    let mut packet = encode_varint(CLIENTBOUND_UPDATE_ENABLED_FEATURES);
    packet.extend(encode_varint(i32::try_from(features.len())?));
    for feature in features {
        put_string(feature, &mut packet)?;
    }
    Ok(packet)
}

/// Send an empty tag update. This does not replace registries absent from the map.
pub fn update_tags_empty() -> Vec<u8> {
    let mut packet = encode_varint(CLIENTBOUND_UPDATE_TAGS);
    packet.push(0); // registry map length
    packet
}

pub fn finish_configuration() -> Vec<u8> {
    encode_varint(CLIENTBOUND_FINISH_CONFIGURATION)
}

pub fn keep_alive(keep_alive_id: i64) -> Vec<u8> {
    let mut packet = encode_varint(CLIENTBOUND_KEEP_ALIVE);
    packet.extend(keep_alive_id.to_be_bytes());
    packet
}

pub fn validate_keep_alive(packet: &[u8], expected_id: i64) -> Result<()> {
    let mut offset = 0;
    ensure!(
        take_varint(packet, &mut offset)? == SERVERBOUND_KEEP_ALIVE,
        "expected Configuration Keep Alive response"
    );
    let end = offset
        .checked_add(8)
        .context("keep-alive length overflow")?;
    let bytes: [u8; 8] = packet
        .get(offset..end)
        .context("truncated Configuration Keep Alive response")?
        .try_into()?;
    ensure!(end == packet.len(), "trailing data in keep-alive response");
    ensure!(
        i64::from_be_bytes(bytes) == expected_id,
        "Configuration Keep Alive response token mismatch"
    );
    Ok(())
}

pub fn validate_finish_acknowledgement(packet: &[u8]) -> Result<()> {
    let (packet_id, used) = crate::protocol::decode_varint(packet)?;
    ensure!(
        packet_id == SERVERBOUND_FINISH_CONFIGURATION && used == packet.len(),
        "expected empty Finish Configuration acknowledgement"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_pack_negotiation_round_trips_and_rejects_trailing_bytes() {
        let packet = select_known_packs(&[]).unwrap();
        assert_eq!(crate::protocol::decode_varint(&packet).unwrap(), (15, 1));
        assert_eq!(validate_known_packs_response(&[7, 0]).unwrap(), Vec::new());
        assert!(validate_known_packs_response(&[7, 0, 0]).is_err());
    }

    #[test]
    fn emits_vanilla_features_empty_tags_and_finish() {
        let features = update_enabled_features(&["minecraft:vanilla"]).unwrap();
        assert_eq!(crate::protocol::decode_varint(&features).unwrap(), (13, 1));
        assert_eq!(update_tags_empty(), vec![14, 0]);
        assert_eq!(finish_configuration(), vec![3]);
        assert_eq!(keep_alive(0x0102030405060708), [4, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(validate_keep_alive(&[4, 1, 2, 3, 4, 5, 6, 7, 8], 0x0102030405060708).is_ok());
        assert!(validate_keep_alive(&[4, 0, 0, 0, 0, 0, 0, 0, 0], 1).is_err());
        assert!(validate_finish_acknowledgement(&[3]).is_ok());
        assert!(validate_finish_acknowledgement(&[3, 0]).is_err());
    }
}
