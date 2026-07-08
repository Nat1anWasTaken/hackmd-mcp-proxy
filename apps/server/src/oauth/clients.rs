use serde::{Deserialize, Serialize};
use thiserror::Error;

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

#[cfg(test)]
mod tests {
    use super::redirect_uri_allowed;

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
}
