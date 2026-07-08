use std::collections::BTreeSet;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::config::{Config, Environment};

pub const HACKMD_SCOPE: &str = "hackmd";
pub const SUPPORTED_SCOPES: [&str; 1] = [HACKMD_SCOPE];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScopeSet {
    scopes: BTreeSet<String>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ScopeError {
    #[error("unsupported OAuth scope: {0}")]
    Unsupported(String),
}

impl ScopeSet {
    pub fn parse(raw: Option<&str>) -> Result<Self, ScopeError> {
        let raw = raw.unwrap_or(HACKMD_SCOPE);
        let mut scopes = BTreeSet::new();
        for scope in raw.split_whitespace().filter(|scope| !scope.is_empty()) {
            if !SUPPORTED_SCOPES.contains(&scope) {
                return Err(ScopeError::Unsupported(scope.to_owned()));
            }
            scopes.insert(scope.to_owned());
        }
        if scopes.is_empty() {
            scopes.insert(HACKMD_SCOPE.to_owned());
        }
        Ok(Self { scopes })
    }

    pub fn as_space_delimited(&self) -> String {
        self.scopes.iter().cloned().collect::<Vec<_>>().join(" ")
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ClientRegistrationRequest {
    pub redirect_uris: Vec<String>,
    pub client_name: Option<String>,
    pub token_endpoint_auth_method: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClientRegistrationResponse {
    pub client_id: String,
    pub client_name: Option<String>,
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub grant_types: Vec<&'static str>,
    pub response_types: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_name: Option<String>,
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: String,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ClientRegistrationError {
    #[error("redirect_uris must not be empty")]
    MissingRedirectUri,
    #[error("redirect_uri is not allowed")]
    InvalidRedirectUri,
    #[error("token_endpoint_auth_method must be none")]
    UnsupportedTokenEndpointAuthMethod,
}

impl ClientRegistrationRequest {
    pub fn validate(
        self,
        allow_localhost_redirects: bool,
    ) -> Result<Self, ClientRegistrationError> {
        if self.redirect_uris.is_empty() {
            return Err(ClientRegistrationError::MissingRedirectUri);
        }
        if self
            .redirect_uris
            .iter()
            .any(|uri| !redirect_uri_allowed(uri, allow_localhost_redirects))
        {
            return Err(ClientRegistrationError::InvalidRedirectUri);
        }
        if self
            .token_endpoint_auth_method
            .as_deref()
            .is_some_and(|method| method != "none")
        {
            return Err(ClientRegistrationError::UnsupportedTokenEndpointAuthMethod);
        }
        Ok(self)
    }
}

impl OAuthClient {
    pub fn response(&self) -> ClientRegistrationResponse {
        ClientRegistrationResponse {
            client_id: self.client_id.clone(),
            client_name: self.client_name.clone(),
            redirect_uris: self.redirect_uris.clone(),
            token_endpoint_auth_method: self.token_endpoint_auth_method.clone(),
            grant_types: vec!["authorization_code"],
            response_types: vec!["code"],
        }
    }

    pub fn allows_redirect_uri(&self, redirect_uri: &str) -> bool {
        self.redirect_uris.iter().any(|uri| uri == redirect_uri)
    }
}

pub fn redirect_uri_allowed(uri: &str, allow_localhost_redirects: bool) -> bool {
    uri.starts_with("https://chatgpt.com/connector/oauth/")
        || uri == "https://chatgpt.com/connector_platform_oauth_redirect"
        || (allow_localhost_redirects
            && (uri.starts_with("http://127.0.0.1:")
                || uri.starts_with("http://localhost:")
                || uri.starts_with("https://127.0.0.1:")
                || uri.starts_with("https://localhost:")))
}

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

#[derive(Debug, Serialize)]
pub struct ProtectedResourceMetadata {
    pub resource: String,
    pub authorization_servers: Vec<String>,
    pub scopes_supported: Vec<&'static str>,
    pub resource_documentation: String,
}

#[derive(Debug, Serialize)]
pub struct AuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: String,
    pub revocation_endpoint: String,
    pub response_types_supported: Vec<&'static str>,
    pub grant_types_supported: Vec<&'static str>,
    pub code_challenge_methods_supported: Vec<&'static str>,
    pub token_endpoint_auth_methods_supported: Vec<&'static str>,
    pub scopes_supported: Vec<&'static str>,
}

impl ProtectedResourceMetadata {
    pub fn from_config(config: &Config) -> Self {
        Self {
            resource: config.resource_url(),
            authorization_servers: vec![config.public_base_url.clone()],
            scopes_supported: SUPPORTED_SCOPES.to_vec(),
            resource_documentation: format!("{}/docs", config.public_base_url),
        }
    }
}

impl AuthorizationServerMetadata {
    pub fn from_config(config: &Config) -> Self {
        Self {
            issuer: config.public_base_url.clone(),
            authorization_endpoint: format!("{}/authorize", config.public_base_url),
            token_endpoint: format!("{}/token", config.public_base_url),
            registration_endpoint: format!("{}/register", config.public_base_url),
            revocation_endpoint: format!("{}/revoke", config.public_base_url),
            response_types_supported: vec!["code"],
            grant_types_supported: vec!["authorization_code"],
            code_challenge_methods_supported: vec!["S256"],
            token_endpoint_auth_methods_supported: vec!["none"],
            scopes_supported: SUPPORTED_SCOPES.to_vec(),
        }
    }
}

pub fn localhost_redirects_allowed(environment: Environment) -> bool {
    environment != Environment::Production
}

#[cfg(test)]
mod tests {
    use super::{PkceError, ScopeError, ScopeSet, redirect_uri_allowed, verify_s256};

    #[test]
    fn allows_chatgpt_redirect_uris() {
        assert!(redirect_uri_allowed(
            "https://chatgpt.com/connector/oauth/callback",
            false
        ));
        assert!(redirect_uri_allowed(
            "https://chatgpt.com/connector_platform_oauth_redirect",
            false
        ));
    }

    #[test]
    fn rejects_untrusted_redirect_uris() {
        assert!(!redirect_uri_allowed("https://example.com/callback", false));
    }

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

    #[test]
    fn parses_single_scope() -> anyhow::Result<()> {
        let scopes = ScopeSet::parse(Some("hackmd"))?;
        assert_eq!(scopes.as_space_delimited(), "hackmd");
        Ok(())
    }

    #[test]
    fn rejects_unknown_scope() {
        assert_eq!(
            ScopeSet::parse(Some("offline_access")),
            Err(ScopeError::Unsupported("offline_access".to_owned()))
        );
    }
}
