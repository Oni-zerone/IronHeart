//! App keys: generation, secret encryption and HMAC signature of pings.

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Payload},
};
use anyhow::{anyhow, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Signed path: it is part of the canonical string.
pub const HEARTBEAT_PATH: &str = "/api/v1/heartbeat";

const NONCE_LEN: usize = 12;
/// Alphabet without ambiguous characters (0/o, 1/l): 32 symbols, so no bias.
const KEY_ID_ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";
const KEY_ID_LEN: usize = 12;

/// Encrypts and decrypts app secrets with the master key (`SECRETS_KEY`).
pub struct SecretBox {
    cipher: Aes256Gcm,
    pub version: i64,
}

impl SecretBox {
    pub fn new(key: &[u8; 32], version: i64) -> Self {
        Self { cipher: Aes256Gcm::new_from_slice(key).expect("32-byte key"), version }
    }

    /// Returns nonce + ciphertext. `app_id` is bound to the ciphertext,
    /// so a secret cannot be moved to another app.
    pub fn seal(&self, plaintext: &[u8], app_id: &str) -> Vec<u8> {
        let nonce: [u8; NONCE_LEN] = rand::random();
        let ciphertext = self
            .cipher
            .encrypt(&nonce.into(), Payload { msg: plaintext, aad: app_id.as_bytes() })
            .expect("AES-GCM encryption");
        [nonce.as_slice(), &ciphertext].concat()
    }

    pub fn open(&self, sealed: &[u8], version: i64, app_id: &str) -> anyhow::Result<Vec<u8>> {
        if version != self.version {
            bail!("secret encrypted with version {version}, current master key {}", self.version);
        }
        if sealed.len() <= NONCE_LEN {
            bail!("encrypted secret too short");
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("length checked");
        self.cipher
            .decrypt(&nonce.into(), Payload { msg: ciphertext, aad: app_id.as_bytes() })
            .map_err(|_| anyhow!("secret cannot be decrypted (wrong master key?)"))
    }
}

pub fn generate_key_id() -> String {
    rand::random::<[u8; KEY_ID_LEN]>()
        .iter()
        .map(|b| KEY_ID_ALPHABET[(b & 31) as usize] as char)
        .collect()
}

/// The secret is a string: the client uses it as is as the HMAC key.
pub fn generate_secret() -> String {
    format!("ihs_{}", URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()))
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// `v1\nPOST\n/api/v1/heartbeat\n<key_id>\n<timestamp>\n<sha256 hex of the body>`
pub fn canonical_string(key_id: &str, timestamp: &str, body_sha256_hex: &str) -> String {
    format!("v1\nPOST\n{HEARTBEAT_PATH}\n{key_id}\n{timestamp}\n{body_sha256_hex}")
}

/// Signature of a request, as a client computes it (entanglement too).
pub fn sign(secret: &[u8], canonical: &str) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts keys of any length");
    mac.update(canonical.as_bytes());
    mac.finalize().into_bytes().into()
}

/// Constant-time comparison.
pub fn verify(secret: &[u8], canonical: &str, signature: &[u8]) -> bool {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts keys of any length");
    mac.update(canonical.as_bytes());
    mac.verify_slice(signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_and_open_bind_the_app() {
        let secret_box = SecretBox::new(&[7; 32], 1);
        let sealed = secret_box.seal(b"secret", "app-1");
        assert_eq!(secret_box.open(&sealed, 1, "app-1").unwrap(), b"secret");
        assert!(secret_box.open(&sealed, 1, "app-2").is_err());
        assert!(secret_box.open(&sealed, 2, "app-1").is_err());
        assert!(SecretBox::new(&[8; 32], 1).open(&sealed, 1, "app-1").is_err());
    }

    #[test]
    fn generated_keys_have_expected_shape() {
        let key_id = generate_key_id();
        assert_eq!(key_id.len(), KEY_ID_LEN);
        assert!(key_id.bytes().all(|b| KEY_ID_ALPHABET.contains(&b)));
        assert!(generate_secret().starts_with("ihs_"));
        assert_ne!(generate_secret(), generate_secret());
    }

    /// Same computation as the openssl example: it is the reference for clients.
    #[test]
    fn signature_matches_reference_vector() {
        let canonical = canonical_string("abcdefghijkm", "1764000000", &sha256_hex(b""));
        assert_eq!(
            canonical,
            "v1\nPOST\n/api/v1/heartbeat\nabcdefghijkm\n1764000000\n\
             e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let signature = sign(b"ihs_test", &canonical);
        // Same values as docs/heartbeat-client.md: if they change, update the document.
        assert_eq!(hex::encode(signature), "395ebdbcd9e58a46090ac3c6ca626a0e00b94301076532b944841b3338cd5f01");
        let with_body = canonical_string("abcdefghijkm", "1764000000", &sha256_hex(br#"{"status":"ok"}"#));
        assert_eq!(
            hex::encode(sign(b"ihs_test", &with_body)),
            "f9cefe637f8e3e4b9688b79f948bf544702d7b76e983f927d6293749d20fe9b3"
        );
        assert!(verify(b"ihs_test", &canonical, &signature));
        assert!(!verify(b"ihs_other", &canonical, &signature));
        assert!(!verify(b"ihs_test", &canonical, &signature[..31]));
    }
}
