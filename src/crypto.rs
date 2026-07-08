use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit as AeadKeyInit, OsRng},
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("encryption failed")]
    Encrypt,
    #[error("decryption failed")]
    Decrypt,
}

pub fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hmac_sha256_hex(value: &str, key: &str) -> String {
    let Ok(mut mac) = <HmacSha256 as Mac>::new_from_slice(key.as_bytes()) else {
        tracing::error!("invalid HMAC key length");
        return String::new();
    };
    mac.update(value.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn fingerprint(value: &str, key: &str) -> String {
    hmac_sha256_hex(value, key).chars().take(12).collect()
}

pub fn encrypt_secret(plaintext: &str, key: &[u8; 32]) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce = [0_u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| CryptoError::Encrypt)?;
    Ok((nonce.to_vec(), ciphertext))
}

pub fn decrypt_secret(
    nonce: &[u8],
    ciphertext: &[u8],
    key: &[u8; 32],
) -> Result<String, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let plaintext = cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| CryptoError::Decrypt)?;
    String::from_utf8(plaintext).map_err(|_| CryptoError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::{decrypt_secret, encrypt_secret, fingerprint, hmac_sha256_hex, random_token};

    #[test]
    fn generated_tokens_are_url_safe_and_distinct() {
        let first = random_token();
        let second = random_token();

        assert_ne!(first, second);
        assert!(!first.contains('='));
        assert!(first.len() >= 43);
    }

    #[test]
    fn hmac_hash_is_stable_for_same_key() {
        assert_eq!(
            hmac_sha256_hex("token", "key"),
            hmac_sha256_hex("token", "key")
        );
        assert_ne!(
            hmac_sha256_hex("token", "key"),
            hmac_sha256_hex("token", "other")
        );
    }

    #[test]
    fn secrets_round_trip() -> anyhow::Result<()> {
        let key = [9_u8; 32];
        let (nonce, ciphertext) = encrypt_secret("hmd_secret", &key)?;

        assert_ne!(ciphertext, b"hmd_secret");
        assert_eq!(decrypt_secret(&nonce, &ciphertext, &key)?, "hmd_secret");
        Ok(())
    }

    #[test]
    fn fingerprints_are_short_and_stable() {
        assert_eq!(fingerprint("abc", "k"), fingerprint("abc", "k"));
        assert_eq!(fingerprint("abc", "k").len(), 12);
    }
}
