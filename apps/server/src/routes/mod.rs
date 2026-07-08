mod health;
mod mcp;
mod oauth;

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(health::router())
        .merge(oauth::router())
        .merge(mcp::router())
}
