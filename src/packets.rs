//! The few server packets the overlay reads. Opcodes are the two bytes in wire order.

use crate::frames::MAX_BUNDLE;
use crate::wire::{varint, Reader};

pub const SELF_INFO: u16 = 0x3336;
pub const CAST: u16 = 0x0238;
pub const COOLDOWNS: u16 = 0x4738;

const MAX_COOLDOWN_MS: u32 = 3_600_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Our own character, sent on every loading screen
    SelfInfo { entity: u32, name: String, server: u16 },
    /// Anyone nearby used a skill. The cooldown is the server's, reductions included.
    Cast { actor: u32, target: u32, skill: u32, cooldown_ms: u32 },
    /// Running cooldowns of our own skills moved: (skill, ms left)
    Cooldowns(Vec<(u32, u32)>),
}

pub fn parse(opcode: u16, body: &[u8]) -> Option<Event> {
    match opcode {
        SELF_INFO => self_info(body),
        CAST => cast(body),
        COOLDOWNS => cooldowns(body),
        _ => None,
    }
}

/// Our character record also rides inside other packets, sometimes in an LZ4 bundle
/// partway through one, and the game repeats it every few minutes. That finds us
/// when the overlay starts mid-session, without waiting for a loading screen.
pub fn embedded_self(body: &[u8]) -> Option<Event> {
    if body.len() < 16 {
        return None;
    }
    find_self(body).or_else(|| bundled_self(body))
}

fn find_self(b: &[u8]) -> Option<Event> {
    (0..b.len().saturating_sub(10)).find_map(|i| {
        if u16::from_be_bytes([b[i], b[i + 1]]) != SELF_INFO {
            return None;
        }
        // stricter than the packet itself, since this could be any bytes
        self_info(&b[i + 2..]).filter(|e| matches!(e, Event::SelfInfo { name, server, .. } if !name.is_empty() && *server != 0))
    })
}

/// `varint len, FF FF, u32 raw size, lz4 block` somewhere inside a packet
fn bundled_self(b: &[u8]) -> Option<Event> {
    for i in 1..b.len().saturating_sub(8) {
        if b[i] != 0xFF || b[i + 1] != 0xFF {
            continue;
        }
        for n in (1..=3).rev() {
            let Some(at) = i.checked_sub(n) else { continue };
            let Some((len, vlen)) = varint(b, at) else { continue };
            if vlen != n || len < 12 || at + len as usize + vlen - 4 > b.len() {
                continue;
            }
            let end = at + len as usize + vlen - 4;
            let raw = u32::from_le_bytes(b[i + 2..i + 6].try_into().unwrap()) as usize;
            if raw == 0 || raw > MAX_BUNDLE || i + 6 > end {
                continue;
            }
            let mut out = vec![0u8; raw];
            if let Ok(size) = lz4_flex::block::decompress_into(&b[i + 6..end], &mut out) {
                if let Some(event) = find_self(&out[..size]) {
                    return Some(event);
                }
            }
        }
    }
    None
}

/// The skill without its specialization digits (the last four), which is the id
/// NC's site and the skill bar use
pub fn base_skill(skill: u32) -> u32 {
    skill / 10_000 * 10_000
}

fn is_entity(id: u32) -> bool {
    (1..=9_999_999).contains(&id)
}

fn is_skill(id: u32) -> bool {
    (1..300_000_000).contains(&id)
}

/// entity varint, 4 bytes, flags u8 (bit 0 set), name length u8, name, server u16, ...
fn self_info(b: &[u8]) -> Option<Event> {
    let mut r = Reader::new(b);
    let entity = r.varint().filter(|&id| is_entity(id))?;
    r.skip(4)?;
    if r.u8()? & 0x01 == 0 {
        return None;
    }
    let len = r.u8()? as usize;
    if !(1..=36).contains(&len) {
        return None;
    }
    let raw = r.take(len)?;
    // some loading screens send a '$' placeholder instead of the name
    let name = if raw[0] == b'$' {
        String::new()
    } else {
        let name = std::str::from_utf8(raw).ok()?;
        if name.chars().any(char::is_control) {
            return None;
        }
        name.to_string()
    };
    let server = r.u16().filter(|s| (1000..3000).contains(s)).unwrap_or(0);
    Some(Event::SelfInfo { entity, name, server })
}

