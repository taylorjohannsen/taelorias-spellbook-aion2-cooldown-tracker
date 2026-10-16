//! The few server packets the overlay reads. Opcodes are the two bytes in wire order.

use crate::frames::MAX_BUNDLE;
use crate::wire::{varint, Reader};

pub const SELF_INFO: u16 = 0x3336;
pub const CAST: u16 = 0x0238;
pub const COOLDOWNS: u16 = 0x4738;
pub const MAP_LOAD: u16 = 0x2136;
pub const BATTLE: u16 = 0x218D;
/// A buff landing, then BUFF_REFRESH as it's renewed. The first carries one more byte.
pub const BUFF: u16 = 0x2A38;
pub const BUFF_REFRESH: u16 = 0x2B38;
/// A buff ending, sent either way. The second carries one more byte.
pub const BUFF_END: u16 = 0x1A92;
pub const BUFF_GONE: u16 = 0x0E92;

const MAX_COOLDOWN_MS: u32 = 3_600_000;
/// Anything longer is a buff that lasts until it's taken off
const MAX_BUFF_MS: u32 = 86_400_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Our own character, sent on every loading screen
    SelfInfo { entity: u32, name: String, server: u16 },
    /// Anyone nearby used a skill. The cooldown is the server's, reductions included.
    Cast { actor: u32, target: u32, skill: u32, cooldown_ms: u32 },
    /// Running cooldowns of our own skills moved: (skill, ms left)
    Cooldowns(Vec<(u32, u32)>),
    /// A map loaded: a loading screen, or a teleport within the same map
    Map(u32),
    /// Someone went in or out of combat, us included, as the server sees it
    Battle { entity: u32, on: bool },
    /// A buff or debuff landed on someone. 0 ms is one that lasts until it's removed.
    Buff { target: u32, buff: u32, duration_ms: u32 },
    BuffEnd { target: u32, buff: u32 },
}

pub fn parse(opcode: u16, body: &[u8]) -> Option<Event> {
    match opcode {
        SELF_INFO => self_info(body),
        CAST => cast(body),
        COOLDOWNS => cooldowns(body),
        MAP_LOAD => map_load(body),
        BATTLE => battle(body),
        BUFF => buff(body, true),
        BUFF_REFRESH => buff(body, false),
        BUFF_END => buff_end(body, false),
        BUFF_GONE => buff_end(body, true),
        _ => None,
    }
}

