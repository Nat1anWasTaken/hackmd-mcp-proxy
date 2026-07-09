use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::patch;

#[derive(Debug, thiserror::Error)]
pub(crate) enum HackMdError {
    #[error("HackMD API token is not configured for this user")]
    MissingCredential,
    #[error("invalid HackMD API URL")]
    InvalidApiUrl(#[from] url::ParseError),
    #[error("invalid HackMD API request: {0}")]
    InvalidRequest(String),
    #[error("HackMD API request failed: {0}")]
    Api(String),
    #[error("HackMD API upstream request failed")]
    Upstream(#[from] reqwest::Error),
    #[error(transparent)]
    Patch(#[from] patch::PatchError),
}

impl IntoResponse for HackMdError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::MissingCredential => StatusCode::FORBIDDEN,
            Self::InvalidRequest(_) | Self::Patch(_) => StatusCode::BAD_REQUEST,
            Self::InvalidApiUrl(_) | Self::Api(_) | Self::Upstream(_) => StatusCode::BAD_GATEWAY,
        };
        tracing::warn!(error = %self, "HackMD request failed");
        (status, self.to_string()).into_response()
    }
}
