use std::time::Duration;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hackmd_mcp_server::{
    build_router,
    config::{Config, Environment, LogFormat},
    state::AppState,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use url::{form_urlencoded, Url};

#[tokio::test]
async fn oauth_metadata_exposes_chatgpt_compatible_endpoints() -> anyhow::Result<()> {
    let app = build_router(AppState::new(test_config()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/oauth-authorization-server")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await?;
    assert_eq!(
        body["authorization_endpoint"],
        "http://127.0.0.1:3000/authorize"
    );
    assert_eq!(body["token_endpoint"], "http://127.0.0.1:3000/token");
    assert_eq!(
        body["code_challenge_methods_supported"],
        serde_json::json!(["S256"])
    );
    Ok(())
}

#[tokio::test]
async fn dcr_authorization_code_and_pkce_token_exchange_work() -> anyhow::Result<()> {
    let app = build_router(AppState::new(test_config()));
    let redirect_uri = "https://chatgpt.com/connector/oauth/test";
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    let register_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
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
    assert_eq!(register_response.status(), StatusCode::OK);
    let registered = response_json(register_response).await?;
    let client_id = registered["client_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing client_id"))?;

    let authorize_query = form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", "opaque-state")
        .append_pair("scope", "hackmd.read hackmd.write")
        .append_pair("resource", "http://127.0.0.1:3000/mcp")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .finish();
    let authorize_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/authorize?{authorize_query}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(authorize_response.status(), StatusCode::SEE_OTHER);
    let redirect = authorize_response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing redirect location"))?;
    let redirect = Url::parse(redirect)?;
    assert_eq!(redirect.as_str().split('?').next(), Some(redirect_uri));
    let code = redirect
        .query_pairs()
        .find_map(|(key, value)| (key == "code").then_some(value.into_owned()))
        .ok_or_else(|| anyhow::anyhow!("missing code"))?;
    let state = redirect
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then_some(value.into_owned()));
    assert_eq!(state.as_deref(), Some("opaque-state"));

    let wrong_verifier_body = token_form(client_id, redirect_uri, &code, "wrong-verifier");
    let wrong_verifier_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(wrong_verifier_body))?,
        )
        .await?;
    assert_eq!(wrong_verifier_response.status(), StatusCode::BAD_REQUEST);

    let token_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(token_form(
                    client_id,
                    redirect_uri,
                    &code,
                    verifier,
                )))?,
        )
        .await?;
    assert_eq!(token_response.status(), StatusCode::OK);
    let token = response_json(token_response).await?;
    assert_eq!(token["token_type"], "Bearer");
    assert_eq!(token["scope"], "hackmd.read hackmd.write");
    assert!(token["access_token"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));

    Ok(())
}

#[tokio::test]
async fn mcp_without_bearer_token_returns_protected_resource_challenge() -> anyhow::Result<()> {
    let app = build_router(AppState::new(test_config()));

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

fn token_form(client_id: &str, redirect_uri: &str, code: &str, verifier: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("code", code)
        .append_pair("code_verifier", verifier)
        .finish()
}

async fn response_json(response: axum::response::Response) -> anyhow::Result<Value> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn test_config() -> Config {
    Config {
        bind_addr: "127.0.0.1:0".to_owned(),
        public_base_url: "http://127.0.0.1:3000".to_owned(),
        environment: Environment::Test,
        log_format: LogFormat::Pretty,
        upstream_mcp_url: "http://127.0.0.1:4000".to_owned(),
        local_hackmd_api_token: Some("hackmd-secret".to_owned()),
        oauth_access_token_hash_key: "test-access-key".to_owned(),
        oauth_authorization_code_hash_key: "test-code-key".to_owned(),
        oauth_auto_approve: true,
        connect_timeout: Duration::from_secs(1),
        request_timeout: Duration::from_secs(1),
        access_token_ttl: Duration::from_secs(3600),
        authorization_code_ttl: Duration::from_secs(300),
    }
}
