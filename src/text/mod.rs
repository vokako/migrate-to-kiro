//! Text helpers shared by the resource modules.

pub mod frontmatter;
pub mod imports;
pub mod secrets;

/// FNV-1a 64. Stable across Rust versions, unlike `DefaultHasher`, so hashes
/// written into files stay comparable between runs.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// FNV-1a over several chunks, with a separator so `["ab","c"]` differs from `["a","bc"]`.
pub fn fnv1a_parts<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        for b in p.iter().chain(std::iter::once(&0u8)) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_are_delimited() {
        assert_ne!(fnv1a_parts([b"ab".as_slice(), b"c".as_slice()]), fnv1a_parts([b"a".as_slice(), b"bc".as_slice()]));
        assert_eq!(fnv1a(b"a"), 0xaf63dc4c8601ec8c);
    }
}
