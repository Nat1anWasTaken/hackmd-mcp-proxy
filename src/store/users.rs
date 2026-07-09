use super::{GitHubUser, Store, StoreError, now};

impl Store {
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
}
