use serde::Deserialize;

use crate::config::Config;

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error("GitHub token exchange failed")]
    TokenExchange(#[source] reqwest::Error),
    #[error("GitHub user fetch failed")]
    UserFetch(#[source] reqwest::Error),
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubUserResponse {
    pub id: i64,
    pub login: String,
}

pub async fn exchange_code(
    client: &reqwest::Client,
    config: &Config,
    code: &str,
) -> Result<String, GitHubError> {
    let response = client
        .post(&config.github_token_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&[
            ("client_id", config.github_client_id.as_str()),
            ("client_secret", config.github_client_secret.as_str()),
            ("code", code),
            ("redirect_uri", config.github_callback_url().as_str()),
        ])
        .send()
        .await
        .map_err(GitHubError::TokenExchange)?
        .error_for_status()
        .map_err(GitHubError::TokenExchange)?
        .json::<TokenResponse>()
        .await
        .map_err(GitHubError::TokenExchange)?;
    Ok(response.access_token)
}

pub async fn fetch_user(
    client: &reqwest::Client,
    config: &Config,
    access_token: &str,
) -> Result<GitHubUserResponse, GitHubError> {
    client
        .get(&config.github_user_url)
        .bearer_auth(access_token)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(GitHubError::UserFetch)?
        .error_for_status()
        .map_err(GitHubError::UserFetch)?
        .json::<GitHubUserResponse>()
        .await
        .map_err(GitHubError::UserFetch)
}
