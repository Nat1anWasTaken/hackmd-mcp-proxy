use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime},
};

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::oauth::{
    clients::{ClientRegistrationRequest, OAuthClient},
    pkce,
    scopes::ScopeSet,
    tokens::{hmac_sha256_hex, random_token},
};

#[derive(Clone, Debug, Default)]
pub struct OAuthStore {
    inner: Arc<Mutex<OAuthStoreInner>>,
}

#[derive(Debug, Default)]
struct OAuthStoreInner {
    clients: HashMap<String, OAuthClient>,
    authorization_codes: HashMap<String, AuthorizationCode>,
    access_tokens: HashMap<String, AccessToken>,
}

#[derive(Clone, Debug)]
pub struct AuthorizeInput {
    pub client_id: String,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub resource: String,
    pub scopes: ScopeSet,
    pub code_challenge: String,
    pub code_challenge_method: String,
}

#[derive(Clone, Debug)]
pub struct ExchangeCodeInput<'a> {
    pub code: &'a str,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub code_verifier: &'a str,
    pub code_hash_key: &'a str,
    pub access_token_hash_key: &'a str,
    pub access_token_ttl: Duration,
}

#[derive(Clone, Debug)]
struct AuthorizationCode {
    user_id: String,
    client_id: String,
    redirect_uri: String,
    resource: String,
    scopes: ScopeSet,
    code_challenge: String,
    code_challenge_method: String,
    expires_at: SystemTime,
    consumed_at: Option<SystemTime>,
}

#[derive(Clone, Debug)]
struct AccessToken {
    user_id: String,
    client_id: String,
    resource: String,
    scopes: ScopeSet,
    expires_at: SystemTime,
    revoked_at: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct AccessTokenContext {
    pub user_id: String,
    pub client_id: String,
    pub scopes: ScopeSet,
}

#[derive(Clone, Debug)]
pub struct IssuedCode {
    pub code: String,
    pub redirect_uri: String,
    pub state: Option<String>,
}

#[derive(Clone, Debug)]
pub struct IssuedAccessToken {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub scope: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OAuthStoreError {
    #[error("client not found")]
    ClientNotFound,
    #[error("redirect_uri is not registered for this client")]
    RedirectUriMismatch,
    #[error("resource is invalid")]
    ResourceMismatch,
    #[error("authorization code is invalid")]
    InvalidAuthorizationCode,
    #[error("authorization code is expired")]
    ExpiredAuthorizationCode,
    #[error("authorization code was already consumed")]
    ConsumedAuthorizationCode,
    #[error(transparent)]
    Pkce(#[from] pkce::PkceError),
}

impl OAuthStore {
    pub async fn register_client(&self, request: ClientRegistrationRequest) -> OAuthClient {
        let client = OAuthClient {
            client_id: format!("client_{}", Uuid::new_v4().simple()),
            client_name: request.client_name,
            redirect_uris: request.redirect_uris,
            token_endpoint_auth_method: "none".to_owned(),
        };
        self.inner
            .lock()
            .await
            .clients
            .insert(client.client_id.clone(), client.clone());
        client
    }

    pub async fn issue_authorization_code(
        &self,
        input: AuthorizeInput,
        user_id: String,
        ttl: Duration,
        hash_key: &str,
        expected_resource: &str,
    ) -> Result<IssuedCode, OAuthStoreError> {
        let mut inner = self.inner.lock().await;
        let client = inner
            .clients
            .get(&input.client_id)
            .ok_or(OAuthStoreError::ClientNotFound)?;
        if !client.allows_redirect_uri(&input.redirect_uri) {
            return Err(OAuthStoreError::RedirectUriMismatch);
        }
        if input.resource != expected_resource {
            return Err(OAuthStoreError::ResourceMismatch);
        }
        if input.code_challenge_method != "S256" {
            return Err(OAuthStoreError::Pkce(pkce::PkceError::UnsupportedMethod));
        }

        let code = random_token();
        let code_hash = hmac_sha256_hex(&code, hash_key);
        inner.authorization_codes.insert(
            code_hash,
            AuthorizationCode {
                user_id,
                client_id: input.client_id,
                redirect_uri: input.redirect_uri.clone(),
                resource: input.resource,
                scopes: input.scopes,
                code_challenge: input.code_challenge,
                code_challenge_method: input.code_challenge_method,
                expires_at: SystemTime::now() + ttl,
                consumed_at: None,
            },
        );

        Ok(IssuedCode {
            code,
            redirect_uri: input.redirect_uri,
            state: input.state,
        })
    }

    pub async fn exchange_code(
        &self,
        input: ExchangeCodeInput<'_>,
    ) -> Result<IssuedAccessToken, OAuthStoreError> {
        let code_hash = hmac_sha256_hex(input.code, input.code_hash_key);
        let mut inner = self.inner.lock().await;
        let auth_code = inner
            .authorization_codes
            .get_mut(&code_hash)
            .ok_or(OAuthStoreError::InvalidAuthorizationCode)?;

        if auth_code.client_id != input.client_id || auth_code.redirect_uri != input.redirect_uri {
            return Err(OAuthStoreError::InvalidAuthorizationCode);
        }
        if auth_code.expires_at <= SystemTime::now() {
            return Err(OAuthStoreError::ExpiredAuthorizationCode);
        }
        if auth_code.consumed_at.is_some() {
            return Err(OAuthStoreError::ConsumedAuthorizationCode);
        }
        pkce::verify_s256(
            input.code_verifier,
            &auth_code.code_challenge,
            &auth_code.code_challenge_method,
        )?;

        auth_code.consumed_at = Some(SystemTime::now());
        let token_record = AccessToken {
            user_id: auth_code.user_id.clone(),
            client_id: auth_code.client_id.clone(),
            resource: auth_code.resource.clone(),
            scopes: auth_code.scopes.clone(),
            expires_at: SystemTime::now() + input.access_token_ttl,
            revoked_at: None,
        };
        let scope = auth_code.scopes.as_space_delimited();

        let token = random_token();
        let token_hash = hmac_sha256_hex(&token, input.access_token_hash_key);
        inner.access_tokens.insert(token_hash, token_record);

        Ok(IssuedAccessToken {
            access_token: token,
            token_type: "Bearer",
            expires_in: input.access_token_ttl.as_secs(),
            scope,
        })
    }

    pub async fn validate_access_token(
        &self,
        token: &str,
        hash_key: &str,
        resource: &str,
    ) -> Option<AccessTokenContext> {
        let token_hash = hmac_sha256_hex(token, hash_key);
        let inner = self.inner.lock().await;
        let access_token = inner.access_tokens.get(&token_hash)?;
        if access_token.resource != resource
            || access_token.expires_at <= SystemTime::now()
            || access_token.revoked_at.is_some()
        {
            return None;
        }
        Some(AccessTokenContext {
            user_id: access_token.user_id.clone(),
            client_id: access_token.client_id.clone(),
            scopes: access_token.scopes.clone(),
        })
    }

    pub async fn revoke_access_token(&self, token: &str, hash_key: &str) {
        let token_hash = hmac_sha256_hex(token, hash_key);
        if let Some(access_token) = self.inner.lock().await.access_tokens.get_mut(&token_hash) {
            access_token.revoked_at = Some(SystemTime::now());
        }
    }
}
