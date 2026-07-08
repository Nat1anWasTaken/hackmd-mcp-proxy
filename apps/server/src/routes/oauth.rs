use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Json, Router,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    config::Environment,
    oauth::{
        clients::{ClientRegistrationRequest, ClientRegistrationResponse},
        metadata::{AuthorizationServerMetadata, ProtectedResourceMetadata},
        scopes::ScopeSet,
        store::{AuthorizeInput, ExchangeCodeInput, OAuthStoreError},
    },
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
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
) -> Result<Json<ClientRegistrationResponse>, OAuthHttpError> {
    let allow_localhost_redirects = state.config().environment != Environment::Production;
    let request = request.validate(allow_localhost_redirects)?;
    let client = state.oauth_store().register_client(request).await;
    Ok(Json(client.response()))
}

async fn authorize(
    State(state): State<AppState>,
    Query(query): Query<AuthorizeQuery>,
) -> Result<Response, OAuthHttpError> {
    if query.response_type != "code" {
        return Err(OAuthHttpError::BadRequest("response_type must be code"));
    }
    if !state.config().oauth_auto_approve {
        return Err(OAuthHttpError::BadRequest(
            "interactive consent is not available yet",
        ));
    }

    let scopes = ScopeSet::parse(query.scope.as_deref())?;
    let issued = state
        .oauth_store()
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
            "single-user".to_owned(),
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

async fn token(
    State(state): State<AppState>,
    Form(request): Form<TokenRequest>,
) -> Result<Json<TokenResponse>, OAuthHttpError> {
    if request.grant_type != "authorization_code" {
        return Err(OAuthHttpError::BadRequest(
            "grant_type must be authorization_code",
        ));
    }
    let issued = state
        .oauth_store()
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
    state
        .oauth_store()
        .revoke_access_token(&request.token, &state.config().oauth_access_token_hash_key)
        .await;
    StatusCode::OK
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

#[derive(Debug, thiserror::Error)]
enum OAuthHttpError {
    #[error("{0}")]
    BadRequest(&'static str),
    #[error(transparent)]
    ClientRegistration(#[from] crate::oauth::clients::ClientRegistrationError),
    #[error(transparent)]
    Scope(#[from] crate::oauth::scopes::ScopeError),
    #[error(transparent)]
    Store(#[from] OAuthStoreError),
    #[error(transparent)]
    Url(#[from] url::ParseError),
}

impl IntoResponse for OAuthHttpError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::BadRequest(_)
            | Self::ClientRegistration(_)
            | Self::Scope(_)
            | Self::Store(_)
            | Self::Url(_) => StatusCode::BAD_REQUEST,
        };
        (status, self.to_string()).into_response()
    }
}