/// ```text
/// actor varint, flags u8, skill u32, u8, [u8 if flags & 1], target, varint, u8,
/// [varint if flags & 2], cooldown ms varint, ...
/// target = flags u8, entity varint, [u32 if flags & 1], yaw f32, [x y z f32 if flags & 2]
/// ```
fn cast(b: &[u8]) -> Option<Event> {
    let mut r = Reader::new(b);
    let actor = r.varint().filter(|&id| is_entity(id))?;
    let flags = r.u8()?;
    let skill = r.u32().filter(|&s| is_skill(s))?;
    r.u8()?;
    if flags & 0x01 != 0 {
        r.u8()?;
    }
    let target_flags = r.u8()?;
    let target = r.varint()?;
    if target_flags & 0x01 != 0 {
        r.u32()?;
    }
    r.skip(4)?;
    if target_flags & 0x02 != 0 {
        r.skip(12)?;
    }
    r.varint()?;
    r.u8()?;
    if flags & 0x02 != 0 {
        r.varint()?;
    }
    let cooldown_ms = r.varint().filter(|&ms| ms <= MAX_COOLDOWN_MS)?;
    Some(Event::Cast { actor, target, skill, cooldown_ms })
}

/// count u8, count × (skill u32, ms left varint)
fn cooldowns(b: &[u8]) -> Option<Event> {
    let mut r = Reader::new(b);
    let count = r.u8()?;
    if !(1..=64).contains(&count) {
        return None;
    }
    let mut list = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let skill = r.u32().filter(|&s| is_skill(s))?;
        let left = r.varint().filter(|&ms| ms <= MAX_COOLDOWN_MS)?;
        list.push((skill, left));
    }
    Some(Event::Cooldowns(list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::hex;

    // Bodies from an EU capture of 2026-10-05, as published with the GPL meter
    // cyberbadger6969/aion2-dps-meter (tests/ProtocolTests.cs)

    #[test]
    fn sorcerer_cast_on_a_boss() {
        let body = hex("c74300 0c28e900 9202 d1e802 b0f930c38dbae8470b71d3c700faa546ea5d01dc6f");
        assert_eq!(
            parse(CAST, &body),
            Some(Event::Cast { actor: 8647, target: 46161, skill: 15_280_140, cooldown_ms: 14_300 })
        );
        assert_eq!(base_skill(15_280_140), 15_280_000);
    }

    #[test]
    fn effect_entity_cast() {
        let body = hex("d1ca0200 0d28e900 0102 d1ca02 a263c741d3c7ea47bd1ed1c70048a546ee64029a3a");
        assert_eq!(
            parse(CAST, &body),
            Some(Event::Cast { actor: 42321, target: 42321, skill: 15_280_141, cooldown_ms: 7_450 })
        );
    }

    #[test]
    fn own_character() {
        let body = hex("ed745e91c12837064e696d61726118051e000000011c0000007f0100007f0100001c000000d002040000000000");
        assert_eq!(
            parse(SELF_INFO, &body),
            Some(Event::SelfInfo { entity: 14957, name: "Nimara".into(), server: 1304 })
        );
    }

    #[test]
    fn finds_our_record_inside_other_packets() {
        let record = hex("3336 ed745e91c12837064e696d61726118051e000000011c0000007f0100007f0100001c000000d002040000000000");
        let me = Some(Event::SelfInfo { entity: 14957, name: "Nimara".into(), server: 1304 });

        let mut plain = vec![0x11; 9];
        plain.extend(&record);
        assert_eq!(embedded_self(&plain), me);

        let mut inner = crate::frames::tests::frame(0x1d37, &[0x22; 30]);
        inner.extend(&record);
        let mut bundled = vec![0x11; 5];
        bundled.extend(crate::frames::tests::bundle(&inner));
        bundled.extend([0x11; 4]);
        assert_eq!(embedded_self(&bundled), me);

        assert_eq!(embedded_self(&[0x33, 0x36].repeat(20)), None);
    }

    #[test]
    fn cooldown_list() {
        let mut body = vec![2];
        body.extend(18_010_000u32.to_le_bytes());
        body.extend([0xDC, 0x6F]);
        body.extend(18_040_000u32.to_le_bytes());
        body.push(0);
        assert_eq!(parse(COOLDOWNS, &body), Some(Event::Cooldowns(vec![(18_010_000, 14_300), (18_040_000, 0)])));
        assert_eq!(parse(COOLDOWNS, &body[..body.len() - 1]), None);
    }

    #[test]
    fn truncated_casts_are_dropped() {
        let body = hex("c74300 0c28e900 9202 d1e802 b0f930c38dbae8470b71d3c700faa546ea5d01dc6f");
        for len in 0..body.len() - 1 {
            assert_eq!(parse(CAST, &body[..len]), None, "length {len}");
        }
    }
}
