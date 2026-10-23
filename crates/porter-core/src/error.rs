use std::io;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: not found")]
    NotFound(String),

    #[error("{0}: already exists")]
    AlreadyExists(String),

    #[error("{0}: permission denied")]
    PermissionDenied(String),

    /// A rename is not possible here (across file systems, or the backend can't
    /// rename this kind of entry); callers fall back to copy + delete.
    #[error("{0}: cannot rename across file systems")]
    CrossDevice(String),

    #[error("`{op}` is not supported by the {scheme} backend")]
    Unsupported { op: &'static str, scheme: String },

    #[error("invalid location `{input}`: {reason}")]
    InvalidLocation { input: String, reason: String },

    #[error("{0}")]
    InvalidArgument(String),

    #[error("cancelled")]
    Cancelled,

    /// Any other failure reported by a backend, a credential store or the config.
    #[error("{context}: {message}")]
    Backend { context: String, message: String },

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
            io::ErrorKind::AlreadyExists => Self::AlreadyExists(path),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(path),
            io::ErrorKind::CrossesDevices => Self::CrossDevice(path),
            _ => Self::Io { path, source },
        }
    }

    pub fn invalid_location(input: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidLocation {
            input: input.into(),
            reason: reason.into(),
        }
    }

    pub fn backend(context: impl Into<String>, message: impl ToString) -> Self {
        Self::Backend {
            context: context.into(),
            message: message.to_string(),
        }
    }

    pub fn unsupported(op: &'static str, scheme: &str) -> Self {
        Self::Unsupported {
            op,
            scheme: scheme.to_owned(),
        }
    }
}
