use std::{env, time::Duration};

use anyhow::{bail, Context};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: String,
    pub public_base_url: String,
    pub environment: Environment,
    pub log_format: LogFormat,
    pub upstream_mcp_url: String,
    pub local_hackmd_api_token: Option<String>,
    pub oauth_access_token_hash_key: String,
    pub oauth_authorization_code_hash_key: String,
    pub oauth_auto_approve: bool,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub access_token_ttl: Duration,
    pub authorization_code_ttl: Duration,
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
        let bind_addr = env_var("BIND_ADDR", "127.0.0.1:3000");
        let public_base_url = env_var("PUBLIC_BASE_URL", "http://127.0.0.1:3000");
        let environment = Environment::parse(&env_var("APP_ENV", "development"))?;
        let log_format = LogFormat::parse(&env_var("LOG_FORMAT", "pretty"))?;
        let upstream_mcp_url = env_var("HACKMD_MCP_URL", "https://mcp.hackmd.io");
        let local_hackmd_api_token = optional_env_var("HACKMD_API_TOKEN");
        let oauth_access_token_hash_key =
            env_var("ACCESS_TOKEN_HASH_KEY", "dev-access-token-hash-key");
        let oauth_authorization_code_hash_key =
            env_var("AUTH_CODE_HASH_KEY", "dev-authorization-code-hash-key");
        let oauth_auto_approve =
            bool_env("OAUTH_AUTO_APPROVE", environment != Environment::Production)?;
        let connect_timeout = duration_seconds("UPSTREAM_CONNECT_TIMEOUT_SECONDS", 10)?;
        let request_timeout = duration_seconds("UPSTREAM_REQUEST_TIMEOUT_SECONDS", 30)?;
        let access_token_ttl = duration_seconds("ACCESS_TOKEN_TTL_SECONDS", 3600)?;
        let authorization_code_ttl = duration_seconds("AUTH_CODE_TTL_SECONDS", 300)?;

        if environment == Environment::Production && !public_base_url.starts_with("https://") {
            bail!("PUBLIC_BASE_URL must use https in production");
        }
        if environment == Environment::Production
            && (oauth_access_token_hash_key.starts_with("dev-")
                || oauth_authorization_code_hash_key.starts_with("dev-"))
        {
            bail!("ACCESS_TOKEN_HASH_KEY and AUTH_CODE_HASH_KEY must be configured in production");
        }

        Ok(Self {
            bind_addr,
            public_base_url: public_base_url.trim_end_matches('/').to_owned(),
            environment,
            log_format,
            upstream_mcp_url: upstream_mcp_url.trim_end_matches('/').to_owned(),
            local_hackmd_api_token,
            oauth_access_token_hash_key,
            oauth_authorization_code_hash_key,
            oauth_auto_approve,
            connect_timeout,
            request_timeout,
            access_token_ttl,
            authorization_code_ttl,
        })
    }

    pub fn resource_url(&self) -> String {
        format!("{}/mcp", self.public_base_url)
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
    let raw = optional_env_var(name);
    match raw.as_deref() {
        None => Ok(default),
        Some("true" | "1" | "yes") => Ok(true),
        Some("false" | "0" | "no") => Ok(false),
        Some(_) => bail!("{name} must be true or false"),
    }
}
