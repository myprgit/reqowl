use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("line {line}: {msg}")]
    Parse { line: usize, msg: String },

    #[error("undefined variable{}: {}", if .0.len() == 1 { "" } else { "s" }, .0.join(", "))]
    UndefinedVariables(Vec<String>),

    #[error("{0}")]
    Variable(String),

    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path}: {msg}")]
    Config { path: PathBuf, msg: String },

    #[error("environment \"{0}\" not found")]
    UnknownEnv(String),

    #[error("request failed: {0}")]
    Http(String),

    #[error("{0} requests are not supported yet")]
    Unsupported(&'static str),

    #[error("WebSocket: {0}")]
    WebSocket(String),

    #[error("OAuth2: {0}")]
    OAuth(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
