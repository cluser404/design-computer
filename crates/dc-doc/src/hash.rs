use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// 128-bit content hash (truncated BLAKE3). Serialized as 32 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, JsonSchema)]
#[schemars(with = "String")]
pub struct Hash128(pub u128);

impl Hash128 {
    /// Hashes a sequence of byte strings, length-prefixing each so boundaries are unambiguous.
    pub fn of(parts: &[&[u8]]) -> Self {
        let mut h = blake3::Hasher::new();
        for p in parts {
            h.update(&(p.len() as u64).to_le_bytes());
            h.update(p);
        }
        let out = h.finalize();
        let mut b = [0u8; 16];
        b.copy_from_slice(&out.as_bytes()[..16]);
        Hash128(u128::from_le_bytes(b))
    }

    pub fn to_hex(self) -> String {
        format!("{:032x}", self.0)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 32 {
            return None;
        }
        u128::from_str_radix(s, 16).ok().map(Hash128)
    }
}

impl fmt::Display for Hash128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl Serialize for Hash128 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash128 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Hash128::from_hex(&s).ok_or_else(|| serde::de::Error::custom("invalid 128-bit hex hash"))
    }
}
