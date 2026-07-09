use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{Path, State},
    http::{HeaderMap, Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hackmd_mcp_proxy::{
    build_router,
    config::{Config, Environment, LogFormat},
    hackmd::{JsonRpcRequest, handle_mcp_request},
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
async fn github_user_stores_hackmd_key_once_and_mcp_uses_local_tools() -> anyhow::Result<()> {
    let hackmd_requests = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let upstream_url = spawn_mock_upstream(hackmd_requests.clone()).await?;
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
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
                .header(header::ACCEPT, "application/json, text/event-stream")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, "must-not-forward=1")
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
                ))?,
        )
        .await?;
    assert_eq!(mcp_response.status(), StatusCode::OK);
    let tools = response_json(mcp_response).await?;
    let tools_json = tools.to_string();
    assert!(tools_json.contains("hackmd_edit_note"));
    assert!(tools_json.contains("Prefer this over hackmd_update_note"));
    assert!(tools_json.contains("Do not use for normal content edits"));
    let listed_tools = &tools["result"]["tools"]
        .as_array()
        .expect("tools list missing")[..];
    let annotations = |name: &str| {
        listed_tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("tool definition missing: {name}"))["annotations"]
            .as_object()
            .expect("annotations missing")
    };

    let expect_flags = |name: &str, read_only_hint: bool, destructive_hint: bool| {
        let annotations = annotations(name);
        assert_eq!(
            annotations["readOnlyHint"].as_bool(),
            Some(read_only_hint),
            "{name} readOnlyHint mismatch"
        );
        assert_eq!(
            annotations["destructiveHint"].as_bool(),
            Some(destructive_hint),
            "{name} destructiveHint mismatch"
        );
    };

    expect_flags("hackmd_list_notes", true, false);
    expect_flags("hackmd_get_note", true, false);
    expect_flags("hackmd_create_note", false, false);
    expect_flags("hackmd_edit_note", false, false);
    expect_flags("hackmd_update_note", false, false);
    expect_flags("hackmd_delete_note", false, true);
    expect_flags("hackmd_list_folders", true, false);
    expect_flags("hackmd_create_folder", false, false);
    expect_flags("hackmd_update_folder", false, false);
    expect_flags("hackmd_delete_folder", false, true);

    let list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "tools/call",
                        "params": {
                            "name": "hackmd_list_notes",
                            "arguments": {
                                "query": "title",
                                "tags": ["docs"],
                                "folder_id": "folder-1",
                                "limit": 10
                            }
                        }
                    })
                    .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(list_response.status(), StatusCode::OK);
    let list = response_json(list_response).await?;
    assert_eq!(list["error"], Value::Null);
    assert_eq!(list["result"]["structuredContent"]["total"], 1);
    assert_eq!(
        list["result"]["structuredContent"]["notes"][0]["patch_path"],
        "notes/note-1.md"
    );

    let edit_response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "tools/call",
                        "params": {
                            "name": "hackmd_edit_note",
                            "arguments": {
                                "note_id": "note-1",
                                "patch": "*** Begin Patch\n*** Update File: notes/note-1.md\n@@\n # Title\n-old text\n+new text\n*** End Patch"
                            }
                        }
                    })
                    .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(edit_response.status(), StatusCode::OK);
    let edit = response_json(edit_response).await?;
    assert_eq!(edit["error"], Value::Null);
    assert_eq!(
        edit["result"]["structuredContent"]["content"],
        "# Title\nnew text\nrepeated\nrepeated\n"
    );

    let requests = hackmd_requests.lock().await;
    let verify = requests
        .iter()
        .find(|request| request.path == "/me")
        .ok_or_else(|| anyhow::anyhow!("missing HackMD token verification request"))?;
    assert_eq!(
        verify.headers.get(header::AUTHORIZATION),
        Some(&"Bearer hackmd-secret".parse()?)
    );

    let patch = requests
        .iter()
        .find(|request| request.method == Method::PATCH && request.path == "/notes/note-1")
        .ok_or_else(|| anyhow::anyhow!("missing HackMD note patch request"))?;
    assert_eq!(
        patch.body["content"],
        "# Title\nnew text\nrepeated\nrepeated\n"
    );

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

