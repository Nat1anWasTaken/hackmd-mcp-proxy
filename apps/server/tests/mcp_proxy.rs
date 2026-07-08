use std::{sync::Arc, time::Duration};

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::State,
    http::{header, HeaderMap, Method, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use hackmd_mcp_server::{
    build_router,
    config::{Config, Environment, LogFormat},
    state::AppState,
};
use tokio::{net::TcpListener, sync::Mutex};
use tower::ServiceExt;

#[tokio::test]
async fn mcp_proxy_replaces_authorization_and_filters_headers() -> anyhow::Result<()> {
    let upstream_requests = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let upstream_url = spawn_upstream(upstream_requests.clone()).await?;
    let app = build_router(AppState::new(test_config(upstream_url)));
    let request_body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp?cursor=next")
                .header(header::AUTHORIZATION, "Bearer wrapper-token")
                .header(header::ACCEPT, "application/json, text/event-stream")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, "session=secret")
                .header("MCP-Protocol-Version", "2025-06-18")
                .header("MCP-Session-Id", "client-session")
                .header("X-Forwarded-For", "127.0.0.1")
                .body(Body::from(request_body))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        response.headers().get("mcp-session-id"),
        Some(&"upstream-session".parse()?)
    );
    assert!(!response.headers().contains_key(header::SET_COOKIE));

    let response_body = to_bytes(response.into_body(), usize::MAX).await?;
    assert_eq!(&response_body[..], br#"{"ok":true}"#);

    let requests = upstream_requests.lock().await;
    assert_eq!(requests.len(), 1);
    let recorded = &requests[0];
    assert_eq!(recorded.method, Method::POST);
    assert_eq!(recorded.query.as_deref(), Some("cursor=next"));
    assert_eq!(
        recorded.headers.get(header::AUTHORIZATION),
        Some(&"Bearer hackmd-secret".parse()?)
    );
    assert_eq!(
        recorded.headers.get(header::ACCEPT),
        Some(&"application/json, text/event-stream".parse()?)
    );
    assert_eq!(
        recorded.headers.get("mcp-protocol-version"),
        Some(&"2025-06-18".parse()?)
    );
    assert_eq!(
        recorded.headers.get("mcp-session-id"),
        Some(&"client-session".parse()?)
    );
    assert!(!recorded.headers.contains_key(header::COOKIE));
    assert!(!recorded.headers.contains_key("x-forwarded-for"));
    assert_eq!(&recorded.body[..], request_body.as_bytes());

    Ok(())
}

#[tokio::test]
async fn mcp_proxy_requires_local_token_during_poc_stage() -> anyhow::Result<()> {
    let upstream_url = spawn_upstream(Arc::new(Mutex::new(Vec::new()))).await?;
    let mut config = test_config(upstream_url);
    config.local_hackmd_api_token = None;
    let app = build_router(AppState::new(config));

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/mcp")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    Ok(())
}

#[derive(Debug)]
struct RecordedRequest {
    method: Method,
    query: Option<String>,
    headers: HeaderMap,
    body: Bytes,
}

async fn spawn_upstream(requests: Arc<Mutex<Vec<RecordedRequest>>>) -> anyhow::Result<String> {
    let app = Router::new()
        .route("/", any(record_upstream))
        .with_state(requests);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::error!(%error, "mock upstream failed");
        }
    });
    Ok(format!("http://{addr}"))
}

async fn record_upstream(
    State(requests): State<Arc<Mutex<Vec<RecordedRequest>>>>,
    method: Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    requests.lock().await.push(RecordedRequest {
        method,
        query: uri.query().map(ToOwned::to_owned),
        headers,
        body,
    });

    (
        StatusCode::ACCEPTED,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::SET_COOKIE, "upstream=secret"),
            (
                header::HeaderName::from_static("mcp-session-id"),
                "upstream-session",
            ),
        ],
        r#"{"ok":true}"#,
    )
        .into_response()
}

fn test_config(upstream_mcp_url: String) -> Config {
    Config {
        bind_addr: "127.0.0.1:0".to_owned(),
        public_base_url: "http://127.0.0.1:3000".to_owned(),
        environment: Environment::Test,
        log_format: LogFormat::Pretty,
        upstream_mcp_url,
        local_hackmd_api_token: Some("hackmd-secret".to_owned()),
        connect_timeout: Duration::from_secs(1),
        request_timeout: Duration::from_secs(5),
    }
}
