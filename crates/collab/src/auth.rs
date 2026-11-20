//! Signing in without passwords: each PC holds its own Ed25519 key (the *device key*), and the
//! person is who Windows says they are (`DOMAIN\user`).
//!
//! At login the server sends a fresh nonce; the client answers with an Ed25519 signature over
//! `nonce || user || device public key` ([`login_message`]). The server checks it against the
//! public key the client announced and against the devices it knows for that person: a person's
//! first device is trusted on first use, any further one waits for an administrator.

use sha2::{Digest, Sha256};

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

/// Length of an Ed25519 public key.
pub const PUBLIC_KEY_LEN: usize = 32;

/// What a client signs to log in: the server's nonce, the user name exactly as sent in `Hello`,
/// and the device's public key. Binding the name and the key stops a signature made for one
/// person or device from being replayed for another.
pub fn login_message(nonce: &[u8], user: &str, public_key: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(nonce.len() + user.len() + public_key.len() + 2);
    m.extend_from_slice(nonce);
    m.push(0);
    m.extend_from_slice(user.as_bytes());
    m.push(0);
    m.extend_from_slice(public_key);
    m
}

/// The short code people and administrators use to tell devices apart: the first 8 base32
/// characters of SHA-256(public key), as `ABCD-EFGH`.
pub fn device_code(public_key: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let h = Sha256::digest(public_key);
    // 8 characters × 5 bits = the first 40 bits (5 bytes) of the hash.
    let mut bits: u64 = 0;
    for b in h.iter().take(5) {
        bits = (bits << 8) | u64::from(*b);
    }
    let mut out = String::with_capacity(9);
    for i in 0..8u32 {
        let idx = ((bits >> (35 - 5 * i)) & 31) as usize;
        if i == 4 {
            out.push('-');
        }
        out.push(char::from(ALPHABET.get(idx).copied().unwrap_or(b'A')));
    }
    out
}

/// A device code as typed by a person: upper case, the dash optional. `None` if it can't be one.
pub fn normalize_code(code: &str) -> Option<String> {
    let c: String = code.chars().filter(|c| !matches!(c, '-' | ' ')).map(|c| c.to_ascii_uppercase()).collect();
    if c.len() != 8 || !c.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)) {
        return None;
    }
    Some(format!("{}-{}", c.get(..4)?, c.get(4..)?))
}

/// This PC's signing key. The private half never leaves the PC.
pub struct DeviceKey {
    #[cfg(not(target_arch = "wasm32"))]
    pair: ring::signature::Ed25519KeyPair,
    pkcs8: Vec<u8>,
}

impl std::fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DeviceKey({})", device_code(&self.public_key()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl DeviceKey {
    /// A new random key.
    pub fn generate() -> crate::Result<DeviceKey> {
        let rng = ring::rand::SystemRandom::new();
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| crate::Error::Protocol("could not make a device key".into()))?;
        DeviceKey::from_pkcs8(doc.as_ref())
    }

    /// A key saved before ([`DeviceKey::pkcs8`]).
    pub fn from_pkcs8(bytes: &[u8]) -> crate::Result<DeviceKey> {
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(bytes)
            .map_err(|_| crate::Error::Protocol("the saved device key is damaged".into()))?;
        Ok(DeviceKey { pair, pkcs8: bytes.to_vec() })
    }

    pub fn public_key(&self) -> Vec<u8> {
        use ring::signature::KeyPair;
        self.pair.public_key().as_ref().to_vec()
    }

    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.pair.sign(msg).as_ref().to_vec()
    }
}

#[cfg(target_arch = "wasm32")]
impl DeviceKey {
    pub fn generate() -> crate::Result<DeviceKey> {
        Err(crate::Error::Protocol("the office server is not available in the browser".into()))
    }
    pub fn from_pkcs8(_bytes: &[u8]) -> crate::Result<DeviceKey> {
        DeviceKey::generate()
    }
    pub fn public_key(&self) -> Vec<u8> {
        Vec::new()
    }
    pub fn sign(&self, _msg: &[u8]) -> Vec<u8> {
        Vec::new()
    }
}

impl DeviceKey {
    /// The key to store (PKCS#8). Secret: keep it protected.
    pub fn pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    pub fn code(&self) -> String {
        device_code(&self.public_key())
    }
}

/// Checks an Ed25519 signature.
pub fn verify(public_key: &[u8], msg: &[u8], signature: &[u8]) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        public_key.len() == PUBLIC_KEY_LEN
            && ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key).verify(msg, signature).is_ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (public_key, msg, signature);
        false
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn a_signature_proves_the_key_the_nonce_and_the_user() {
        let k = DeviceKey::generate().expect("key");
        let pk = k.public_key();
        let msg = login_message(&[7; 24], r"DDS\vikram", &pk);
        let sig = k.sign(&msg);
        assert!(verify(&pk, &msg, &sig));
        assert!(!verify(&pk, &login_message(&[8; 24], r"DDS\vikram", &pk), &sig), "another nonce");
        assert!(!verify(&pk, &login_message(&[7; 24], r"DDS\anita", &pk), &sig), "another user");
        let other = DeviceKey::generate().expect("key");
        assert!(!verify(&other.public_key(), &msg, &sig), "another device");
        assert!(!verify(&pk[..31], &msg, &sig), "a short key is refused, not a crash");
        // A saved key is the same key.
        let again = DeviceKey::from_pkcs8(k.pkcs8()).expect("reload");
        assert_eq!(again.public_key(), pk);
        assert!(DeviceKey::from_pkcs8(b"junk").is_err());
    }

    #[test]
    fn device_codes_are_short_stable_and_typed_back() {
        let code = device_code(&[1u8; 32]);
        assert_eq!(code.len(), 9);
        assert_eq!(code.chars().nth(4), Some('-'));
        assert!(code.chars().filter(|c| *c != '-').all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
        assert_eq!(code, device_code(&[1u8; 32]));
        assert_ne!(code, device_code(&[2u8; 32]));
        // SHA-256 of the empty string starts e3 b0 c4 42 98:
        // 11100 01110 11000 01100 | 01000 10000 10100 11000 = 28 14 24 12 | 8 16 20 24.
        assert_eq!(device_code(b""), "4OYM-IQUY");
        assert_eq!(normalize_code("4oym iquy").as_deref(), Some("4OYM-IQUY"));
        assert_eq!(normalize_code("4OYM-IQU1"), None);
        assert_eq!(normalize_code("short"), None);
    }
}
