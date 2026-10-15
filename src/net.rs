//! Captured frames to TCP segments, and segments back into ordered byte streams.

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use crate::frames::FrameDecoder;

const MAX_HELD_SEGMENTS: usize = 128;
const MAX_HELD_BYTES: usize = 2 * 1024 * 1024;
const IDLE: Duration = Duration::from_secs(120);
const SWEEP_EVERY: Duration = Duration::from_secs(10);

/// One direction of a TCP connection. IPv4 addresses sit in the last four bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StreamKey {
    pub src: [u8; 16],
    pub dst: [u8; 16],
    pub sport: u16,
    pub dport: u16,
}

pub struct Segment<'a> {
    pub key: StreamKey,
    pub seq: u32,
    pub syn: bool,
    pub fin: bool,
    pub rst: bool,
    pub payload: &'a [u8],
}

/// The TCP segment in a captured frame, for the link types Npcap hands out:
/// Ethernet, its loopback adapter, and raw IP from VPN adapters
pub fn segment(link: i32, frame: &[u8]) -> Option<Segment<'_>> {
    let ip = match link {
        1 => {
            let mut o = 12;
            let mut ethertype = be16(frame, o)?;
            while ethertype == 0x8100 || ethertype == 0x88A8 {
                o += 4;
                ethertype = be16(frame, o)?;
            }
            match ethertype {
                0x0800 | 0x86DD => frame.get(o + 2..)?,
                _ => return None,
            }
        }
        0 => frame.get(4..)?,
        12 | 14 | 101 | 228 | 229 => frame,
        _ => return None,
    };
    match ip.first()? >> 4 {
        4 => ipv4(ip),
        6 => ipv6(ip),
        _ => None,
    }
}

fn ipv4(ip: &[u8]) -> Option<Segment<'_>> {
    let header = ((ip.first()? & 0x0F) as usize) * 4;
    if header < 20 || ip.len() < header || ip[9] != 6 {
        return None;
    }
    // fragments: more-fragments flag or an offset
    if be16(ip, 6)? & 0x3FFF != 0 {
        return None;
    }
    // segmentation offload leaves the length at 0 on outgoing packets
    let total = be16(ip, 2)? as usize;
    let end = if total == 0 { ip.len() } else { total.min(ip.len()) };
    let mut src = [0u8; 16];
    let mut dst = [0u8; 16];
    src[12..].copy_from_slice(&ip[12..16]);
    dst[12..].copy_from_slice(&ip[16..20]);
    tcp(ip.get(header..end)?, src, dst)
}

fn ipv6(ip: &[u8]) -> Option<Segment<'_>> {
    if ip.len() < 40 || ip[6] != 6 {
        return None;
    }
    let length = be16(ip, 4)? as usize;
    let end = if length == 0 { ip.len() } else { (40 + length).min(ip.len()) };
    tcp(ip.get(40..end)?, ip[8..24].try_into().ok()?, ip[24..40].try_into().ok()?)
}

fn tcp(t: &[u8], src: [u8; 16], dst: [u8; 16]) -> Option<Segment<'_>> {
    let offset = ((*t.get(12)? >> 4) as usize) * 4;
    if offset < 20 || offset > t.len() {
        return None;
    }
    let flags = t[13];
    Some(Segment {
        key: StreamKey { src, dst, sport: be16(t, 0)?, dport: be16(t, 2)? },
        seq: u32::from_be_bytes(t.get(4..8)?.try_into().ok()?),
        syn: flags & 0x02 != 0,
        fin: flags & 0x01 != 0,
        rst: flags & 0x04 != 0,
        payload: &t[offset..],
    })
}

fn be16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

/// Every TCP stream on the machine, each with its own game-frame decoder
pub struct Streams {
    map: HashMap<StreamKey, Stream>,
    swept: Instant,
}

impl Streams {
    pub fn new() -> Self {
        Self { map: HashMap::new(), swept: Instant::now() }
    }

    pub fn push(&mut self, seg: Segment, now: Instant, sink: &mut dyn FnMut(&StreamKey, u16, &[u8])) {
        if now.duration_since(self.swept) > SWEEP_EVERY {
            self.map.retain(|_, s| now.duration_since(s.seen) < IDLE);
            self.swept = now;
        }
        let key = seg.key;
        if seg.rst {
            self.map.remove(&key);
            return;
        }
        let stream = self.map.entry(key).or_insert_with(|| Stream::new(now));
        if seg.syn {
            *stream = Stream::new(now);
            stream.next = Some(seg.seq.wrapping_add(1));
        }
        stream.seen = now;
        if !stream.decoder.is_dead() && !seg.payload.is_empty() {
            stream.accept(seg.seq, seg.payload, &mut |op, body| sink(&key, op, body));
        }
        if seg.fin {
            self.map.remove(&key);
        }
    }
}

