use axum::{routing::get, Json, Router};
use serde::Serialize;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/health", get(health))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use crate::{build_router, config::Config, state::AppState};

    #[tokio::test]
    async fn health_returns_ok() -> anyhow::Result<()> {
        let app = build_router(AppState::new(test_config()));
        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty())?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        assert_eq!(&body[..], br#"{"status":"ok"}"#);
        Ok(())
    }

    fn test_config() -> Config {
        Config {
            bind_addr: "127.0.0.1:0".to_owned(),
            public_base_url: "http://127.0.0.1:3000".to_owned(),
            environment: crate::config::Environment::Test,
            log_format: crate::config::LogFormat::Pretty,
            upstream_mcp_url: "http://127.0.0.1:4000".to_owned(),
            local_hackmd_api_token: None,
            connect_timeout: std::time::Duration::from_secs(1),
            request_timeout: std::time::Duration::from_secs(1),
        }
    }
}
