use axum::{
    http::{HeaderMap, HeaderValue, header},
    response::Response,
};
use url::form_urlencoded;

use super::error::AppError;
use crate::{crypto::hmac_sha256_hex, hackmd, state::AppState, store::GitHubUser};

const SESSION_COOKIE: &str = "hmcp_session";

pub(super) async fn current_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<GitHubUser>, AppError> {
    let Some(session_token) = cookie_value(headers, SESSION_COOKIE) else {
        return Ok(None);
    };
    let session_hash = hmac_sha256_hex(&session_token, &state.config().session_hash_key);
    Ok(state.store().validate_web_session(&session_hash).await?)
}

pub(super) fn bearer_challenge(state: &AppState) -> Response {
    hackmd::bearer_challenge(&format!(
        "{}/.well-known/oauth-protected-resource",
        state.config().public_base_url
    ))
}

pub(super) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
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

pub(super) fn session_cookie(token: &str, secure: bool) -> Result<HeaderValue, AppError> {
    let secure_attr = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={token}; HttpOnly{secure_attr}; SameSite=Lax; Path=/"
    ))
    .map_err(|_| AppError::Internal("invalid session cookie"))
}

pub(super) fn github_start_url(return_to: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("return_to", return_to)
        .finish();
    format!("/auth/github/start?{query}")
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header};

    use super::{bearer_token, cookie_value};

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
}
