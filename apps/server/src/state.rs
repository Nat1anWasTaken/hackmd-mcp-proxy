use std::sync::Arc;

use crate::config::Config;

#[derive(Clone, Debug)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

#[derive(Debug)]
struct AppStateInner {
    config: Config,
    http_client: reqwest::Client,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let http_client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "failed to build configured http client; using defaults");
                reqwest::Client::new()
            });

        Self {
            inner: Arc::new(AppStateInner {
                config,
                http_client,
            }),
        }
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.inner.http_client
    }
}
