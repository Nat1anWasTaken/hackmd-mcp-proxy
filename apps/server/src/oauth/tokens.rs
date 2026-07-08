use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hmac_sha256_hex(value: &str, key: &str) -> String {
    let Ok(mut mac) = HmacSha256::new_from_slice(key.as_bytes()) else {
        tracing::error!("invalid HMAC key length");
        return String::new();
    };
    mac.update(value.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::{hmac_sha256_hex, random_token};

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
}
