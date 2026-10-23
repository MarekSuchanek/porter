use std::io;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: not found")]
    NotFound(String),

    #[error("{0}: permission denied")]
    PermissionDenied(String),

    #[error("`{op}` is not supported by the {scheme} backend")]
    Unsupported { op: &'static str, scheme: String },

    #[error("invalid location `{input}`: {reason}")]
    InvalidLocation { input: String, reason: String },

    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
}

impl Error {
    /// Wraps an I/O error, promoting common kinds to dedicated variants.
    pub fn io(path: impl Into<String>, source: io::Error) -> Self {
        let path = path.into();
        match source.kind() {
            io::ErrorKind::NotFound => Self::NotFound(path),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(path),
            _ => Self::Io { path, source },
        }
    }

    pub fn invalid_location(input: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidLocation {
            input: input.into(),
            reason: reason.into(),
        }
    }
}
