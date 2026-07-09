use std::time::Duration;

use sqlx::Row;

use super::{GitHubUser, Store, StoreError, now};

impl Store {
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
}
