use thiserror::Error;

#[derive(Error, Debug)]
pub enum SakichanError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("User input error: {0}")]
    UserInput(String),

    #[error("{0}")]
    Other(String),
}

impl From<String> for SakichanError {
    fn from(s: String) -> Self {
        SakichanError::Other(s)
    }
}

impl From<&str> for SakichanError {
    fn from(s: &str) -> Self {
        SakichanError::Other(s.to_string())
    }
}

pub type Result<T> = std::result::Result<T, SakichanError>;