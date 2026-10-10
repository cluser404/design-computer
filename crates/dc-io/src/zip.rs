//! A minimal, strict ZIP reader/writer for *stored* (uncompressed) entries.
//!
//! The output is a normal ZIP that standard tools open. Writing is fully deterministic: fixed
//! timestamps, fixed field values, caller-chosen entry order. The reader never panics on hostile
//! input: every offset and length is bounds-checked and malformed data is an error.

use std::fmt;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const END_SIG: u32 = 0x0605_4b50;
/// 1980-01-01, the earliest ZIP date: a constant, so output never depends on the clock.
const DOS_DATE: u16 = 0x0021;
const FLAG_UTF8: u16 = 0x0800;
/// Refuse absurd inputs instead of allocating for them.
const MAX_TOTAL: usize = 1 << 30;
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, PartialEq, Eq)]
pub enum ZipError {
    NotAZip,
    Truncated,
    Unsupported(&'static str),
    BadChecksum(String),
    /// The two copies of an entry's header disagree.
    Corrupt(&'static str),
    TooLarge,
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZipError::NotAZip => write!(f, "not a zip file"),
            ZipError::Truncated => write!(f, "zip file is truncated or has invalid offsets"),
            ZipError::Unsupported(w) => write!(f, "unsupported zip feature: {w}"),
            ZipError::BadChecksum(n) => {
                write!(f, "checksum mismatch in entry '{n}': the file is corrupt")
            }
            ZipError::Corrupt(w) => write!(f, "zip file is corrupt: {w}"),
            ZipError::TooLarge => write!(f, "zip file is too large"),
        }
    }
}

impl std::error::Error for ZipError {}

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}
static CRC_TABLE: [u32; 256] = crc_table();

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = CRC_TABLE[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

fn put16(o: &mut Vec<u8>, v: u16) {
    o.extend_from_slice(&v.to_le_bytes());
}
fn put32(o: &mut Vec<u8>, v: u32) {
    o.extend_from_slice(&v.to_le_bytes());
}

/// Writes the entries, in the given order, as a ZIP archive.
pub fn write(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let n = name.as_bytes();
        // Local file header.
        put32(&mut out, LOCAL_SIG);
        put16(&mut out, 10); // version needed: 1.0 (stored)
        put16(&mut out, FLAG_UTF8);
        put16(&mut out, 0); // method: stored
        put16(&mut out, 0); // time
        put16(&mut out, DOS_DATE);
        put32(&mut out, crc);
        put32(&mut out, data.len() as u32);
        put32(&mut out, data.len() as u32);
        put16(&mut out, n.len() as u16);
        put16(&mut out, 0);
        out.extend_from_slice(n);
        out.extend_from_slice(data);
        // Central directory header.
        put32(&mut central, CENTRAL_SIG);
        put16(&mut central, 20); // made by
        put16(&mut central, 10);
        put16(&mut central, FLAG_UTF8);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put16(&mut central, DOS_DATE);
        put32(&mut central, crc);
        put32(&mut central, data.len() as u32);
        put32(&mut central, data.len() as u32);
        put16(&mut central, n.len() as u16);
        put16(&mut central, 0); // extra
        put16(&mut central, 0); // comment
        put16(&mut central, 0); // disk
        put16(&mut central, 0); // internal attrs
        put32(&mut central, 0); // external attrs
        put32(&mut central, offset);
        central.extend_from_slice(n);
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    put32(&mut out, END_SIG);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, entries.len() as u16);
    put16(&mut out, entries.len() as u16);
    put32(&mut out, cd_size);
    put32(&mut out, cd_offset);
    put16(&mut out, 0);
    out
}

fn u16_at(b: &[u8], o: usize) -> Result<u16, ZipError> {
    b.get(o..o + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(ZipError::Truncated)
}
fn u32_at(b: &[u8], o: usize) -> Result<u32, ZipError> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(ZipError::Truncated)
}

