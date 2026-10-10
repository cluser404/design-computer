//! A small, deterministic PNG encoder (8-bit RGB). Compression is greedy LZ77 with fixed Huffman
//! codes: simple, and very effective on renders, which are mostly flat colour.

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    /// Writes `count` bits of `v`, least significant bit first (how DEFLATE packs extra bits).
    fn put(&mut self, v: u32, count: u32) {
        self.acc |= v << self.n;
        self.n += count;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// Writes a Huffman code, most significant bit first.
    fn code(&mut self, code: u32, len: u32) {
        let mut rev = 0;
        for i in 0..len {
            rev |= ((code >> i) & 1) << (len - 1 - i);
        }
        self.put(rev, len);
    }

    fn literal_length(&mut self, sym: u32) {
        match sym {
            0..=143 => self.code(0x30 + sym, 8),
            144..=255 => self.code(0x190 + (sym - 144), 9),
            256..=279 => self.code(sym - 256, 7),
            _ => self.code(0xC0 + (sym - 280), 8),
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Compresses `data` as one fixed-Huffman DEFLATE block.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut b = Bits {
        out: Vec::with_capacity(data.len() / 4 + 16),
        acc: 0,
        n: 0,
    };
    b.put(1, 1); // final block
    b.put(1, 2); // fixed Huffman
    const WINDOW: usize = 32_768;
    const HASH: usize = 1 << 15;
    let mut head = vec![usize::MAX; HASH];
    let mut prev = vec![usize::MAX; data.len()];
    let hash = |i: usize| -> usize {
        ((usize::from(data[i]) << 10) ^ (usize::from(data[i + 1]) << 5) ^ usize::from(data[i + 2]))
            & (HASH - 1)
    };
    let mut i = 0;
    while i < data.len() {
        let (mut best_len, mut best_dist) = (0usize, 0usize);
        if i + 3 <= data.len() {
            let mut cand = head[hash(i)];
            let mut tries = 0;
            while cand != usize::MAX && i - cand <= WINDOW && tries < 48 {
                let max = (data.len() - i).min(258);
                let mut l = 0;
                while l < max && data[cand + l] == data[i + l] {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = i - cand;
                    if l == max {
                        break;
                    }
                }
                cand = prev[cand];
                tries += 1;
            }
        }
        let advance = if best_len >= 3 {
            let li = LEN_BASE
                .iter()
                .rposition(|&base| base as usize <= best_len)
                .expect("length >= 3");
            b.literal_length(257 + li as u32);
            b.put(
                (best_len - LEN_BASE[li] as usize) as u32,
                u32::from(LEN_EXTRA[li]),
            );
            let di = DIST_BASE
                .iter()
                .rposition(|&base| base as usize <= best_dist)
                .expect("distance >= 1");
            b.code(di as u32, 5);
            b.put(
                (best_dist - DIST_BASE[di] as usize) as u32,
                u32::from(DIST_EXTRA[di]),
            );
            best_len
        } else {
            b.literal_length(u32::from(data[i]));
            1
        };
        // Index every position we skip over, so later matches can find them.
        for (k, p) in prev.iter_mut().enumerate().skip(i).take(advance) {
            if k + 3 <= data.len() {
                let h = hash(k);
                *p = head[h];
                head[h] = k;
            }
        }
        i += advance;
    }
    b.literal_length(256);
    b.finish()
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

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
static CRC: [u32; 256] = crc_table();

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = CRC[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Encodes 8-bit RGB pixels (`width * height * 3` bytes, row-major) as a PNG file.
pub fn encode_rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    assert_eq!(
        rgb.len(),
        width as usize * height as usize * 3,
        "pixel buffer size"
    );
    let row = width as usize * 3;
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0); // filter: none
        raw.extend_from_slice(&rgb[y * row..(y + 1) * row]);
    }
    let mut z = vec![0x78, 0x01];
    z.extend(deflate(&raw));
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, RGB, deflate, no filter method, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Inflates the fixed-Huffman stream this module writes (and nothing else).
    pub(crate) fn inflate_fixed(z: &[u8]) -> Vec<u8> {
        let (mut pos, mut bit) = (0usize, 0u32);
        let mut rd = |n: u32| -> u32 {
            let mut v = 0;
            for k in 0..n {
                let b = (z[pos] >> bit) & 1;
                v |= u32::from(b) << k;
                bit += 1;
                if bit == 8 {
                    bit = 0;
                    pos += 1;
                }
            }
            v
        };
        assert_eq!(rd(1), 1, "final block");
        assert_eq!(rd(2), 1, "fixed Huffman");
        let mut out: Vec<u8> = Vec::new();
        loop {
            let (mut code, mut len) = (0u32, 0);
            let sym = loop {
                code = (code << 1) | rd(1);
                len += 1;
                match len {
                    7 if code <= 0b001_0111 => break 256 + code,
                    8 if (0b0011_0000..=0b1011_1111).contains(&code) => break code - 0b0011_0000,
                    8 if (0b1100_0000..=0b1100_0111).contains(&code) => {
                        break 280 + code - 0b1100_0000
                    }
                    9 if (0b1_1001_0000..=0b1_1111_1111).contains(&code) => {
                        break 144 + code - 0b1_1001_0000
                    }
                    _ => assert!(len < 9, "bad code"),
                }
            };
            if sym < 256 {
                out.push(sym as u8);
            } else if sym == 256 {
                return out;
            } else {
                let li = (sym - 257) as usize;
                let length = LEN_BASE[li] as usize + rd(u32::from(LEN_EXTRA[li])) as usize;
                let mut dc = 0u32;
                for _ in 0..5 {
                    dc = (dc << 1) | rd(1);
                }
                let dist = DIST_BASE[dc as usize] as usize
                    + rd(u32::from(DIST_EXTRA[dc as usize])) as usize;
                for _ in 0..length {
                    out.push(out[out.len() - dist]);
                }
            }
        }
    }

    #[test]
    fn checksums_match_known_vectors() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn deflate_round_trips_awkward_inputs() {
        let mut cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![7],
            vec![1, 2, 3],
            vec![0; 100_000],
            (0..=255u8).cycle().take(70_000).collect(),
        ];
        // Pseudo-random bytes: incompressible, exercises literals above 143.
        let mut x = 0x1234_5678u32;
        cases.push(
            (0..20_000)
                .map(|_| {
                    x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (x >> 24) as u8
                })
                .collect(),
        );
        // Long repeats at every distance up to the window.
        let mut v = Vec::new();
        for d in [1usize, 2, 3, 4, 100, 5000, 32_000] {
            v.extend((0..d).map(|i| (i * 31 % 251) as u8));
            let tail: Vec<u8> = v[v.len() - d..].to_vec();
            v.extend_from_slice(&tail);
        }
        cases.push(v);
        for c in cases {
            assert_eq!(inflate_fixed(&deflate(&c)), c, "len {}", c.len());
        }
    }

    #[test]
    fn flat_images_compress_enormously_and_the_file_is_well_formed() {
        let (w, h) = (640u32, 480u32);
        let rgb = vec![200u8; (w * h * 3) as usize];
        let png = encode_rgb(w, h, &rgb);
        // Fixed Huffman with 258-byte matches costs ~13 bits per 258 bytes: about 130:1 at best.
        assert!(
            png.len() < 8_000,
            "{} bytes for a flat 640x480 image",
            png.len()
        );
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), w);
        assert_eq!(&png[png.len() - 12..png.len() - 8], &[0, 0, 0, 0]);
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        assert_eq!(png, encode_rgb(w, h, &rgb), "deterministic");
    }
}
