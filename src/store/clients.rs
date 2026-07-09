use sqlx::Row;
use uuid::Uuid;

use super::{Store, StoreError, now};
use crate::oauth::OAuthClient;

impl Store {
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
}