/// Reads all entries (name, bytes), verifying every checksum.
pub fn read(b: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ZipError> {
    if b.len() > MAX_TOTAL {
        return Err(ZipError::TooLarge);
    }
    if b.len() < 22 || u32_at(b, 0).ok() != Some(LOCAL_SIG) && u32_at(b, 0).ok() != Some(END_SIG) {
        return Err(ZipError::NotAZip);
    }
    // End-of-central-directory record: scan backwards (it may be followed by a comment).
    let lo = b.len().saturating_sub(22 + 65_535);
    let eocd = (lo..=b.len() - 22)
        .rev()
        .find(|&i| u32_at(b, i).ok() == Some(END_SIG))
        .ok_or(ZipError::NotAZip)?;
    let (disk, cd_disk) = (u16_at(b, eocd + 4)?, u16_at(b, eocd + 6)?);
    if disk != 0 || cd_disk != 0 {
        return Err(ZipError::Unsupported("multi-disk archives"));
    }
    let n = usize::from(u16_at(b, eocd + 10)?);
    let (cd_size, cd_off) = (
        u32_at(b, eocd + 12)? as usize,
        u32_at(b, eocd + 16)? as usize,
    );
    if n > MAX_ENTRIES {
        return Err(ZipError::TooLarge);
    }
    if cd_off.checked_add(cd_size).is_none_or(|e| e > eocd) {
        return Err(ZipError::Truncated);
    }
    let mut out = Vec::with_capacity(n);
    let mut p = cd_off;
    for _ in 0..n {
        if u32_at(b, p)? != CENTRAL_SIG {
            return Err(ZipError::Truncated);
        }
        let method = u16_at(b, p + 10)?;
        let crc = u32_at(b, p + 16)?;
        let (csize, usize_) = (u32_at(b, p + 20)? as usize, u32_at(b, p + 24)? as usize);
        let (nlen, elen, clen) = (
            usize::from(u16_at(b, p + 28)?),
            usize::from(u16_at(b, p + 30)?),
            usize::from(u16_at(b, p + 32)?),
        );
        let local = u32_at(b, p + 42)? as usize;
        if method != 0 {
            return Err(ZipError::Unsupported(
                "compressed entries (this format uses stored entries)",
            ));
        }
        if csize != usize_ || csize == 0xFFFF_FFFF {
            return Err(ZipError::Unsupported("zip64 or inconsistent sizes"));
        }
        let name = b.get(p + 46..p + 46 + nlen).ok_or(ZipError::Truncated)?;
        let name = String::from_utf8(name.to_vec())
            .map_err(|_| ZipError::Unsupported("non-UTF-8 entry name"))?;
        // Local header: re-read name/extra lengths, since the data starts after them.
        if u32_at(b, local)? != LOCAL_SIG {
            return Err(ZipError::Truncated);
        }
        let (lnlen, lelen) = (
            usize::from(u16_at(b, local + 26)?),
            usize::from(u16_at(b, local + 28)?),
        );
        // The name is stored in both headers; a mismatch means one of them was damaged.
        if b.get(local + 30..local + 30 + lnlen) != Some(name.as_bytes()) {
            return Err(ZipError::Corrupt(
                "entry name differs between its two headers",
            ));
        }
        let start = local + 30 + lnlen + lelen;
        let data = start
            .checked_add(csize)
            .and_then(|end| b.get(start..end))
            .ok_or(ZipError::Truncated)?;
        if crc32(data) != crc {
            return Err(ZipError::BadChecksum(name));
        }
        out.push((name, data.to_vec()));
        p += 46 + nlen + elen + clen;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn round_trips_and_is_deterministic() {
        let entries: Vec<(&str, &[u8])> = vec![
            ("a.txt", b"hello"),
            ("dir/b.bin", &[0, 1, 2, 255]),
            ("empty", b""),
        ];
        let z = write(&entries);
        assert_eq!(z, write(&entries));
        let back = read(&z).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back[0], ("a.txt".to_string(), b"hello".to_vec()));
        assert_eq!(back[1].1, vec![0, 1, 2, 255]);
        assert_eq!(back[2].1, Vec::<u8>::new());
    }

    #[test]
    fn corruption_is_detected_and_hostile_input_never_panics() {
        let z = write(&[("data", &[7u8; 300]), ("more", b"xyz")]);
        // Any single flipped byte is an error, a checksum failure, or harmlessly in a field the
        // reader ignores: but it must never panic and never return wrong data silently.
        for i in 0..z.len() {
            let mut bad = z.clone();
            bad[i] ^= 0x55;
            if let Ok(entries) = read(&bad) {
                let orig = read(&z).unwrap();
                assert_eq!(entries, orig, "byte {i} changed data without detection");
            }
        }
        // Every truncation is rejected cleanly.
        for len in 0..z.len() {
            assert!(read(&z[..len]).is_err(), "truncated to {len} bytes");
        }
        assert_eq!(
            read(b"PK nonsense not a zip at all......").unwrap_err(),
            ZipError::NotAZip
        );
        assert!(read(&[]).is_err());
    }
}
