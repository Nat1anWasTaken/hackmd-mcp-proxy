use axum::{
    Form, Json, Router,
    extract::{OriginalUri, Query, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    crypto::{decrypt_secret, encrypt_secret, fingerprint, hmac_sha256_hex, random_token},
    github, hackmd,
    oauth::{
        AuthorizationServerMetadata, ClientRegistrationRequest, ProtectedResourceMetadata,
        ScopeSet, localhost_redirects_allowed,
    },
    state::AppState,
    store::{AuthorizeInput, ExchangeCodeInput},
};

mod error;
mod pages;
mod session;

use error::AppError;
use pages::{render_settings, render_token_form};
use session::{bearer_challenge, bearer_token, current_user, github_start_url, session_cookie};

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