struct Stream {
    next: Option<u32>,
    held: Vec<(u32, Vec<u8>)>,
    held_bytes: usize,
    decoder: FrameDecoder,
    seen: Instant,
}

impl Stream {
    fn new(now: Instant) -> Self {
        Self { next: None, held: Vec::new(), held_bytes: 0, decoder: FrameDecoder::new(), seen: now }
    }

    fn accept(&mut self, seq: u32, data: &[u8], sink: &mut dyn FnMut(u16, &[u8])) {
        let next = *self.next.get_or_insert(seq);
        let ahead = seq.wrapping_sub(next);
        if ahead == 0 {
            self.deliver(data, sink);
        } else if ahead > u32::MAX / 2 {
            // a retransmit, maybe with new bytes on the end
            let behind = next.wrapping_sub(seq) as usize;
            if behind < data.len() {
                self.deliver(&data[behind..], sink);
            }
        } else {
            self.held.push((seq, data.to_vec()));
            self.held_bytes += data.len();
            if self.held.len() > MAX_HELD_SEGMENTS || self.held_bytes > MAX_HELD_BYTES {
                // the missing bytes aren't coming: jump the gap and resync on a heartbeat
                let earliest = self.held.iter().map(|(s, _)| *s).min_by_key(|s| s.wrapping_sub(next)).unwrap();
                self.next = Some(earliest);
                self.decoder.reset();
                self.drain(sink);
            }
        }
    }

    fn deliver(&mut self, data: &[u8], sink: &mut dyn FnMut(u16, &[u8])) {
        self.decoder.feed(data, sink);
        self.next = self.next.map(|n| n.wrapping_add(data.len() as u32));
        self.drain(sink);
    }

    /// Feed held segments that now line up
    fn drain(&mut self, sink: &mut dyn FnMut(u16, &[u8])) {
        while let Some(next) = self.next {
            let Some(i) = self.held.iter().position(|(s, _)| next.wrapping_sub(*s) <= u32::MAX / 2) else { break };
            let (seq, data) = self.held.swap_remove(i);
            self.held_bytes -= data.len();
            let skip = next.wrapping_sub(seq) as usize;
            if skip < data.len() {
                self.decoder.feed(&data[skip..], sink);
                self.next = Some(next.wrapping_add((data.len() - skip) as u32));
            }
        }
    }
}

impl fmt::Display for StreamKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let addr = |a: &[u8; 16]| {
            if a[..12].iter().all(|&b| b == 0) {
                std::net::Ipv4Addr::new(a[12], a[13], a[14], a[15]).to_string()
            } else {
                format!("[{}]", std::net::Ipv6Addr::from(*a))
            }
        };
        write!(f, "{}:{} -> {}:{}", addr(&self.src), self.sport, addr(&self.dst), self.dport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::tests::{frame, handshake};

    fn ethernet(seq: u32, flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; 12];
        f.extend([0x08, 0x00]);
        let total = (20 + 20 + payload.len()) as u16;
        f.extend([0x45, 0, (total >> 8) as u8, total as u8, 0, 0, 0x40, 0, 64, 6, 0, 0]);
        f.extend([203, 0, 113, 7, 192, 168, 1, 20]);
        f.extend(7777u16.to_be_bytes());
        f.extend(50123u16.to_be_bytes());
        f.extend(seq.to_be_bytes());
        f.extend([0, 0, 0, 0, 0x50, flags, 0xFF, 0xFF, 0, 0, 0, 0]);
        f.extend(payload);
        f.extend([0; 6]); // ethernet padding past the IP length
        f
    }

    #[test]
    fn reads_an_ethernet_frame() {
        let f = ethernet(1000, 0x18, b"hello");
        let seg = segment(1, &f).unwrap();
        assert_eq!(seg.payload, b"hello");
        assert_eq!(seg.seq, 1000);
        assert_eq!(seg.key.to_string(), "203.0.113.7:7777 -> 192.168.1.20:50123");
    }

    #[test]
    fn reorders_and_drops_retransmits() {
        let mut stream = handshake();
        stream.extend(frame(0x0238, &[1; 300]));
        stream.extend(frame(0x4738, &[2; 10]));
        let chunks: Vec<(u32, &[u8])> = stream.chunks(100).enumerate().map(|(i, c)| (5000 + i as u32 * 100, c)).collect();
        // 0, 2, 1 (out of order), 1 again (retransmit), then the rest
        let order = [0usize, 2, 1, 1, 3];
        let mut streams = Streams::new();
        let mut got = Vec::new();
        let now = Instant::now();
        for &i in &order {
            let (seq, data) = chunks[i];
            let f = ethernet(seq, 0x18, data);
            streams.push(segment(1, &f).unwrap(), now, &mut |_, op, body| got.push((op, body.len())));
        }
        assert_eq!(got, [(0x0036, 8), (0x0238, 300), (0x4738, 10)]);
    }
}
