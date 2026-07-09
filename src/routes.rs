use axum::{
    Form, Json, Router,
    extract::{OriginalUri, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use url::{Url, form_urlencoded};

use crate::{
    crypto::{decrypt_secret, encrypt_secret, fingerprint, hmac_sha256_hex, random_token},
    github, hackmd,
    oauth::{
        AuthorizationServerMetadata, ClientRegistrationRequest, ProtectedResourceMetadata,
        ScopeSet, localhost_redirects_allowed,
    },
    state::AppState,
    store::{AuthorizeInput, ExchangeCodeInput, GitHubUser, StoreError},
};

const SESSION_COOKIE: &str = "hmcp_session";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server_metadata),
        )
        .route("/register", post(register_client))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/revoke", post(revoke))
        .route("/auth/github/start", get(github_start))
        .route("/auth/github/callback", get(github_callback))
        .route("/hackmd/token", post(save_hackmd_token))
        .route("/settings", get(settings))
        .route("/settings/disconnect", post(disconnect))
        .route("/mcp", post(mcp).get(mcp).delete(mcp))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn protected_resource_metadata(
    State(state): State<AppState>,
) -> Json<ProtectedResourceMetadata> {
    Json(ProtectedResourceMetadata::from_config(state.config()))
}

async fn authorization_server_metadata(
    State(state): State<AppState>,
) -> Json<AuthorizationServerMetadata> {
    Json(AuthorizationServerMetadata::from_config(state.config()))
}

async fn register_client(
    State(state): State<AppState>,
    Json(request): Json<ClientRegistrationRequest>,
) -> Result<Json<crate::oauth::ClientRegistrationResponse>, AppError> {
    let request = request.validate(localhost_redirects_allowed(state.config().environment))?;
    let client = state
        .store()
        .register_client(request.client_name, request.redirect_uris)
        .await?;
    Ok(Json(client.response()))
}

async fn authorize(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Query(query): Query<AuthorizeQuery>,
) -> Result<Response, AppError> {
    validate_authorize_query(&state, &query).await?;

    let return_to = uri.to_string();
    let Some(user) = current_user(&state, &headers).await? else {
        return Ok(Redirect::to(&github_start_url(&return_to)).into_response());
    };

    if state
        .store()
        .get_hackmd_credential(user.github_id)
        .await?
        .is_none()
    {
        return Ok(Html(render_token_form(&user, &return_to, None)).into_response());
    }

    issue_code_redirect(&state, query, user.github_id).await
}

async fn token(
    State(state): State<AppState>,
    Form(request): Form<TokenRequest>,
) -> Result<Json<TokenResponse>, AppError> {
    if request.grant_type != "authorization_code" {
        return Err(AppError::BadRequest(
            "grant_type must be authorization_code",
        ));
    }
    let issued = state
        .store()
        .exchange_code(ExchangeCodeInput {
            code: &request.code,
            client_id: &request.client_id,
            redirect_uri: &request.redirect_uri,
            code_verifier: &request.code_verifier,
            code_hash_key: &state.config().oauth_authorization_code_hash_key,
            access_token_hash_key: &state.config().oauth_access_token_hash_key,
            access_token_ttl: state.config().access_token_ttl,
        })
        .await?;

    Ok(Json(TokenResponse {
        access_token: issued.access_token,
        token_type: issued.token_type,
        expires_in: issued.expires_in,
        scope: issued.scope,
    }))
}

async fn revoke(State(state): State<AppState>, Form(request): Form<RevokeRequest>) -> StatusCode {
    if let Err(error) = state
        .store()
        .revoke_access_token(&request.token, &state.config().oauth_access_token_hash_key)
        .await
    {
        tracing::warn!(%error, "failed to revoke access token");
    }
    StatusCode::OK
}

async fn github_start(
    State(state): State<AppState>,
    Query(query): Query<GitHubStartQuery>,
) -> Result<Redirect, AppError> {
    let return_to = query.return_to.unwrap_or_else(|| "/settings".to_owned());
    if !return_to.starts_with('/') || return_to.starts_with("//") {
        return Err(AppError::BadRequest("invalid return_to"));
    }

    let state_token = random_token();
    let state_hash = hmac_sha256_hex(&state_token, &state.config().github_state_hash_key);
    state
        .store()
        .create_github_state(&state_hash, &return_to, state.config().github_state_ttl)
        .await?;

    let mut url = Url::parse(&state.config().github_authorize_url)?;
    url.query_pairs_mut()
        .append_pair("client_id", &state.config().github_client_id)
        .append_pair("redirect_uri", &state.config().github_callback_url())
        .append_pair("scope", "read:user")
        .append_pair("state", &state_token);
    Ok(Redirect::to(url.as_str()))
}