/// Instanced dungeons live in the 6xxxxx map range
pub fn is_dungeon(map: u32) -> bool {
    (600_000..700_000).contains(&map)
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

/// The class a skill belongs to, from the first two digits of its id (11 to 19),
/// or 0 for anything else
pub fn class_of(skill: u32) -> u32 {
    if (11_000_000..20_000_000).contains(&skill) {
        skill / 1_000_000
    } else {
        0
    }
}

/// What a buff is tracked by: the skill behind it, so a change of
/// specialization keeps tracking it, or the buff itself when no skill is
pub fn buff_key(buff: u32) -> u32 {
    if buff >= 100_000_000 {
        base_skill(buff / 10)
    } else {
        buff
    }
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

/// loads u32, map u32, ...
fn map_load(b: &[u8]) -> Option<Event> {
    let mut r = Reader::new(b);
    r.u32()?;
    let map = r.u32().filter(|&m| (1..10_000_000).contains(&m))?;
    Some(Event::Map(map))
}

/// entity varint, varint, 1 in combat / 0 out, [deadline u64 when a timed boss is pulled]
fn battle(b: &[u8]) -> Option<Event> {
    let mut r = Reader::new(b);
    let entity = r.varint().filter(|&id| is_entity(id))?;
    r.varint()?;
    let on = match r.varint()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    matches!(r.remaining(), 0 | 8).then_some(Event::Battle { entity, on })
}

/// target varint, [u8, always 1, if `flagged`], varint, instance varint, buff u32,
/// duration ms u32, u32, started u64, caster varint, ...
fn buff(b: &[u8], flagged: bool) -> Option<Event> {
    let mut r = Reader::new(b);
    let target = r.varint().filter(|&id| is_entity(id))?;
    if flagged {
        r.u8()?;
    }
    r.varint()?;
    r.varint()?;
    let buff = r.u32().filter(|&id| id != 0)?;
    let duration = r.u32()?;
    r.skip(12)?;
    let duration_ms = if duration > MAX_BUFF_MS { 0 } else { duration };
    Some(Event::Buff { target, buff, duration_ms })
}

/// target varint, [u8, always 1, if `flagged`], buff u32
fn buff_end(b: &[u8], flagged: bool) -> Option<Event> {
    let mut r = Reader::new(b);
    let target = r.varint().filter(|&id| is_entity(id))?;
    if flagged {
        r.u8()?;
    }
    let buff = r.u32().filter(|&id| id != 0)?;
    (r.remaining() == 0).then_some(Event::BuffEnd { target, buff })
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

    // Our own toggles from a Global capture of 2026-10-15; the timed boss (Ferocious
    // Horn Nuakum) and the plain one are from cyberbadger6969/aion2-dps-meter
    #[test]
    fn combat_toggles() {
        assert_eq!(parse(BATTLE, &hex("c8330001")), Some(Event::Battle { entity: 6600, on: true }));
        assert_eq!(parse(BATTLE, &hex("c8330000")), Some(Event::Battle { entity: 6600, on: false }));
        assert_eq!(parse(BATTLE, &hex("f19702 01 01 ea66962da1010000")), Some(Event::Battle { entity: 35825, on: true }));
        assert_eq!(parse(BATTLE, &hex("fb3e0001")), Some(Event::Battle { entity: 8059, on: true }));
        assert_eq!(parse(BATTLE, &hex("c8330002")), None);
        assert_eq!(parse(BATTLE, &hex("c833000100")), None);
    }

    // Our own Wish of Concentration landing, a Chanter's aura being renewed on us, and
    // Winter's Shackles and Element Enhancement ending, Global 2026-10-15
    #[test]
    fn buffs() {
        let wish = hex("bf37 01 13 d907 a5282009 50460000 00000000 66e28441a1010000 bf37 01aa9de90000 360f0747 4d2a0ec7 12ddc246");
        assert_eq!(parse(BUFF, &wish), Some(Event::Buff { target: 7103, buff: 153_102_501, duration_ms: 18_000 }));
        let aura = hex("9b1a 13 38 d3ffd20a 60090000 00000000 11b98d41a1010000 fb52 0a95191501 02 3c61af46 142725c5 6aac27c6");
        assert_eq!(parse(BUFF_REFRESH, &aura), Some(Event::Buff { target: 3355, buff: 181_600_211, duration_ms: 2_400 }));
        let forever = hex("9b1a 11 8a0a 91e2f505 ffffffff ffffffff 8075d52abb030000 9b1a 0100");
        assert_eq!(parse(BUFF_REFRESH, &forever), Some(Event::Buff { target: 3355, buff: 100_000_401, duration_ms: 0 }));
        assert_eq!(parse(BUFF_END, &hex("bf37 8d9b0109")), Some(Event::BuffEnd { target: 7103, buff: 151_100_301 }));
        assert_eq!(parse(BUFF_END, &hex("bf37 8d9b010900")), None);
        assert_eq!(parse(BUFF_GONE, &hex("bf37 01 11dc2d09")), Some(Event::BuffEnd { target: 7103, buff: 154_000_401 }));
        assert_eq!(buff_key(154_000_401), 15_400_000);
        assert_eq!(buff_key(153_102_501), 15_310_000);
        assert_eq!(buff_key(3_015), 3_015);
    }

    #[test]
    fn map_loads() {
        let mut body = 1u32.to_le_bytes().to_vec();
        body.extend(600_092u32.to_le_bytes());
        assert_eq!(parse(MAP_LOAD, &body), Some(Event::Map(600_092)));
        assert!(is_dungeon(600_092) && !is_dungeon(1110));
        assert_eq!(parse(MAP_LOAD, &body[..6]), None);
    }

    #[test]
    fn truncated_casts_are_dropped() {
        let body = hex("c74300 0c28e900 9202 d1e802 b0f930c38dbae8470b71d3c700faa546ea5d01dc6f");
        for len in 0..body.len() - 1 {
            assert_eq!(parse(CAST, &body[..len]), None, "length {len}");
        }
    }
}
