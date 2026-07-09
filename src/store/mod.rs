use std::{
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::oauth::ScopeSet;

mod clients;
mod credentials;
mod schema;
mod sessions;
mod tokens;
mod users;

#[derive(Clone, Debug)]
pub struct Store {
    pool: SqlitePool,
}

#[derive(Clone, Debug)]
pub struct GitHubUser {
    pub github_id: i64,
    pub github_login: String,
}

#[derive(Clone, Debug)]
pub struct CredentialRecord {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub fingerprint: String,
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
pub struct IssuedCode {
    pub code: String,
    pub redirect_uri: String,
    pub state: Option<String>,
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
pub struct IssuedAccessToken {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub scope: String,
}

#[derive(Clone, Debug)]
pub struct AccessTokenContext {
    pub github_user_id: i64,
    pub client_id: String,
    pub scopes: ScopeSet,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
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
    Pkce(#[from] crate::oauth::PkceError),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

impl Store {
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let options = SqliteConnectOptions::from_str(database_url)
            .with_context(|| format!("invalid DATABASE_URL {database_url}"))?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .context("failed to connect to SQLite")?;
        let store = Self { pool };
        store.init_schema().await?;
        Ok(store)
    }

    #[cfg(test)]
    pub async fn in_memory() -> anyhow::Result<Self> {
        Self::connect("sqlite::memory:").await
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