async fn github_callback(
    State(state): State<AppState>,
    Query(query): Query<GitHubCallbackQuery>,
) -> Result<Response, AppError> {
    let Some(code) = query.code else {
        return Err(AppError::BadRequest("missing GitHub code"));
    };
    let Some(state_token) = query.state else {
        return Err(AppError::BadRequest("missing GitHub state"));
    };
    let state_hash = hmac_sha256_hex(&state_token, &state.config().github_state_hash_key);
    let Some(return_to) = state.store().consume_github_state(&state_hash).await? else {
        return Err(AppError::BadRequest("invalid GitHub state"));
    };

    let github_token = github::exchange_code(state.http_client(), state.config(), &code).await?;
    let github_user =
        github::fetch_user(state.http_client(), state.config(), &github_token).await?;
    let user = state
        .store()
        .upsert_github_user(github_user.id, &github_user.login)
        .await?;

    let session_token = random_token();
    let session_hash = hmac_sha256_hex(&session_token, &state.config().session_hash_key);
    state
        .store()
        .create_web_session(
            &session_hash,
            user.github_id,
            state.config().web_session_ttl,
        )
        .await?;

    let mut response = Redirect::to(&return_to).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        session_cookie(&session_token, state.config().secure_cookies)?,
    );
    Ok(response)
}

async fn save_hackmd_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(request): Form<HackMdTokenForm>,
) -> Result<Response, AppError> {
    let Some(user) = current_user(&state, &headers).await? else {
        return Ok(Redirect::to(&github_start_url(&request.return_to)).into_response());
    };

    let token = request.hackmd_api_token.trim();
    if token.is_empty() {
        return Ok(Html(render_token_form(
            &user,
            &request.return_to,
            Some("Token is required."),
        ))
        .into_response());
    }

    if let Err(error) =
        hackmd::verify_token(state.http_client(), &state.config().hackmd_api_url, token).await
    {
        tracing::warn!(github_user_id = user.github_id, %error, "HackMD token verification failed");
        return Ok(Html(render_token_form(
            &user,
            &request.return_to,
            Some("The HackMD API token could not be verified."),
        ))
        .into_response());
    }

    let (nonce, ciphertext) = encrypt_secret(token, &state.config().token_encryption_key)?;
    let token_fingerprint = fingerprint(token, &state.config().session_hash_key);
    state
        .store()
        .store_hackmd_credential(user.github_id, nonce, ciphertext, &token_fingerprint)
        .await?;
    Ok(Redirect::to(&request.return_to).into_response())
}

async fn settings(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let Some(user) = current_user(&state, &headers).await? else {
        return Ok(Redirect::to(&github_start_url("/settings")).into_response());
    };
    let credential = state.store().get_hackmd_credential(user.github_id).await?;
    Ok(Html(render_settings(
        &user,
        credential.as_ref().map(|c| c.fingerprint.as_str()),
    ))
    .into_response())
}

async fn disconnect(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(user) = current_user(&state, &headers).await? else {
        return Ok(Redirect::to(&github_start_url("/settings")).into_response());
    };
    state
        .store()
        .delete_hackmd_credential(user.github_id)
        .await?;
    Ok(Redirect::to("/settings").into_response())
}

async fn mcp(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Response {
    let Some(access_token) = bearer_token(&headers) else {
        return bearer_challenge(&state);
    };
    let token_context = match state
        .store()
        .validate_access_token(
            access_token,
            &state.config().oauth_access_token_hash_key,
            &state.config().resource_url(),
        )
        .await
    {
        Ok(Some(token_context)) => token_context,
        Ok(None) => return bearer_challenge(&state),
        Err(error) => {
            tracing::warn!(%error, "access token validation failed");
            return StatusCode::UNAUTHORIZED.into_response();
        }
    };

    let credential = match state
        .store()
        .get_hackmd_credential(token_context.github_user_id)
        .await
    {
        Ok(Some(credential)) => credential,
        Ok(None) => return hackmd::HackMdError::MissingCredential.into_response(),
        Err(error) => {
            tracing::warn!(%error, "failed to load HackMD credential");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let hackmd_token = match decrypt_secret(
        &credential.nonce,
        &credential.ciphertext,
        &state.config().token_encryption_key,
    ) {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "failed to decrypt HackMD credential");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if method != Method::POST {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!(%error, "failed to read MCP request body");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    let request = match serde_json::from_slice::<hackmd::JsonRpcRequest>(&body_bytes) {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(%error, "invalid MCP JSON-RPC request");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": "parse error" }
                })),
            )
                .into_response();
        }
    };

    let response = hackmd::handle_mcp_request(
        state.http_client(),
        &state.config().hackmd_api_url,
        &hackmd_token,
        request,
    )
    .await;
    Json(response).into_response()
}

