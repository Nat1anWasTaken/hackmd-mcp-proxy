use std::sync::Arc;

use crate::config::Config;

#[derive(Clone, Debug)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

#[derive(Debug)]
struct AppStateInner {
    config: Config,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self {
            inner: Arc::new(AppStateInner { config }),
        }
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }
}
