use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::{github, store::StoreError};

#[derive(Debug, thiserror::Error)]
pub(super) enum AppError {
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
