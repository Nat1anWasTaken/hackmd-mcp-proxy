use std::sync::Arc;

use anyhow::Context;

use crate::{config::Config, store::Store};

#[derive(Clone, Debug)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

#[derive(Debug)]
struct AppStateInner {
    config: Config,
    http_client: reqwest::Client,
    store: Store,
}

impl AppState {
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        let http_client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .user_agent("hackmd-mcp-proxy/0.1.0")
            .build()
            .context("failed to build HTTP client")?;
        let store = Store::connect(&config.database_url).await?;

        Ok(Self {
            inner: Arc::new(AppStateInner {
                config,
                http_client,
                store,
            }),
        })
    }

    pub async fn new_for_test(config: Config, store: Store) -> anyhow::Result<Self> {
        let http_client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .user_agent("hackmd-mcp-proxy/0.1.0")
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            inner: Arc::new(AppStateInner {
                config,
                http_client,
                store,
            }),
        })
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.inner.http_client
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }
}
