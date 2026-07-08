use axum::{
    body::Body,
    extract::{OriginalUri, State},
    http::{HeaderMap, Method},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};

use crate::{hackmd::proxy, state::AppState};

pub fn router() -> Router<AppState> {
    Router::new().route("/mcp", post(mcp).get(mcp).delete(mcp))
}

async fn mcp(
    State(state): State<AppState>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(hackmd_api_token) = state.config().local_hackmd_api_token.as_deref() else {
        return proxy::ProxyError::MissingHackmdToken.into_response();
    };

    match proxy::proxy_mcp_request(
        state.http_client(),
        &state.config().upstream_mcp_url,
        hackmd_api_token,
        method,
        uri,
        headers,
        body,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

#[allow(dead_code)]
fn _route_type_check() {
    let _: Router<AppState> = Router::new().route("/mcp", get(mcp).post(mcp).delete(mcp));
}
