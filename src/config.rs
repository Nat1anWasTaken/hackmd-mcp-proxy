use std::{env, str::FromStr, time::Duration};

use anyhow::{Context, bail};
use base64::{Engine, engine::general_purpose::STANDARD};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: String,
    pub public_base_url: String,
    pub database_url: String,
    pub environment: Environment,
    pub log_format: LogFormat,
    pub hackmd_api_url: String,
    pub github_client_id: String,
    pub github_client_secret: String,
    pub github_authorize_url: String,
    pub github_token_url: String,
    pub github_user_url: String,
    pub token_encryption_key: [u8; 32],
    pub session_hash_key: String,
    pub oauth_access_token_hash_key: String,
    pub oauth_authorization_code_hash_key: String,
    pub github_state_hash_key: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub access_token_ttl: Duration,
    pub authorization_code_ttl: Duration,
    pub web_session_ttl: Duration,
    pub github_state_ttl: Duration,
    pub secure_cookies: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Environment {
    Development,
    Test,
    Production,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogFormat {
    Pretty,
    Json,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        let bind_addr = env_var("BIND_ADDR", "127.0.0.1:3000");
        let public_base_url = env_var("PUBLIC_BASE_URL", "http://127.0.0.1:3000");
        let database_url = env_var("DATABASE_URL", "sqlite://hackmd-mcp-proxy.db");
        let environment = Environment::parse(&env_var("APP_ENV", "development"))?;
        let log_format = LogFormat::parse(&env_var("LOG_FORMAT", "pretty"))?;
        let hackmd_api_url = env_var("HACKMD_API_URL", "https://api.hackmd.io/v1");
        let github_client_id = env_var("GITHUB_CLIENT_ID", "dev-github-client-id");
        let github_client_secret = env_var("GITHUB_CLIENT_SECRET", "dev-github-client-secret");
        let github_authorize_url = env_var(
            "GITHUB_AUTHORIZE_URL",
            "https://github.com/login/oauth/authorize",
        );
        let github_token_url = env_var(
            "GITHUB_TOKEN_URL",
            "https://github.com/login/oauth/access_token",
        );
        let github_user_url = env_var("GITHUB_USER_URL", "https://api.github.com/user");
        let token_encryption_key = encryption_key_from_env(environment)?;
        let session_hash_key = env_var("SESSION_HASH_KEY", "dev-session-hash-key");
        let oauth_access_token_hash_key =
            env_var("ACCESS_TOKEN_HASH_KEY", "dev-access-token-hash-key");
        let oauth_authorization_code_hash_key =
            env_var("AUTH_CODE_HASH_KEY", "dev-authorization-code-hash-key");
        let github_state_hash_key = env_var("GITHUB_STATE_HASH_KEY", "dev-github-state-hash-key");
        let connect_timeout = duration_seconds("UPSTREAM_CONNECT_TIMEOUT_SECONDS", 10)?;
        let request_timeout = duration_seconds("UPSTREAM_REQUEST_TIMEOUT_SECONDS", 30)?;
        let access_token_ttl = duration_seconds("ACCESS_TOKEN_TTL_SECONDS", 3600)?;
        let authorization_code_ttl = duration_seconds("AUTH_CODE_TTL_SECONDS", 300)?;
        let web_session_ttl = duration_seconds("WEB_SESSION_TTL_SECONDS", 60 * 60 * 24 * 30)?;
        let github_state_ttl = duration_seconds("GITHUB_STATE_TTL_SECONDS", 600)?;
        let secure_cookies = bool_env(
            "SECURE_COOKIES",
            environment == Environment::Production || public_base_url.starts_with("https://"),
        )?;

        if environment == Environment::Production && !public_base_url.starts_with("https://") {
            bail!("PUBLIC_BASE_URL must use https in production");
        }
        if environment == Environment::Production
            && (github_client_id.starts_with("dev-")
                || github_client_secret.starts_with("dev-")
                || session_hash_key.starts_with("dev-")
                || oauth_access_token_hash_key.starts_with("dev-")
                || oauth_authorization_code_hash_key.starts_with("dev-")
                || github_state_hash_key.starts_with("dev-"))
        {
            bail!("production secrets must be configured");
        }

        Ok(Self {
            bind_addr,
            public_base_url: public_base_url.trim_end_matches('/').to_owned(),
            database_url,
            environment,
            log_format,
            hackmd_api_url: hackmd_api_url.trim_end_matches('/').to_owned(),
            github_client_id,
            github_client_secret,
            github_authorize_url,
            github_token_url,
            github_user_url,
            token_encryption_key,
            session_hash_key,
            oauth_access_token_hash_key,
            oauth_authorization_code_hash_key,
            github_state_hash_key,
            connect_timeout,
            request_timeout,
            access_token_ttl,
            authorization_code_ttl,
            web_session_ttl,
            github_state_ttl,
            secure_cookies,
        })
    }

    pub fn resource_url(&self) -> String {
        format!("{}/mcp", self.public_base_url)
    }

    pub fn github_callback_url(&self) -> String {
        format!("{}/auth/github/callback", self.public_base_url)
    }
}

impl Environment {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "development" | "dev" => Ok(Self::Development),
            "test" => Ok(Self::Test),
            "production" | "prod" => Ok(Self::Production),
            _ => bail!("APP_ENV must be development, test, or production"),
        }
    }
}

impl LogFormat {
    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "pretty" => Ok(Self::Pretty),
            "json" => Ok(Self::Json),
            _ => bail!("LOG_FORMAT must be pretty or json"),
        }
    }
}

fn env_var(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn optional_env_var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn duration_seconds(name: &str, default: u64) -> anyhow::Result<Duration> {
    let raw = env_var(name, &default.to_string());
    let seconds = raw
        .parse::<u64>()
        .with_context(|| format!("{name} must be an integer number of seconds"))?;
    Ok(Duration::from_secs(seconds))
}

fn bool_env(name: &str, default: bool) -> anyhow::Result<bool> {
    match optional_env_var(name).as_deref() {
        None => Ok(default),
        Some("true" | "1" | "yes") => Ok(true),
        Some("false" | "0" | "no") => Ok(false),
        Some(_) => bail!("{name} must be true or false"),
    }
}

fn encryption_key_from_env(environment: Environment) -> anyhow::Result<[u8; 32]> {
    let Some(raw) = optional_env_var("TOKEN_ENCRYPTION_KEY") else {
        if environment == Environment::Production {
            bail!("TOKEN_ENCRYPTION_KEY must be configured in production");
        }
        return Ok([7_u8; 32]);
    };
    let bytes = STANDARD
        .decode(raw)
        .context("TOKEN_ENCRYPTION_KEY must be base64")?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("TOKEN_ENCRYPTION_KEY must decode to 32 bytes"))
}

impl FromStr for Environment {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}
