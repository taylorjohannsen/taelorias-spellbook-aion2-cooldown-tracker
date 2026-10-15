//! Splits one direction of a TCP stream into game packets.
//!
//! ```text
//! frame   = varint len, [0xF0..=0xFE], payload        (frame size = len + varint bytes - 4)
//! payload = opcode(2) body | FF FF, u32 raw size, lz4 block holding more frames
//! ```
//!
//! 0x00 bytes between frames are padding. The decoder lines up on the heartbeat frame
//! (`0E 00 36`, ~19 a second) and goes back to hunting for one whenever a length stops
//! making sense. Nothing comes out until a few heartbeats have decoded in a row: a
//! download can contain those three bytes by chance, but not a run of real frames.

use crate::wire::varint;

const MAX_FRAME: usize = 65_535;
const MAX_UNBUNDLED_WAIT: usize = 16_384;
pub const MAX_BUNDLE: usize = 8 * 1024 * 1024;
/// A stream that hasn't proven itself in this many bytes isn't the game
const MAX_UNCONFIRMED: usize = 512 * 1024;
const CONFIRM_HEARTBEATS: u32 = 3;
const HEARTBEAT: u16 = 0x0036;
const MAX_DEPTH: u32 = 4;
const HEARTBEATS: [[u8; 3]; 2] = [[0x0E, 0x00, 0x36], [0x06, 0x00, 0x36]];

pub struct FrameDecoder {
    buf: Vec<u8>,
    hunting: bool,
    scanned: usize,
    heartbeats: u32,
    dead: bool,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self { buf: Vec::new(), hunting: true, scanned: 0, heartbeats: 0, dead: false }
    }

    /// Gave up on this stream: no heartbeats, so it's some other program's traffic
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Bytes were lost, so the next byte isn't a frame boundary
    pub fn reset(&mut self) {
        self.buf.clear();
        self.hunting = true;
    }

    pub fn feed(&mut self, data: &[u8], sink: &mut dyn FnMut(u16, &[u8])) {
        if self.dead {
            return;
        }
        self.buf.extend_from_slice(data);
        let mut pos = 0;
        while pos < self.buf.len() {
            let span = &self.buf[pos..];

            if self.hunting {
                match find_heartbeat(span) {
                    Some(at) => {
                        pos += at;
                        self.hunting = false;
                        continue;
                    }
                    None => {
                        // a heartbeat may straddle two segments
                        pos += span.len().saturating_sub(2);
                        break;
                    }
                }
            }

            if span[0] == 0x00 {
                pos += 1;
                continue;
            }

            let Some((len, vlen)) = varint(span, 0) else {
                if span.len() < 5 {
                    break; // the rest of the varint hasn't arrived
                }
                pos += self.resync();
                continue;
            };
            if len < 6 || len as usize + vlen - 4 > MAX_FRAME {
                pos += self.resync();
                continue;
            }
            let size = len as usize + vlen - 4;
            if size > span.len() {
                if size > MAX_UNBUNDLED_WAIT && !is_bundle(span, vlen) {
                    pos += self.resync();
                    continue;
                }
                break; // wait for the rest of the frame
            }

            let heartbeats = &mut self.heartbeats;
            handle_frame(&span[..size], vlen, 0, &mut |opcode, body| {
                if opcode == HEARTBEAT {
                    *heartbeats += 1;
                }
                if *heartbeats >= CONFIRM_HEARTBEATS {
                    sink(opcode, body);
                }
            });
            pos += size;
        }
        if self.heartbeats < CONFIRM_HEARTBEATS {
            self.scanned += pos;
            if self.scanned > MAX_UNCONFIRMED {
                self.dead = true;
                self.buf = Vec::new();
                return;
            }
        }
        self.buf.drain(..pos);
    }

    fn resync(&mut self) -> usize {
        self.hunting = true;
        // an unproven stream has to show its heartbeats without a break
        if self.heartbeats < CONFIRM_HEARTBEATS {
            self.heartbeats = 0;
        }
        1
    }
}

fn find_heartbeat(span: &[u8]) -> Option<usize> {
    span.windows(3).position(|w| HEARTBEATS.iter().any(|h| w == h))
}

fn payload_start(frame: &[u8], vlen: usize) -> usize {
    match frame.get(vlen) {
        Some(0xF0..=0xFE) => vlen + 1,
        _ => vlen,
    }
}

fn is_bundle(frame: &[u8], vlen: usize) -> bool {
    let o = payload_start(frame, vlen);
    frame.get(o..o + 2) == Some(&[0xFF, 0xFF])
}

fn handle_frame(frame: &[u8], vlen: usize, depth: u32, sink: &mut dyn FnMut(u16, &[u8])) {
    let mut o = payload_start(frame, vlen);
    if o + 2 > frame.len() {
        return;
    }
    if frame[o] == 0xFF && frame[o + 1] == 0xFF {
        o += 2;
        let Some(raw) = frame.get(o..o + 4) else { return };
        let raw = u32::from_le_bytes(raw.try_into().unwrap()) as usize;
        o += 4;
        if raw == 0 || raw > MAX_BUNDLE || depth >= MAX_DEPTH {
            return;
        }
        let mut out = vec![0u8; raw];
        if let Ok(n) = lz4_flex::block::decompress_into(&frame[o..], &mut out) {
            walk_bundle(&out[..n], depth + 1, sink);
        }
        return;
    }
    let opcode = u16::from_be_bytes([frame[o], frame[o + 1]]);
    sink(opcode, &frame[o + 2..]);
}

