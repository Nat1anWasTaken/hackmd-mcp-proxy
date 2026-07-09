use super::Store;

impl Store {
    pub(super) async fn init_schema(&self) -> anyhow::Result<()> {
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
}
