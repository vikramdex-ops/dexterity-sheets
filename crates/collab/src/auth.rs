//! Passwords: PBKDF2-HMAC-SHA256 with a per-user salt, and a challenge-response login so the
//! password (and the stored key) never cross the network.
//!
//! The server keeps `key = PBKDF2(password, salt)`. At login it sends the salt and a fresh nonce;
//! the client derives the key and answers `HMAC(key, nonce)`.

use sha2::{Digest, Sha256};

pub const ITERATIONS: u32 = 120_000;
const BLOCK: usize = 64;

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        let h = Sha256::digest(key);
        k[..32].copy_from_slice(&h);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let ih = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(ih);
    let mut out = [0u8; 32];
    out.copy_from_slice(&outer.finalize());
    out
}

/// PBKDF2-HMAC-SHA256 with one 32-byte block.
pub fn pbkdf2(password: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut msg = salt.to_vec();
    msg.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_sha256(password.as_bytes(), &msg);
    let mut out = u;
    for _ in 1..iterations.max(1) {
        u = hmac_sha256(password.as_bytes(), &u);
        for (o, x) in out.iter_mut().zip(u.iter()) {
            *o ^= x;
        }
    }
    out
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    // Without an OS random source, fall back to the clock: a weaker nonce, never a crash.
    if getrandom::fill(&mut b).is_err() {
        let t = crate::now_ms().to_le_bytes();
        for (i, x) in b.iter_mut().enumerate() {
            *x = t[i % 8] ^ (i as u8).wrapping_mul(31);
        }
    }
    b
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect()
}

/// Constant-time comparison.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The answer to a login challenge.
pub fn proof(password: &str, salt_hex: &str, iterations: u32, nonce_hex: &str) -> String {
    let salt = unhex(salt_hex).unwrap_or_default();
    let nonce = unhex(nonce_hex).unwrap_or_default();
    let key = pbkdf2(password, &salt, iterations);
    hex(&hmac_sha256(&key, &nonce))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc4231_case_2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex(&mac), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn pbkdf2_matches_known_vector() {
        // RFC 7914 §11 PBKDF2-HMAC-SHA256 test vector (first 32 bytes).
        let k = pbkdf2("passwd", b"salt", 1);
        assert_eq!(hex(&k), "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc");
    }

    #[test]
    fn proof_round_trip() {
        let salt = hex(&[1, 2, 3, 4]);
        let nonce = hex(&[9; 16]);
        let key = pbkdf2("s3cret", &[1, 2, 3, 4], 10);
        let expect = hex(&hmac_sha256(&key, &[9; 16]));
        assert_eq!(proof("s3cret", &salt, 10, &nonce), expect);
        assert_ne!(proof("wrong", &salt, 10, &nonce), expect);
    }
}