async fn validate_authorize_query(
    state: &AppState,
    query: &AuthorizeQuery,
) -> Result<(), AppError> {
    if query.response_type != "code" {
        return Err(AppError::BadRequest("response_type must be code"));
    }
    if query.code_challenge_method != "S256" {
        return Err(AppError::BadRequest("code_challenge_method must be S256"));
    }
    ScopeSet::parse(query.scope.as_deref())?;
    let Some(client) = state.store().find_client(&query.client_id).await? else {
        return Err(AppError::BadRequest("client not found"));
    };
    if !client.allows_redirect_uri(&query.redirect_uri) {
        return Err(AppError::BadRequest("redirect_uri mismatch"));
    }
    let resource = query
        .resource
        .clone()
        .unwrap_or_else(|| state.config().resource_url());
    if resource != state.config().resource_url() {
        return Err(AppError::BadRequest("resource mismatch"));
    }
    Ok(())
}

async fn issue_code_redirect(
    state: &AppState,
    query: AuthorizeQuery,
    github_user_id: i64,
) -> Result<Response, AppError> {
    let scopes = ScopeSet::parse(query.scope.as_deref())?;
    let issued = state
        .store()
        .issue_authorization_code(
            AuthorizeInput {
                client_id: query.client_id,
                redirect_uri: query.redirect_uri,
                state: query.state,
                resource: query
                    .resource
                    .unwrap_or_else(|| state.config().resource_url()),
                scopes,
                code_challenge: query.code_challenge,
                code_challenge_method: query.code_challenge_method,
            },
            github_user_id,
            state.config().authorization_code_ttl,
            &state.config().oauth_authorization_code_hash_key,
            &state.config().resource_url(),
        )
        .await?;

    let mut redirect_url = Url::parse(&issued.redirect_uri)?;
    {
        let mut pairs = redirect_url.query_pairs_mut();
        pairs.append_pair("code", &issued.code);
        if let Some(state) = issued.state {
            pairs.append_pair("state", &state);
        }
    }
    Ok(Redirect::to(redirect_url.as_str()).into_response())
}

async fn current_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<GitHubUser>, AppError> {
    let Some(session_token) = cookie_value(headers, SESSION_COOKIE) else {
        return Ok(None);
    };
    let session_hash = hmac_sha256_hex(&session_token, &state.config().session_hash_key);
    Ok(state.store().validate_web_session(&session_hash).await?)
}

fn bearer_challenge(state: &AppState) -> Response {
    hackmd::bearer_challenge(&format!(
        "{}/.well-known/oauth-protected-resource",
        state.config().public_base_url
    ))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .filter(|token| !token.is_empty())
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie.split(';') {
        let (key, value) = part.trim().split_once('=')?;
        if key == name {
            return Some(value.to_owned());
        }
    }
    None
}

fn session_cookie(token: &str, secure: bool) -> Result<HeaderValue, AppError> {
    let secure_attr = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={token}; HttpOnly{secure_attr}; SameSite=Lax; Path=/"
    ))
    .map_err(|_| AppError::Internal("invalid session cookie"))
}

fn github_start_url(return_to: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("return_to", return_to)
        .finish();
    format!("/auth/github/start?{query}")
}

