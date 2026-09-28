use std::error::Error;
use std::fmt::Display;

pub type BoxedError = Box<dyn Error + Send + Sync + 'static>;

#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Conflict(String),
    Docker(String),
    Internal(String),
}

impl Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::NotFound(msg) => write!(f, "not found: {msg}"),
            AppError::Conflict(msg) => write!(f, "conflict: {msg}"),
            AppError::Docker(msg) => write!(f, "docker error: {msg}"),
            AppError::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Internal(err.to_string())
    }
}
