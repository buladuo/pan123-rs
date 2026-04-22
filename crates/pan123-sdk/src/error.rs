use thiserror::Error;

pub type Result<T> = std::result::Result<T, Pan123Error>;

#[derive(Debug, Error)]
pub enum Pan123Error {
    #[error("io error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    #[error("http error: {source}")]
    Http {
        #[from]
        source: reqwest::Error,
    },

    #[error("json error: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },

    #[error("api error {code}: {message}")]
    Api { code: i64, message: String },

    #[error("authentication required")]
    AuthRequired,

    #[error("resource not found: {0}")]
    NotFound(String),

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("operation failed: {0}")]
    Operation(String),

    #[error("network timeout after {attempts} attempts")]
    Timeout { attempts: usize },

    #[error("rate limit exceeded, retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("file conflict: {path}")]
    FileConflict { path: String },

    #[error("insufficient storage space")]
    InsufficientStorage,

    #[error("invalid token format")]
    InvalidToken,

    #[error("encryption error: {0}")]
    Encryption(String),

    #[error("configuration error: {0}")]
    Config(String),
}

impl Pan123Error {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Pan123Error::Io { .. }
                | Pan123Error::Http { .. }
                | Pan123Error::Timeout { .. }
        )
    }

    pub fn is_auth_error(&self) -> bool {
        matches!(
            self,
            Pan123Error::AuthRequired | Pan123Error::InvalidToken
        )
    }

    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            Pan123Error::InvalidPath(_)
                | Pan123Error::FileConflict { .. }
                | Pan123Error::Config(_)
        )
    }

    pub fn with_context(self, context: impl Into<String>) -> Self {
        match self {
            Pan123Error::Operation(msg) => {
                Pan123Error::Operation(format!("{}: {}", context.into(), msg))
            }
            other => other,
        }
    }
}