#[tokio::test]
async fn edit_note_patch_errors_return_json_rpc_errors_without_upstream_patch() -> anyhow::Result<()>
{
    let hackmd_requests = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let upstream_url = spawn_mock_upstream(hackmd_requests.clone()).await?;

    let cases = [
        (
            "missing begin",
            "*** Update File: notes/note-1.md\n@@\n-old text\n+new text\n*** End Patch",
            "patch must start with *** Begin Patch",
        ),
        (
            "wrong target",
            "*** Begin Patch\n*** Update File: notes/other.md\n@@\n-old text\n+new text\n*** End Patch",
            "patch targets notes/other.md, expected notes/note-1.md",
        ),
        (
            "missing context",
            "*** Begin Patch\n*** Update File: notes/note-1.md\n@@\n-missing text\n+new text\n*** End Patch",
            "patch hunk context was not found",
        ),
        (
            "ambiguous context",
            "*** Begin Patch\n*** Update File: notes/note-1.md\n@@\n-repeated\n+changed\n*** End Patch",
            "patch hunk context matched multiple locations",
        ),
    ];

    for (label, patch, expected_message) in cases {
        hackmd_requests.lock().await.clear();

        let response = call_mcp_tool(
            &upstream_url,
            "hackmd_edit_note",
            serde_json::json!({
                "note_id": "note-1",
                "patch": patch
            }),
        )
        .await?;

        assert_eq!(response["result"], Value::Null, "{label}");
        assert_eq!(response["error"]["code"], -32000, "{label}");
        assert_eq!(response["error"]["message"], expected_message, "{label}");

        let requests = hackmd_requests.lock().await;
        assert!(
            requests
                .iter()
                .any(|request| request.method == Method::GET && request.path == "/notes/note-1"),
            "{label}"
        );
        assert!(
            requests
                .iter()
                .all(|request| request.method != Method::PATCH),
            "{label}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn edit_note_routes_team_workspace_and_uses_team_patch_path() -> anyhow::Result<()> {
    let hackmd_requests = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let upstream_url = spawn_mock_upstream(hackmd_requests.clone()).await?;

    let response = call_mcp_tool(
        &upstream_url,
        "hackmd_edit_note",
        serde_json::json!({
            "workspace": { "kind": "team", "team_path": "core-team" },
            "note_id": "note-1",
            "patch": "*** Begin Patch\n*** Update File: teams/core-team/notes/note-1.md\n@@\n # Title\n-old text\n+team text\n*** End Patch"
        }),
    )
    .await?;

    assert_eq!(response["error"], Value::Null);
    assert_eq!(
        response["result"]["structuredContent"]["patch_path"],
        "teams/core-team/notes/note-1.md"
    );
    assert_eq!(response["result"]["structuredContent"]["changed"], true);
    assert_eq!(
        response["result"]["structuredContent"]["content"],
        "# Title\nteam text\nrepeated\nrepeated\n"
    );

    let requests = hackmd_requests.lock().await;
    assert!(
        requests.iter().any(|request| request.method == Method::GET
            && request.path == "/teams/core-team/notes/note-1")
    );
    let patch = requests
        .iter()
        .find(|request| {
            request.method == Method::PATCH && request.path == "/teams/core-team/notes/note-1"
        })
        .ok_or_else(|| anyhow::anyhow!("missing team HackMD note patch request"))?;
    assert_eq!(
        patch.body["content"],
        "# Title\nteam text\nrepeated\nrepeated\n"
    );

    Ok(())
}

#[tokio::test]
async fn edit_note_noop_returns_unchanged_without_upstream_patch() -> anyhow::Result<()> {
    let hackmd_requests = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let upstream_url = spawn_mock_upstream(hackmd_requests.clone()).await?;

    let response = call_mcp_tool(
        &upstream_url,
        "hackmd_edit_note",
        serde_json::json!({
            "note_id": "note-1",
            "patch": "*** Begin Patch\n*** Update File: notes/note-1.md\n@@\n # Title\n old text\n*** End Patch"
        }),
    )
    .await?;

    assert_eq!(response["error"], Value::Null);
    assert_eq!(response["result"]["structuredContent"]["changed"], false);
    assert_eq!(
        response["result"]["structuredContent"]["content"],
        "# Title\nold text\nrepeated\nrepeated\n"
    );

    let requests = hackmd_requests.lock().await;
    assert!(
        requests
            .iter()
            .any(|request| request.method == Method::GET && request.path == "/notes/note-1")
    );
    assert!(
        requests
            .iter()
            .all(|request| request.method != Method::PATCH)
    );

    Ok(())
}

#[tokio::test]
async fn update_note_patches_metadata_and_rejects_empty_fields() -> anyhow::Result<()> {
    let hackmd_requests = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let upstream_url = spawn_mock_upstream(hackmd_requests.clone()).await?;

    let update = call_mcp_tool(
        &upstream_url,
        "hackmd_update_note",
        serde_json::json!({
            "note_id": "note-1",
            "fields": {
                "title": "Updated",
                "tags": ["docs", "release"]
            }
        }),
    )
    .await?;

    assert_eq!(update["error"], Value::Null);

    let empty = call_mcp_tool(
        &upstream_url,
        "hackmd_update_note",
        serde_json::json!({
            "note_id": "note-1",
            "fields": {}
        }),
    )
    .await?;

    assert_eq!(empty["result"], Value::Null);
    assert_eq!(empty["error"]["code"], -32000);
    assert_eq!(
        empty["error"]["message"],
        "invalid HackMD API request: fields must contain at least one note property"
    );

    let requests = hackmd_requests.lock().await;
    let patch_requests = requests
        .iter()
        .filter(|request| request.method == Method::PATCH && request.path == "/notes/note-1")
        .collect::<Vec<_>>();
    assert_eq!(patch_requests.len(), 1);
    assert_eq!(patch_requests[0].body["title"], "Updated");
    assert_eq!(
        patch_requests[0].body["tags"],
        serde_json::json!(["docs", "release"])
    );

    Ok(())
}

#[derive(Debug)]
struct RecordedRequest {
    method: Method,
    path: String,
    headers: HeaderMap,
    body: Value,
}

async fn spawn_mock_upstream(requests: Arc<Mutex<Vec<RecordedRequest>>>) -> anyhow::Result<String> {
    let app = Router::new()
        .route("/login/oauth/access_token", post(github_token))
        .route("/user", get(github_user))
        .route("/me", get(hackmd_me))
        .route("/notes", get(list_notes).post(create_note))
        .route("/notes/{note_id}", any(note_item))
        .route(
            "/teams/{team_path}/notes",
            get(list_notes).post(create_note),
        )
        .route("/teams/{team_path}/notes/{note_id}", any(team_note_item))
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
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    requests.lock().await.push(RecordedRequest {
        method,
        path: uri.path().to_owned(),
        headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });

    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn hackmd_me(
    State(requests): State<Arc<Mutex<Vec<RecordedRequest>>>>,
    uri: axum::http::Uri,
    method: Method,
    headers: HeaderMap,
) -> Response {
    requests.lock().await.push(RecordedRequest {
        method,
        path: uri.path().to_owned(),
        headers,
        body: Value::Null,
    });

    Json(serde_json::json!({
        "id": "user-1",
        "name": "Octocat",
        "userPath": "octocat",
        "photo": "",
        "email": null,
        "teams": [],
        "upgraded": false
    }))
    .into_response()
}

async fn list_notes(
    State(requests): State<Arc<Mutex<Vec<RecordedRequest>>>>,
    uri: axum::http::Uri,
    method: Method,
    headers: HeaderMap,
) -> Response {
    requests.lock().await.push(RecordedRequest {
        method,
        path: uri.path().to_owned(),
        headers,
        body: Value::Null,
    });

    Json(serde_json::json!([
        {
            "id": "note-1",
            "shortId": "short-1",
            "title": "Title",
            "description": "Description",
            "tags": ["docs"],
            "createdAt": 1,
            "lastChangedAt": 2,
            "folderPaths": [{ "id": "folder-1" }]
        }
    ]))
    .into_response()
}

async fn create_note(
    State(requests): State<Arc<Mutex<Vec<RecordedRequest>>>>,
    uri: axum::http::Uri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    mock_hackmd(State(requests), uri, method, headers, body).await
}

async fn note_item(
    State(requests): State<Arc<Mutex<Vec<RecordedRequest>>>>,
    Path(note_id): Path<String>,
    uri: axum::http::Uri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    requests.lock().await.push(RecordedRequest {
        method: method.clone(),
        path: uri.path().to_owned(),
        headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });

    match method {
        Method::GET => Json(serde_json::json!({
            "id": note_id,
            "title": "Title",
            "content": "# Title\nold text\nrepeated\nrepeated\n",
            "tags": ["docs"]
        }))
        .into_response(),
        Method::PATCH => Json(serde_json::json!({ "ok": true })).into_response(),
        Method::DELETE => StatusCode::NO_CONTENT.into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

async fn team_note_item(
    State(requests): State<Arc<Mutex<Vec<RecordedRequest>>>>,
    Path((_team_path, note_id)): Path<(String, String)>,
    uri: axum::http::Uri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    note_item(State(requests), Path(note_id), uri, method, headers, body).await
}

async fn call_mcp_tool(upstream_url: &str, name: &str, arguments: Value) -> anyhow::Result<Value> {
    let response = handle_mcp_request(
        &reqwest::Client::new(),
        upstream_url,
        "hackmd-secret",
        JsonRpcRequest {
            id: Some(serde_json::json!(1)),
            jsonrpc: Some("2.0".to_owned()),
            method: "tools/call".to_owned(),
            params: Some(serde_json::json!({
                "name": name,
                "arguments": arguments
            })),
        },
    )
    .await;

    Ok(serde_json::to_value(response)?)
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
        hackmd_api_url: upstream_url.clone(),
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
