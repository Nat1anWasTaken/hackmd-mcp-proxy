pub mod config;
pub mod crypto;
pub mod github;
pub mod hackmd;
pub mod oauth;
pub mod observability;
pub mod patch;
pub mod routes;
pub mod state;
pub mod store;

use axum::Router;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(routes::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
