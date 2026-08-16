use std::time::Duration;

use sqlx::Row;

use super::{
    AccessTokenContext, AuthorizeInput, ExchangeCodeInput, IssuedAccessToken, IssuedCode,
    RefreshTokenInput, Store, StoreError, now,
};
use crate::{
    crypto::{hmac_sha256_hex, random_token},
    oauth::{ScopeSet, verify_s256},
};

impl Store {
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
        let refresh_token = random_token();
        let refresh_token_hash = hmac_sha256_hex(&refresh_token, input.access_token_hash_key);
        let scope: String = row.try_get("scopes")?;
        let github_user_id = row.get::<i64, _>("github_user_id");
        let resource = row.get::<String, _>("resource");
        sqlx::query(
            r#"
            INSERT INTO oauth_access_tokens (
                token_hash, github_user_id, client_id, resource, scopes, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&token_hash)
        .bind(github_user_id)
        .bind(&client_id)
        .bind(&resource)
        .bind(&scope)
        .bind(now() + input.access_token_ttl.as_secs() as i64)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO oauth_refresh_tokens (
                token_hash, github_user_id, client_id, resource, scopes, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(refresh_token_hash)
        .bind(github_user_id)
        .bind(&client_id)
        .bind(resource)
        .bind(&scope)
        .bind(now() + input.refresh_token_ttl.as_secs() as i64)
        .execute(&self.pool)
        .await?;

        Ok(IssuedAccessToken {
            access_token: token,
            token_type: "Bearer",
            expires_in: input.access_token_ttl.as_secs(),
            scope,
            refresh_token,
        })
    }

    pub async fn refresh_access_token(
        &self,
        input: RefreshTokenInput<'_>,
    ) -> Result<IssuedAccessToken, StoreError> {
        let old_hash = hmac_sha256_hex(input.refresh_token, input.token_hash_key);
        let mut transaction = self.pool.begin().await?;
        let Some(row) = sqlx::query(
            r#"
            SELECT github_user_id, client_id, resource, scopes, expires_at, revoked_at
            FROM oauth_refresh_tokens
            WHERE token_hash = ?
            "#,
        )
        .bind(&old_hash)
        .fetch_optional(&mut *transaction)
        .await?
        else {
            return Err(StoreError::InvalidRefreshToken);
        };

        let client_id: String = row.try_get("client_id")?;
        let expires_at: i64 = row.try_get("expires_at")?;
        let revoked_at: Option<i64> = row.try_get("revoked_at")?;
        if client_id != input.client_id || expires_at <= now() || revoked_at.is_some() {
            return Err(StoreError::InvalidRefreshToken);
        }

        let rotated = sqlx::query(
            "UPDATE oauth_refresh_tokens SET revoked_at = ? WHERE token_hash = ? AND revoked_at IS NULL",
        )
        .bind(now())
        .bind(&old_hash)
        .execute(&mut *transaction)
        .await?;
        if rotated.rows_affected() != 1 {
            return Err(StoreError::InvalidRefreshToken);
        }

        let github_user_id: i64 = row.try_get("github_user_id")?;
        let resource: String = row.try_get("resource")?;
        let scope: String = row.try_get("scopes")?;
        let access_token = random_token();
        let access_hash = hmac_sha256_hex(&access_token, input.token_hash_key);
        let refresh_token = random_token();
        let refresh_hash = hmac_sha256_hex(&refresh_token, input.token_hash_key);

        sqlx::query(
            r#"
            INSERT INTO oauth_access_tokens (
                token_hash, github_user_id, client_id, resource, scopes, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(access_hash)
        .bind(github_user_id)
        .bind(&client_id)
        .bind(&resource)
        .bind(&scope)
        .bind(now() + input.access_token_ttl.as_secs() as i64)
        .execute(&mut *transaction)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO oauth_refresh_tokens (
                token_hash, github_user_id, client_id, resource, scopes, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(refresh_hash)
        .bind(github_user_id)
        .bind(client_id)
        .bind(resource)
        .bind(&scope)
        .bind(now() + input.refresh_token_ttl.as_secs() as i64)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        Ok(IssuedAccessToken {
            access_token,
            token_type: "Bearer",
            expires_in: input.access_token_ttl.as_secs(),
            scope,
            refresh_token,
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

    pub async fn revoke_refresh_token(
        &self,
        token: &str,
        hash_key: &str,
    ) -> Result<(), StoreError> {
        let token_hash = hmac_sha256_hex(token, hash_key);
        sqlx::query("UPDATE oauth_refresh_tokens SET revoked_at = ? WHERE token_hash = ?")
            .bind(now())
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
