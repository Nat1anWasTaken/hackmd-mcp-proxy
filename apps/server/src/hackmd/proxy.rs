use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::TryStreamExt;
use reqwest::Url;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("HackMD API token is not configured")]
    MissingHackmdToken,
    #[error("invalid upstream MCP URL")]
    InvalidUpstreamUrl(#[from] url::ParseError),
    #[error("failed to read MCP request body")]
    RequestBody(#[from] axum::Error),
    #[error("HackMD MCP upstream request failed")]
    Upstream(#[from] reqwest::Error),
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::MissingHackmdToken => StatusCode::SERVICE_UNAVAILABLE,
            Self::InvalidUpstreamUrl(_) | Self::RequestBody(_) | Self::Upstream(_) => {
                StatusCode::BAD_GATEWAY
            }
        };

        tracing::warn!(error = %self, "mcp proxy request failed");
        (status, self.to_string()).into_response()
    }
}

pub async fn proxy_mcp_request(
    client: &reqwest::Client,
    upstream_mcp_url: &str,
    hackmd_api_token: &str,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ProxyError> {
    let url = upstream_url(upstream_mcp_url, &uri)?;
    let body_bytes = axum::body::to_bytes(body, usize::MAX).await?;
    let mut request = client
        .request(method, url)
        .headers(filtered_request_headers(&headers))
        .bearer_auth(hackmd_api_token);

    if !body_bytes.is_empty() {
        request = request.body(body_bytes);
    }

    let upstream = request.send().await?;
    Ok(streaming_response(upstream))
}

fn upstream_url(base: &str, request_uri: &Uri) -> Result<Url, url::ParseError> {
    let mut url = Url::parse(base)?;
    url.set_query(request_uri.query());
    Ok(url)
}

pub fn filtered_request_headers(headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::new();
    for (name, value) in headers {
        if should_forward_request_header(name) {
            filtered.append(name, value.clone());
        }
    }
    filtered
}

pub fn filtered_response_headers(headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::new();
    for (name, value) in headers {
        if should_forward_response_header(name) {
            filtered.append(name, value.clone());
        }
    }
    filtered
}

fn should_forward_request_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "accept"
            | "content-type"
            | "mcp-protocol-version"
            | "mcp-session-id"
            | "last-event-id"
            | "user-agent"
    )
}

fn should_forward_response_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "content-type" | "mcp-session-id" | "mcp-protocol-version" | "cache-control"
    )
}

fn streaming_response(upstream: reqwest::Response) -> Response {
    let status = upstream.status();
    let headers = filtered_response_headers(upstream.headers());
    let stream = upstream.bytes_stream().map_err(std::io::Error::other);
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    response.headers_mut().extend(headers);
    response
}

pub fn bearer_challenge(resource_metadata_url: &str) -> Response {
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    if let Ok(value) = HeaderValue::from_str(&format!(
        r#"Bearer resource_metadata="{resource_metadata_url}""#
    )) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

pub fn json_rpc_error(id: Option<serde_json::Value>, code: i64, message: &str) -> Response {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(serde_json::Value::Null),
        "error": {
            "code": code,
            "message": message,
        }
    });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

pub fn bytes_to_json_rpc_id(body: &Bytes) -> Option<serde_json::Value> {
    let value = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    value.get("id").cloned()
}

#[cfg(test)]
mod tests {
    use axum::http::{header, HeaderMap, HeaderValue};

    use super::{filtered_request_headers, filtered_response_headers};

    #[test]
    fn request_header_filter_removes_credentials_and_network_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrapper"),
        );
        headers.insert(header::COOKIE, HeaderValue::from_static("session=secret"));
        headers.insert("mcp-session-id", HeaderValue::from_static("session-1"));
        headers.insert("x-forwarded-for", HeaderValue::from_static("127.0.0.1"));

        let filtered = filtered_request_headers(&headers);

        assert_eq!(
            filtered.get(header::ACCEPT),
            Some(&HeaderValue::from_static("application/json"))
        );
        assert_eq!(
            filtered.get("mcp-session-id"),
            Some(&HeaderValue::from_static("session-1"))
        );
        assert!(!filtered.contains_key(header::AUTHORIZATION));
        assert!(!filtered.contains_key(header::COOKIE));
        assert!(!filtered.contains_key("x-forwarded-for"));
    }

    #[test]
    fn response_header_filter_preserves_only_mcp_safe_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        headers.insert("mcp-session-id", HeaderValue::from_static("session-1"));
        headers.insert(
            header::SET_COOKIE,
            HeaderValue::from_static("upstream=secret"),
        );
        headers.insert(header::SERVER, HeaderValue::from_static("upstream"));

        let filtered = filtered_response_headers(&headers);

        assert_eq!(
            filtered.get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/event-stream"))
        );
        assert_eq!(
            filtered.get("mcp-session-id"),
            Some(&HeaderValue::from_static("session-1"))
        );
        assert!(!filtered.contains_key(header::SET_COOKIE));
        assert!(!filtered.contains_key(header::SERVER));
    }
}
