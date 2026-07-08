use serde::Serialize;

use crate::{config::Config, oauth::scopes::SUPPORTED_SCOPES};

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