/// Frames inside a decompressed bundle. No hunting here: a bad length ends the bundle.
fn walk_bundle(data: &[u8], depth: u32, sink: &mut dyn FnMut(u16, &[u8])) {
    let mut pos = 0;
    while pos < data.len() {
        if data[pos] == 0x00 {
            pos += 1;
            continue;
        }
        let span = &data[pos..];
        let Some((len, vlen)) = varint(span, 0) else { return };
        if len < 6 || len as usize + vlen - 4 > span.len() {
            return;
        }
        let size = len as usize + vlen - 4;
        handle_frame(&span[..size], vlen, depth, sink);
        pos += size;
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::wire::encode_varint;

    pub fn frame(opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        encode_varint(body.len() as u32 + 6, &mut out);
        out.extend_from_slice(&opcode.to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    pub fn bundle(frames: &[u8]) -> Vec<u8> {
        let packed = lz4_flex::block::compress(frames);
        let mut out = Vec::new();
        encode_varint(packed.len() as u32 + 10, &mut out);
        out.extend_from_slice(&[0xFF, 0xFF]);
        out.extend_from_slice(&(frames.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
        out
    }

    pub fn heartbeat() -> Vec<u8> {
        frame(0x0036, &[1, 2, 3, 4, 5, 6, 7, 8])
    }

    /// Enough heartbeats for a decoder to trust the stream
    pub fn handshake() -> Vec<u8> {
        (0..CONFIRM_HEARTBEATS).flat_map(|_| heartbeat()).collect()
    }

    fn decode(chunks: &[&[u8]]) -> Vec<(u16, Vec<u8>)> {
        let mut decoder = FrameDecoder::new();
        let mut got = Vec::new();
        for chunk in chunks {
            decoder.feed(chunk, &mut |op, body| {
                if op != HEARTBEAT {
                    got.push((op, body.to_vec()))
                }
            });
        }
        got
    }

    #[test]
    fn heartbeat_matches_the_wire() {
        assert_eq!(&heartbeat()[..3], &[0x0E, 0x00, 0x36]);
    }

    #[test]
    fn splits_frames_across_segments() {
        let mut stream = vec![0x13, 0x37, 0x99]; // joined mid-stream
        stream.extend(handshake());
        stream.extend(frame(0x0238, &[9; 40]));
        stream.push(0x00);
        stream.extend(frame(0x4738, &[1, 2, 3]));
        for cut in 1..stream.len() {
            let got = decode(&[&stream[..cut], &stream[cut..]]);
            let ops: Vec<u16> = got.iter().map(|(op, _)| *op).collect();
            assert_eq!(ops, [0x0238, 0x4738], "cut at {cut}");
            assert_eq!(got[0].1, vec![9; 40]);
        }
    }

    #[test]
    fn unpacks_bundles_and_nested_bundles() {
        let mut inner = frame(0x0238, &[7; 30]);
        inner.extend(frame(0x4738, &[5; 12]));
        let nested = bundle(&frame(0x3336, &[3; 20]));
        inner.extend(&nested);
        let mut stream = handshake();
        stream.extend(bundle(&inner));
        let ops: Vec<u16> = decode(&[&stream]).iter().map(|(op, _)| *op).collect();
        assert_eq!(ops, [0x0238, 0x4738, 0x3336]);
    }

    #[test]
    fn flag_byte_before_the_payload_is_skipped() {
        let mut f = frame(0x0238, &[1; 10]);
        f.insert(1, 0xF3);
        f[0] += 1;
        let mut stream = handshake();
        stream.extend(f);
        assert_eq!(decode(&[&stream]), [(0x0238, vec![1; 10])]);
    }

    #[test]
    fn other_traffic_never_produces_packets() {
        let mut decoder = FrameDecoder::new();
        let mut x = 0x1234_5678u32;
        let noise: Vec<u8> = (0..600_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        let mut count = 0;
        for (i, chunk) in noise.chunks(1400).enumerate() {
            decoder.feed(chunk, &mut |_, _| count += 1);
            // far more chance heartbeats than a real download would have
            if i % 8 == 0 {
                decoder.feed(&heartbeat(), &mut |_, _| count += 1);
            }
        }
        assert_eq!(count, 0);
        assert!(decoder.is_dead());
    }

    #[test]
    fn resyncs_on_the_next_heartbeat_after_a_gap() {
        let mut decoder = FrameDecoder::new();
        let mut got = Vec::new();
        let mut stream = handshake();
        stream.extend(frame(0x0238, &[1; 8]));
        decoder.feed(&stream, &mut |op, _| got.push(op));
        decoder.reset();
        let mut after = frame(0x0238, &[2; 50])[7..].to_vec(); // tail of a frame we lost the start of
        after.extend(heartbeat());
        after.extend(frame(0x4738, &[3; 5]));
        decoder.feed(&after, &mut |op, _| got.push(op));
        got.retain(|&op| op != HEARTBEAT);
        assert_eq!(got, [0x0238, 0x4738]);
    }
}
