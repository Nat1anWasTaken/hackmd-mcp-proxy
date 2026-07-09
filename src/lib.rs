pub mod config;
mod crypto;
mod github;
pub mod hackmd;
mod http;
pub mod oauth;
pub mod observability;
mod patch;
pub mod state;
pub mod store;

use axum::Router;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(http::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