fn render_token_form(user: &GitHubUser, return_to: &str, error: Option<&str>) -> String {
    let error_html = error
        .map(|error| format!(r#"<p class="error">{}</p>"#, escape_html(error)))
        .unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Connect HackMD</title>
  <style>{}</style>
</head>
<body>
  <main>
    <h1>Connect HackMD</h1>
    <p>Signed in with GitHub as <strong>{}</strong>.</p>
    <p>Paste a HackMD API token. It will be verified, encrypted, and stored for this GitHub user.</p>
    {}
    <form method="post" action="/hackmd/token">
      <input type="hidden" name="return_to" value="{}">
      <label>HackMD API Token
        <input name="hackmd_api_token" type="password" autocomplete="off" required autofocus>
      </label>
      <button type="submit">Save and continue</button>
    </form>
  </main>
</body>
</html>"#,
        page_css(),
        escape_html(&user.github_login),
        error_html,
        escape_html(return_to)
    )
}

fn render_settings(user: &GitHubUser, fingerprint: Option<&str>) -> String {
    let status = fingerprint
        .map(|fingerprint| {
            format!(
                "Connected. Token fingerprint: <code>{}</code>",
                escape_html(fingerprint)
            )
        })
        .unwrap_or_else(|| "Not connected.".to_owned());
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>HackMD MCP Settings</title>
  <style>{}</style>
</head>
<body>
  <main>
    <h1>HackMD MCP Settings</h1>
    <p>Signed in with GitHub as <strong>{}</strong>.</p>
    <p>{}</p>
    <form method="post" action="/hackmd/token">
      <input type="hidden" name="return_to" value="/settings">
      <label>Update HackMD API Token
        <input name="hackmd_api_token" type="password" autocomplete="off" required>
      </label>
      <button type="submit">Save token</button>
    </form>
    <form method="post" action="/settings/disconnect">
      <button class="secondary" type="submit">Disconnect HackMD</button>
    </form>
  </main>
</body>
</html>"#,
        page_css(),
        escape_html(&user.github_login),
        status
    )
}

fn page_css() -> &'static str {
    "body{font-family:system-ui,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;margin:0;background:#f7f7f5;color:#151515}main{max-width:560px;margin:12vh auto;padding:32px;background:#fff;border:1px solid #ddd;border-radius:8px}label{display:block;margin:24px 0 12px;font-weight:600}input{display:block;width:100%;box-sizing:border-box;margin-top:8px;padding:10px;border:1px solid #aaa;border-radius:6px;font:inherit}button{padding:10px 14px;border:0;border-radius:6px;background:#166534;color:white;font:inherit;font-weight:600;cursor:pointer}.secondary{margin-top:16px;background:#555}.error{padding:10px 12px;background:#fee2e2;border:1px solid #fecaca;border-radius:6px;color:#991b1b}code{background:#eee;padding:2px 5px;border-radius:4px}"
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Deserialize)]
struct AuthorizeQuery {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    state: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
    code_challenge: String,
    code_challenge_method: String,
}

#[derive(Debug, Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: String,
    redirect_uri: String,
    client_id: String,
    code_verifier: String,
}

#[derive(Debug, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
    scope: String,
}

#[derive(Debug, Deserialize)]
struct RevokeRequest {
    token: String,
}

#[derive(Debug, Deserialize)]
struct GitHubStartQuery {
    return_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubCallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HackMdTokenForm {
    hackmd_api_token: String,
    return_to: String,
}

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("{0}")]
    BadRequest(&'static str),
    #[error("{0}")]
    Internal(&'static str),
    #[error(transparent)]
    ClientRegistration(#[from] crate::oauth::ClientRegistrationError),
    #[error(transparent)]
    Scope(#[from] crate::oauth::ScopeError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Crypto(#[from] crate::crypto::CryptoError),
    #[error(transparent)]
    GitHub(#[from] github::GitHubError),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::BadRequest(_)
            | Self::ClientRegistration(_)
            | Self::Scope(_)
            | Self::Store(StoreError::ClientNotFound)
            | Self::Store(StoreError::RedirectUriMismatch)
            | Self::Store(StoreError::ResourceMismatch)
            | Self::Store(StoreError::InvalidAuthorizationCode)
            | Self::Store(StoreError::ExpiredAuthorizationCode)
            | Self::Store(StoreError::ConsumedAuthorizationCode)
            | Self::Store(StoreError::Pkce(_))
            | Self::Url(_) => StatusCode::BAD_REQUEST,
            Self::Internal(_)
            | Self::Store(StoreError::Sqlx(_))
            | Self::Store(StoreError::Serde(_))
            | Self::Crypto(_)
            | Self::GitHub(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        } else {
            tracing::warn!(error = %self, "request rejected");
        }
        (status, self.to_string()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header};

    use super::{bearer_token, cookie_value, escape_html};

    #[test]
    fn extracts_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc"),
        );
        assert_eq!(bearer_token(&headers), Some("abc"));
    }

    #[test]
    fn extracts_cookie_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; hmcp_session=abc; x=y"),
        );
        assert_eq!(
            cookie_value(&headers, "hmcp_session").as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn escapes_html() {
        assert_eq!(escape_html("<x>&\"'"), "&lt;x&gt;&amp;&quot;&#39;");
    }
}
