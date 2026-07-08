use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use futures_util::TryStreamExt;
use reqwest::Url;

#[derive(Debug, thiserror::Error)]
pub enum HackMdError {
    #[error("HackMD API token is not configured for this user")]
    MissingCredential,
    #[error("invalid upstream MCP URL")]
    InvalidUpstreamUrl(#[from] url::ParseError),
    #[error("failed to read MCP request body")]
    RequestBody(#[from] axum::Error),
    #[error("HackMD MCP upstream request failed")]
    Upstream(#[from] reqwest::Error),
}

impl IntoResponse for HackMdError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::MissingCredential => StatusCode::FORBIDDEN,
            Self::InvalidUpstreamUrl(_) | Self::RequestBody(_) | Self::Upstream(_) => {
                StatusCode::BAD_GATEWAY
            }
        };
        tracing::warn!(error = %self, "HackMD request failed");
        (status, self.to_string()).into_response()
    }
}

pub async fn verify_token(
    client: &reqwest::Client,
    upstream_mcp_url: &str,
    hackmd_api_token: &str,
) -> Result<(), HackMdError> {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"hackmd-mcp-proxy","version":"0.1.0"}}}"#;
    let upstream = client
        .post(upstream_mcp_url)
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::CONTENT_TYPE, "application/json")
        .bearer_auth(hackmd_api_token)
        .body(body)
        .send()
        .await?;
    if upstream.status().is_success() {
        Ok(())
    } else {
        Err(HackMdError::MissingCredential)
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
) -> Result<Response, HackMdError> {
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

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header};

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
