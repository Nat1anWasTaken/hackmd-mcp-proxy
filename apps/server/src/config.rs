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
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
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
        let connect_timeout = duration_seconds("UPSTREAM_CONNECT_TIMEOUT_SECONDS", 10)?;
        let request_timeout = duration_seconds("UPSTREAM_REQUEST_TIMEOUT_SECONDS", 30)?;

        if environment == Environment::Production && !public_base_url.starts_with("https://") {
            bail!("PUBLIC_BASE_URL must use https in production");
        }

        Ok(Self {
            bind_addr,
            public_base_url: public_base_url.trim_end_matches('/').to_owned(),
            environment,
            log_format,
            upstream_mcp_url: upstream_mcp_url.trim_end_matches('/').to_owned(),
            local_hackmd_api_token,
            connect_timeout,
            request_timeout,
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
