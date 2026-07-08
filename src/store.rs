use std::{
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use uuid::Uuid;

use crate::{
    crypto::{hmac_sha256_hex, random_token},
    oauth::{OAuthClient, ScopeSet, verify_s256},
};

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

    async fn init_schema(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS github_users (
                github_id INTEGER PRIMARY KEY,
                github_login TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS web_sessions (
                session_hash TEXT PRIMARY KEY,
                github_user_id INTEGER NOT NULL REFERENCES github_users(github_id) ON DELETE CASCADE,
                expires_at INTEGER NOT NULL,
                revoked_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS github_oauth_states (
                state_hash TEXT PRIMARY KEY,
                return_to TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                consumed_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS hackmd_credentials (
                github_user_id INTEGER PRIMARY KEY REFERENCES github_users(github_id) ON DELETE CASCADE,
                nonce BLOB NOT NULL,
                ciphertext BLOB NOT NULL,
                fingerprint TEXT NOT NULL,
                verified_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS oauth_clients (
                client_id TEXT PRIMARY KEY,
                client_name TEXT,
                redirect_uris_json TEXT NOT NULL,
                token_endpoint_auth_method TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS oauth_authorization_codes (
                code_hash TEXT PRIMARY KEY,
                github_user_id INTEGER NOT NULL REFERENCES github_users(github_id) ON DELETE CASCADE,
                client_id TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
                redirect_uri TEXT NOT NULL,
                resource TEXT NOT NULL,
                scopes TEXT NOT NULL,
                code_challenge TEXT NOT NULL,
                code_challenge_method TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                consumed_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS oauth_access_tokens (
                token_hash TEXT PRIMARY KEY,
                github_user_id INTEGER NOT NULL REFERENCES github_users(github_id) ON DELETE CASCADE,
                client_id TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
                resource TEXT NOT NULL,
                scopes TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                revoked_at INTEGER
            );
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn register_client(
        &self,
        client_name: Option<String>,
        redirect_uris: Vec<String>,
    ) -> Result<OAuthClient, StoreError> {
        let client = OAuthClient {
            client_id: format!("client_{}", Uuid::new_v4().simple()),
            client_name,
            redirect_uris,
            token_endpoint_auth_method: "none".to_owned(),
        };
        sqlx::query(
            r#"
            INSERT INTO oauth_clients (
                client_id, client_name, redirect_uris_json, token_endpoint_auth_method, created_at
            ) VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(&client.client_id)
        .bind(&client.client_name)
        .bind(serde_json::to_string(&client.redirect_uris)?)
        .bind(&client.token_endpoint_auth_method)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(client)
    }

    pub async fn find_client(&self, client_id: &str) -> Result<Option<OAuthClient>, StoreError> {
        let Some(row) = sqlx::query(
            "SELECT client_id, client_name, redirect_uris_json, token_endpoint_auth_method FROM oauth_clients WHERE client_id = ?",
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };

        let redirect_uris_json: String = row.try_get("redirect_uris_json")?;
        Ok(Some(OAuthClient {
            client_id: row.try_get("client_id")?,
            client_name: row.try_get("client_name")?,
            redirect_uris: serde_json::from_str(&redirect_uris_json)?,
            token_endpoint_auth_method: row.try_get("token_endpoint_auth_method")?,
        }))
    }

    pub async fn upsert_github_user(
        &self,
        github_id: i64,
        github_login: &str,
    ) -> Result<GitHubUser, StoreError> {
        sqlx::query(
            r#"
            INSERT INTO github_users (github_id, github_login, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(github_id) DO UPDATE SET
                github_login = excluded.github_login,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(github_id)
        .bind(github_login)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(GitHubUser {
            github_id,
            github_login: github_login.to_owned(),
        })
    }

    pub async fn create_github_state(
        &self,
        state_hash: &str,
        return_to: &str,
        ttl: Duration,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO github_oauth_states (state_hash, return_to, expires_at) VALUES (?, ?, ?)",
        )
        .bind(state_hash)
        .bind(return_to)
        .bind(now() + ttl.as_secs() as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn consume_github_state(
        &self,
        state_hash: &str,
    ) -> Result<Option<String>, StoreError> {
        let Some(row) = sqlx::query(
            "SELECT return_to, expires_at, consumed_at FROM github_oauth_states WHERE state_hash = ?",
        )
        .bind(state_hash)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let expires_at: i64 = row.try_get("expires_at")?;
        let consumed_at: Option<i64> = row.try_get("consumed_at")?;
        if expires_at <= now() || consumed_at.is_some() {
            return Ok(None);
        }
        sqlx::query("UPDATE github_oauth_states SET consumed_at = ? WHERE state_hash = ?")
            .bind(now())
            .bind(state_hash)
            .execute(&self.pool)
            .await?;
        Ok(Some(row.try_get("return_to")?))
    }

    pub async fn create_web_session(
        &self,
        session_hash: &str,
        github_user_id: i64,
        ttl: Duration,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO web_sessions (session_hash, github_user_id, expires_at) VALUES (?, ?, ?)",
        )
        .bind(session_hash)
        .bind(github_user_id)
        .bind(now() + ttl.as_secs() as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn validate_web_session(
        &self,
        session_hash: &str,
    ) -> Result<Option<GitHubUser>, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT u.github_id, u.github_login
            FROM web_sessions s
            JOIN github_users u ON u.github_id = s.github_user_id
            WHERE s.session_hash = ? AND s.expires_at > ? AND s.revoked_at IS NULL
            "#,
        )
        .bind(session_hash)
        .bind(now())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| GitHubUser {
            github_id: row.get("github_id"),
            github_login: row.get("github_login"),
        }))
    }

    pub async fn store_hackmd_credential(
        &self,
        github_user_id: i64,
        nonce: Vec<u8>,
        ciphertext: Vec<u8>,
        fingerprint: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO hackmd_credentials (
                github_user_id, nonce, ciphertext, fingerprint, verified_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(github_user_id) DO UPDATE SET
                nonce = excluded.nonce,
                ciphertext = excluded.ciphertext,
                fingerprint = excluded.fingerprint,
                verified_at = excluded.verified_at,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(github_user_id)
        .bind(nonce)
        .bind(ciphertext)
        .bind(fingerprint)
        .bind(now())
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_hackmd_credential(
        &self,
        github_user_id: i64,
    ) -> Result<Option<CredentialRecord>, StoreError> {
        let row = sqlx::query(
            "SELECT nonce, ciphertext, fingerprint FROM hackmd_credentials WHERE github_user_id = ?",
        )
        .bind(github_user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| CredentialRecord {
            nonce: row.get("nonce"),
            ciphertext: row.get("ciphertext"),
            fingerprint: row.get("fingerprint"),
        }))
    }

    pub async fn delete_hackmd_credential(&self, github_user_id: i64) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM hackmd_credentials WHERE github_user_id = ?")
            .bind(github_user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn issue_authorization_code(
        &self,
        input: AuthorizeInput,
        github_user_id: i64,
        ttl: Duration,
        hash_key: &str,
        expected_resource: &str,
    ) -> Result<IssuedCode, StoreError> {
        let client = self
            .find_client(&input.client_id)
            .await?
            .ok_or(StoreError::ClientNotFound)?;
        if !client.allows_redirect_uri(&input.redirect_uri) {
            return Err(StoreError::RedirectUriMismatch);
        }
        if input.resource != expected_resource {
            return Err(StoreError::ResourceMismatch);
        }
        if input.code_challenge_method != "S256" {
            return Err(StoreError::Pkce(crate::oauth::PkceError::UnsupportedMethod));
        }

        let code = random_token();
        let code_hash = hmac_sha256_hex(&code, hash_key);
        sqlx::query(
            r#"
            INSERT INTO oauth_authorization_codes (
                code_hash, github_user_id, client_id, redirect_uri, resource, scopes,
                code_challenge, code_challenge_method, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&code_hash)
        .bind(github_user_id)
        .bind(&input.client_id)
        .bind(&input.redirect_uri)
        .bind(&input.resource)
        .bind(input.scopes.as_space_delimited())
        .bind(&input.code_challenge)
        .bind(&input.code_challenge_method)
        .bind(now() + ttl.as_secs() as i64)
        .execute(&self.pool)
        .await?;

        Ok(IssuedCode {
            code,
            redirect_uri: input.redirect_uri,
            state: input.state,
        })
    }

    pub async fn exchange_code(
        &self,
        input: ExchangeCodeInput<'_>,
    ) -> Result<IssuedAccessToken, StoreError> {
        let code_hash = hmac_sha256_hex(input.code, input.code_hash_key);
        let Some(row) = sqlx::query(
            r#"
            SELECT github_user_id, client_id, redirect_uri, resource, scopes, code_challenge,
                   code_challenge_method, expires_at, consumed_at
            FROM oauth_authorization_codes
            WHERE code_hash = ?
            "#,
        )
        .bind(&code_hash)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Err(StoreError::InvalidAuthorizationCode);
        };

        let client_id: String = row.try_get("client_id")?;
        let redirect_uri: String = row.try_get("redirect_uri")?;
        let expires_at: i64 = row.try_get("expires_at")?;
        let consumed_at: Option<i64> = row.try_get("consumed_at")?;

        if client_id != input.client_id || redirect_uri != input.redirect_uri {
            return Err(StoreError::InvalidAuthorizationCode);
        }
        if expires_at <= now() {
            return Err(StoreError::ExpiredAuthorizationCode);
        }
        if consumed_at.is_some() {
            return Err(StoreError::ConsumedAuthorizationCode);
        }

        let code_challenge: String = row.try_get("code_challenge")?;
        let code_challenge_method: String = row.try_get("code_challenge_method")?;
        verify_s256(input.code_verifier, &code_challenge, &code_challenge_method)?;

        sqlx::query("UPDATE oauth_authorization_codes SET consumed_at = ? WHERE code_hash = ?")
            .bind(now())
            .bind(&code_hash)
            .execute(&self.pool)
            .await?;

        let token = random_token();
        let token_hash = hmac_sha256_hex(&token, input.access_token_hash_key);
        let scope: String = row.try_get("scopes")?;
        sqlx::query(
            r#"
            INSERT INTO oauth_access_tokens (
                token_hash, github_user_id, client_id, resource, scopes, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&token_hash)
        .bind(row.get::<i64, _>("github_user_id"))
        .bind(&client_id)
        .bind(row.get::<String, _>("resource"))
        .bind(&scope)
        .bind(now() + input.access_token_ttl.as_secs() as i64)
        .execute(&self.pool)
        .await?;

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
    ) -> Result<Option<AccessTokenContext>, StoreError> {
        let token_hash = hmac_sha256_hex(token, hash_key);
        let Some(row) = sqlx::query(
            r#"
            SELECT github_user_id, client_id, resource, scopes, expires_at, revoked_at
            FROM oauth_access_tokens
            WHERE token_hash = ?
            "#,
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let token_resource: String = row.try_get("resource")?;
        let expires_at: i64 = row.try_get("expires_at")?;
        let revoked_at: Option<i64> = row.try_get("revoked_at")?;
        if token_resource != resource || expires_at <= now() || revoked_at.is_some() {
            return Ok(None);
        }
        Ok(Some(AccessTokenContext {
            github_user_id: row.try_get("github_user_id")?,
            client_id: row.try_get("client_id")?,
            scopes: ScopeSet::parse(Some(row.get::<String, _>("scopes").as_str()))
                .map_err(|_| StoreError::InvalidAuthorizationCode)?,
        }))
    }

    pub async fn revoke_access_token(&self, token: &str, hash_key: &str) -> Result<(), StoreError> {
        let token_hash = hmac_sha256_hex(token, hash_key);
        sqlx::query("UPDATE oauth_access_tokens SET revoked_at = ? WHERE token_hash = ?")
            .bind(now())
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
