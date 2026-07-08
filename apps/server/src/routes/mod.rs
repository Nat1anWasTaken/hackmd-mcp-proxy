mod health;
mod mcp;

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().merge(health::router()).merge(mcp::router())
}
