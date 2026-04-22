use thiserror::Error;

pub type Result<T> = std::result::Result<T, Pan123Error>;

#[derive(Debug, Error)]
pub enum Pan123Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
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
}
