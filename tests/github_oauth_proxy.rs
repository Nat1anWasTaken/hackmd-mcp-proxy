use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hackmd_mcp_proxy::{
    build_router,
    config::{Config, Environment, LogFormat},
    state::AppState,
    store::Store,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, sync::Mutex};
use tower::ServiceExt;
use url::{Url, form_urlencoded};
use uuid::Uuid;

#[tokio::test]
async fn github_user_stores_hackmd_key_once_and_mcp_uses_it() -> anyhow::Result<()> {
    let upstream_requests = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let upstream_url = spawn_mock_upstream(upstream_requests.clone()).await?;
    let config = test_config(upstream_url.clone());
    let store = Store::connect(&format!(
        "sqlite://target/test-{}.db",
        Uuid::new_v4().simple()
    ))
    .await?;
    let app = build_router(AppState::new_for_test(config, store).await?);
    let redirect_uri = "https://chatgpt.com/connector/oauth/test";
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    let client_id = register_client(&app, redirect_uri).await?;
    let authorize_path = authorize_path(&client_id, redirect_uri, &challenge);

    let authorize_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&authorize_path)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(authorize_response.status(), StatusCode::SEE_OTHER);
    let login_location = location(&authorize_response)?;
    assert!(login_location.starts_with("/auth/github/start?"));

    let github_start_response = app
        .clone()
        .oneshot(Request::builder().uri(login_location).body(Body::empty())?)
        .await?;
    assert_eq!(github_start_response.status(), StatusCode::SEE_OTHER);
    let github_location = location(&github_start_response)?;
    let github_url = Url::parse(github_location)?;
    let github_state = github_url
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then_some(value.into_owned()))
        .ok_or_else(|| anyhow::anyhow!("missing GitHub state"))?;

    let callback_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/github/callback?code=github-code&state={github_state}"
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(callback_response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&callback_response)?, authorize_path);
    let session_cookie = callback_response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .ok_or_else(|| anyhow::anyhow!("missing session cookie"))?
        .to_owned();

    let needs_token_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&authorize_path)
                .header(header::COOKIE, &session_cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(needs_token_response.status(), StatusCode::OK);
    let needs_token_body = body_string(needs_token_response).await?;
    assert!(needs_token_body.contains("Connect HackMD"));

    let token_form = form_urlencoded::Serializer::new(String::new())
        .append_pair("hackmd_api_token", "hackmd-secret")
        .append_pair("return_to", &authorize_path)
        .finish();
    let save_token_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/hackmd/token")
                .header(header::COOKIE, &session_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(token_form))?,
        )
        .await?;
    assert_eq!(save_token_response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&save_token_response)?, authorize_path);

    let code_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&authorize_path)
                .header(header::COOKIE, &session_cookie)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(code_response.status(), StatusCode::SEE_OTHER);
    let chatgpt_redirect = Url::parse(location(&code_response)?)?;
    let code = chatgpt_redirect
        .query_pairs()
        .find_map(|(key, value)| (key == "code").then_some(value.into_owned()))
        .ok_or_else(|| anyhow::anyhow!("missing code"))?;
    let state = chatgpt_redirect
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then_some(value.into_owned()));
    assert_eq!(state.as_deref(), Some("opaque-state"));

    let token_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(token_form_body(
                    &client_id,
                    redirect_uri,
                    &code,
                    verifier,
                )))?,
        )
        .await?;
    assert_eq!(token_response.status(), StatusCode::OK);
    let token = response_json(token_response).await?;
    let access_token = token["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing access token"))?;

    let mcp_response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp?cursor=next")
                .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
                .header(header::ACCEPT, "application/json, text/event-stream")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, "must-not-forward=1")
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
                ))?,
        )
        .await?;
    assert_eq!(mcp_response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        mcp_response.headers().get("mcp-session-id"),
        Some(&"upstream-session".parse()?)
    );

    let requests = upstream_requests.lock().await;
    let proxied = requests
        .iter()
        .find(|request| request.query.as_deref() == Some("cursor=next"))
        .ok_or_else(|| anyhow::anyhow!("missing proxied MCP request"))?;
    assert_eq!(
        proxied.headers.get(header::AUTHORIZATION),
        Some(&"Bearer hackmd-secret".parse()?)
    );
    assert!(!proxied.headers.contains_key(header::COOKIE));

    Ok(())
}

