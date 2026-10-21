use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // 400
    InvalidRequest,
    InvalidCursor,
    AmbiguousTime,

    // 401
    AuthRequired,
    AuthExpired,
    BootstrapInvalid,

    // 403
    OriginDenied,
    CsrfDenied,
    ScopeDenied,
    ActionDenied,
    SecretAccessDenied,

    // 404
    NotFound,

    // 409
    Conflict,
    StaleRevision,
    ApprovalRequired,
    ApprovalExpired,
    PlanChanged,
    NeedsReconciliation,
    ResourceInUse,

    // 410
    EventCursorExpired,
    ArtifactExpired,

    // 413
    BodyTooLarge,
    OutputLimit,

    // 415
    UnsupportedMedia,

    // 422
    UnsupportedCapability,
    InvalidPlan,
    ModelUnavailable,
    TestsFailed,

    // 429
    RateLimited,
    Busy,

    // 502
    ProviderError,
    ToolFailed,

    // 503
    Offline,
    KeyringUnavailable,
    SandboxUnavailable,
    DbUnavailable,
    StorageLimit,
    SchemaTooNew,
    NotificationUnavailable,
    ChildStillRunning,
    MigrationFailed,

    // 504
    Timeout,

    // 500
    Internal,
    IntegrityFailure,

    // Job error status
    Cancelled,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_string(self).unwrap_or_else(|_| "INTERNAL".to_string());
        write!(f, "{}", s.trim_matches('"'))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_errors: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_cursor: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub details: PublicDetails,
}

#[derive(Debug)]
pub struct AppError {
    pub code: ErrorCode,
    pub safe_message: String,
    pub retryable: bool,
    pub details: Box<PublicDetails>,
    pub request_id: Option<Uuid>,
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl AppError {
    pub fn new(code: ErrorCode, safe_message: impl Into<String>) -> Self {
        Self {
            code,
            safe_message: safe_message.into(),
            retryable: false,
            details: Box::new(PublicDetails::default()),
            request_id: None,
            source: None,
        }
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn with_details(mut self, details: PublicDetails) -> Self {
        self.details = Box::new(details);
        self
    }

    pub fn with_request_id(mut self, request_id: Uuid) -> Self {
        self.request_id = Some(request_id);
        self
    }

    pub fn with_source(
        mut self,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.safe_message
    }

    pub fn public_error(&self) -> PublicError {
        PublicError {
            code: self.code,
            message: self.safe_message.clone(),
            retryable: self.retryable,
            details: (*self.details).clone(),
        }
    }

    pub fn status(&self) -> StatusCode {
        match self.code {
            ErrorCode::InvalidRequest | ErrorCode::InvalidCursor | ErrorCode::AmbiguousTime => {
                StatusCode::BAD_REQUEST
            }
            ErrorCode::AuthRequired | ErrorCode::AuthExpired | ErrorCode::BootstrapInvalid => {
                StatusCode::UNAUTHORIZED
            }
            ErrorCode::OriginDenied
            | ErrorCode::CsrfDenied
            | ErrorCode::ScopeDenied
            | ErrorCode::ActionDenied
            | ErrorCode::SecretAccessDenied => StatusCode::FORBIDDEN,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::Conflict
            | ErrorCode::StaleRevision
            | ErrorCode::ApprovalRequired
            | ErrorCode::ApprovalExpired
            | ErrorCode::PlanChanged
            | ErrorCode::NeedsReconciliation
            | ErrorCode::ResourceInUse => StatusCode::CONFLICT,
            ErrorCode::EventCursorExpired | ErrorCode::ArtifactExpired => StatusCode::GONE,
            ErrorCode::BodyTooLarge | ErrorCode::OutputLimit => StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::UnsupportedMedia => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ErrorCode::UnsupportedCapability
            | ErrorCode::InvalidPlan
            | ErrorCode::ModelUnavailable
            | ErrorCode::TestsFailed => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::RateLimited | ErrorCode::Busy => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::ProviderError | ErrorCode::ToolFailed => StatusCode::BAD_GATEWAY,
            ErrorCode::Offline
            | ErrorCode::KeyringUnavailable
            | ErrorCode::SandboxUnavailable
            | ErrorCode::DbUnavailable
            | ErrorCode::StorageLimit
            | ErrorCode::SchemaTooNew
            | ErrorCode::NotificationUnavailable
            | ErrorCode::ChildStillRunning
            | ErrorCode::MigrationFailed => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Timeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::Internal | ErrorCode::IntegrityFailure | ErrorCode::Cancelled => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.safe_message)
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|s| s.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        match err {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::new(ErrorCode::NotFound, "Record not found")
            }
            _ => AppError::new(ErrorCode::DbUnavailable, err.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
