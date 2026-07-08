use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum PkceError {
    #[error("code_challenge_method must be S256")]
    UnsupportedMethod,
    #[error("code_verifier is invalid")]
    InvalidVerifier,
    #[error("code_verifier does not match code_challenge")]
    ChallengeMismatch,
}

pub fn verify_s256(
    code_verifier: &str,
    code_challenge: &str,
    code_challenge_method: &str,
) -> Result<(), PkceError> {
    if code_challenge_method != "S256" {
        return Err(PkceError::UnsupportedMethod);
    }
    if !(43..=128).contains(&code_verifier.len())
        || !code_verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
    {
        return Err(PkceError::InvalidVerifier);
    }

    let digest = Sha256::digest(code_verifier.as_bytes());
    let expected = URL_SAFE_NO_PAD.encode(digest);
    if expected == code_challenge {
        Ok(())
    } else {
        Err(PkceError::ChallengeMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::{verify_s256, PkceError};

    #[test]
    fn verifies_s256_challenge() -> anyhow::Result<()> {
        verify_s256(
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            "S256",
        )?;
        Ok(())
    }

    #[test]
    fn rejects_mismatched_challenge() {
        assert_eq!(
            verify_s256(
                "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
                "wrong",
                "S256",
            ),
            Err(PkceError::ChallengeMismatch)
        );
    }
}