#[tokio::test]
async fn missing_mcp_bearer_returns_resource_challenge() -> anyhow::Result<()> {
    let upstream_url = spawn_mock_upstream(Arc::new(Mutex::new(Vec::new()))).await?;
    let config = test_config(upstream_url);
    let store = Store::connect(&format!(
        "sqlite://target/test-{}.db",
        Uuid::new_v4().simple()
    ))
    .await?;
    let app = build_router(AppState::new_for_test(config, store).await?);

    let response = app
        .oneshot(Request::builder().uri("/mcp").body(Body::empty())?)
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers().get(header::WWW_AUTHENTICATE),
        Some(
            &r#"Bearer resource_metadata="http://127.0.0.1:3000/.well-known/oauth-protected-resource""#
                .parse()?
        )
    );
    Ok(())
}

#[derive(Debug)]
struct RecordedRequest {
    query: Option<String>,
    headers: HeaderMap,
}

async fn spawn_mock_upstream(requests: Arc<Mutex<Vec<RecordedRequest>>>) -> anyhow::Result<String> {
    let app = Router::new()
        .route("/login/oauth/access_token", post(github_token))
        .route("/user", get(github_user))
        .route("/mcp", any(mock_hackmd))
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

async fn github_token() -> Json<Value> {
    Json(serde_json::json!({ "access_token": "github-token" }))
}

async fn github_user() -> Json<Value> {
    Json(serde_json::json!({ "id": 42, "login": "octocat" }))
}

async fn mock_hackmd(
    State(requests): State<Arc<Mutex<Vec<RecordedRequest>>>>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let _ = body;
    requests.lock().await.push(RecordedRequest {
        query: uri.query().map(ToOwned::to_owned),
        headers,
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

async fn register_client(app: &Router, redirect_uri: &str) -> anyhow::Result<String> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "client_name": "ChatGPT",
                        "redirect_uris": [redirect_uri],
                        "token_endpoint_auth_method": "none"
                    })
                    .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let registered = response_json(response).await?;
    registered["client_id"]
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("missing client_id"))
}

fn authorize_path(client_id: &str, redirect_uri: &str, challenge: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", "opaque-state")
        .append_pair("scope", "hackmd")
        .append_pair("resource", "http://127.0.0.1:3000/mcp")
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .finish();
    format!("/authorize?{query}")
}

fn token_form_body(client_id: &str, redirect_uri: &str, code: &str, verifier: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("code", code)
        .append_pair("code_verifier", verifier)
        .finish()
}

fn location(response: &Response) -> anyhow::Result<&str> {
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing location header"))
}

async fn response_json(response: Response) -> anyhow::Result<Value> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn body_string(response: Response) -> anyhow::Result<String> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(String::from_utf8(bytes.to_vec())?)
}

fn test_config(upstream_url: String) -> Config {
    Config {
        bind_addr: "127.0.0.1:0".to_owned(),
        public_base_url: "http://127.0.0.1:3000".to_owned(),
        database_url: "sqlite::memory:".to_owned(),
        environment: Environment::Test,
        log_format: LogFormat::Pretty,
        upstream_mcp_url: format!("{upstream_url}/mcp"),
        github_client_id: "github-client".to_owned(),
        github_client_secret: "github-secret".to_owned(),
        github_authorize_url: format!("{upstream_url}/login/oauth/authorize"),
        github_token_url: format!("{upstream_url}/login/oauth/access_token"),
        github_user_url: format!("{upstream_url}/user"),
        token_encryption_key: [3_u8; 32],
        session_hash_key: "test-session-key".to_owned(),
        oauth_access_token_hash_key: "test-access-key".to_owned(),
        oauth_authorization_code_hash_key: "test-code-key".to_owned(),
        github_state_hash_key: "test-github-state-key".to_owned(),
        connect_timeout: Duration::from_secs(1),
        request_timeout: Duration::from_secs(5),
        access_token_ttl: Duration::from_secs(3600),
        authorization_code_ttl: Duration::from_secs(300),
        web_session_ttl: Duration::from_secs(3600),
        github_state_ttl: Duration::from_secs(300),
        secure_cookies: false,
    }
}
