//! Little-endian reads over a packet body. Every read is bounds-checked and
//! returns None rather than guessing, so a malformed packet is just dropped.

/// LEB128 varint at `o`: (value, bytes used). None when cut off or longer than 5 bytes.
pub fn varint(b: &[u8], mut o: usize) -> Option<(u32, usize)> {
    let start = o;
    let mut value = 0u32;
    let mut shift = 0;
    loop {
        if shift > 28 {
            return None;
        }
        let byte = *b.get(o)?;
        o += 1;
        value |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some((value, o - start));
        }
        shift += 7;
    }
}

pub struct Reader<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, o: 0 }
    }

    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.o..self.o.checked_add(n)?)?;
        self.o += n;
        Some(s)
    }

    pub fn remaining(&self) -> usize {
        self.b.len() - self.o
    }

    pub fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }

    pub fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    pub fn varint(&mut self) -> Option<u32> {
        let (value, len) = varint(self.b, self.o)?;
        self.o += len;
        Some(value)
    }
}

#[cfg(test)]
pub fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[cfg(test)]
pub fn encode_varint(mut v: u32, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints() {
        assert_eq!(varint(&[0x0E], 0), Some((14, 1)));
        assert_eq!(varint(&[0xC7, 0x43], 0), Some((8647, 2)));
        assert_eq!(varint(&[0xD1, 0xE8, 0x02], 0), Some((46161, 3)));
        assert_eq!(varint(&[0x80], 0), None);
        assert_eq!(varint(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01], 0), None);
        let mut out = Vec::new();
        encode_varint(300_000, &mut out);
        assert_eq!(varint(&out, 0), Some((300_000, out.len())));
    }

    #[test]
    fn reads_stop_at_the_end() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert_eq!(r.u16(), Some(0x0201));
        assert_eq!(r.u16(), None);
        assert_eq!(r.u8(), Some(3));
        assert_eq!(r.u8(), None);
    }
}
