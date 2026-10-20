//! FNV-1a 64-bit hashing over a canonical byte encoding.
//!
//! The state hash is how we prove determinism across operating systems and
//! between native and WASM builds (ADR 0002 §5–6). FNV-1a is tiny, has no
//! platform-dependent behaviour and needs no dependency. Every value is
//! written little-endian with a fixed width, so the encoding is canonical.

const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// Incremental FNV-1a 64-bit hasher with fixed-width little-endian writers.
#[derive(Clone, Debug)]
pub struct Fnv1a {
    state: u64,
}

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

impl Fnv1a {
    /// A hasher initialised with the FNV-1a offset basis.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: OFFSET_BASIS,
        }
    }

    /// Feed raw bytes.
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(PRIME);
        }
    }

    /// Feed one byte.
    pub fn write_u8(&mut self, v: u8) {
        self.write(&[v]);
    }

    /// Feed a `u16`, little-endian.
    pub fn write_u16(&mut self, v: u16) {
        self.write(&v.to_le_bytes());
    }

    /// Feed a `u32`, little-endian.
    pub fn write_u32(&mut self, v: u32) {
        self.write(&v.to_le_bytes());
    }

    /// Feed a `u64`, little-endian.
    pub fn write_u64(&mut self, v: u64) {
        self.write(&v.to_le_bytes());
    }

    /// Feed a `u128`, little-endian.
    pub fn write_u128(&mut self, v: u128) {
        self.write(&v.to_le_bytes());
    }

    /// Feed an optional `u32` as a tag byte followed by the value, so `None`
    /// and `Some(0)` encode differently.
    pub fn write_opt_u32(&mut self, v: Option<u32>) {
        match v {
            None => self.write_u8(0),
            Some(x) => {
                self.write_u8(1);
                self.write_u32(x);
            }
        }
    }

    /// The hash of everything written so far.
    #[must_use]
    pub const fn finish(&self) -> u64 {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_reference_vectors() {
        // Published FNV-1a 64 test vectors.
        assert_eq!(Fnv1a::new().finish(), 0xcbf2_9ce4_8422_2325);
        let mut h = Fnv1a::new();
        h.write(b"a");
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
        let mut h = Fnv1a::new();
        h.write(b"foobar");
        assert_eq!(h.finish(), 0x8594_4171_f739_67e8);
    }
}
