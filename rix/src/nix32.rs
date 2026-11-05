//! Nix's base-32 encoding ("nix32"), used in store paths and hashes.
//!
//! The alphabet omits e, o, u and t, and bytes are consumed least-significant bit
//! first from the *end* of the buffer, exactly as in Nix's `printHash32`.

pub const ALPHABET: &[u8; 32] = b"0123456789abcdfghijklmnpqrsvwxyz";

pub fn encoded_len(bytes: usize) -> usize {
    (bytes * 8).div_ceil(5)
}

pub fn encode(bytes: &[u8]) -> String {
    let len = encoded_len(bytes.len());
    let mut out = String::with_capacity(len);
    for n in (0..len).rev() {
        let b = n * 5;
        let i = b / 8;
        let j = b % 8;
        let mut c = (bytes[i] as u16) >> j;
        if i + 1 < bytes.len() {
            c |= (bytes[i + 1] as u16) << (8 - j);
        }
        out.push(ALPHABET[(c & 0x1f) as usize] as char);
    }
    out
}

pub fn decode(s: &str) -> Option<Vec<u8>> {
    let bytes_len = s.len() * 5 / 8;
    if encoded_len(bytes_len) != s.len() {
        return None;
    }
    let mut out = vec![0u8; bytes_len];
    for (n, ch) in s.bytes().rev().enumerate() {
        let digit = ALPHABET.iter().position(|&a| a == ch)? as u16;
        let b = n * 5;
        let i = b / 8;
        let j = b % 8;
        out[i] |= (digit << j) as u8;
        let carry = (digit >> (8 - j)) as u8;
        if i + 1 < out.len() {
            out[i + 1] |= carry;
        } else if carry != 0 {
            return None; // Non-canonical: bits beyond the hash length.
        }
    }
    Some(out)
}

pub fn is_valid_char(c: u8) -> bool {
    ALPHABET.contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn sha256_of_empty_matches_nix() {
        // `nix hash file --base32 /dev/null` (empty input)
        let h = Sha256::digest(b"");
        assert_eq!(
            encode(&h),
            "0mdqa9w1p6cmli6976v4wi0sw9r4p5prkj7lzfd1877wk11c9c73"
        );
    }

    #[test]
    fn sha256_of_abc_matches_nix() {
        // `echo -n abc | nix hash file --base32 /dev/stdin`
        let h = Sha256::digest(b"abc");
        assert_eq!(
            encode(&h),
            "1b8m03r63zqhnjf7l5wnldhh7c134ap5vpj0850ymkq1iyzicy5s"
        );
    }

    #[test]
    fn roundtrip() {
        for len in [0usize, 1, 5, 16, 20, 32, 64] {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let e = encode(&data);
            assert_eq!(e.len(), encoded_len(len));
            assert_eq!(decode(&e).unwrap(), data);
        }
        assert!(decode("e000").is_none());
    }
}
