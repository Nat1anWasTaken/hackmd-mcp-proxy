use sqlx::Row;

use super::{CredentialRecord, Store, StoreError, now};

impl Store {
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
}
