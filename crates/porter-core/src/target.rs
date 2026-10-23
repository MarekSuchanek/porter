use std::fmt;
use std::sync::Arc;

use crate::{Location, Provider};

/// A location paired with the provider that serves it.
#[derive(Clone)]
pub struct Target {
    pub location: Location,
    pub provider: Arc<dyn Provider>,
}

impl Target {
    pub fn new(location: Location, provider: Arc<dyn Provider>) -> Self {
        Self { location, provider }
    }

    pub fn path(&self) -> &str {
        &self.location.path
    }

    /// The same backend at another path.
    pub fn with_path(&self, path: impl Into<String>) -> Self {
        Self {
            location: Location {
                path: path.into(),
                ..self.location.clone()
            },
            provider: self.provider.clone(),
        }
    }

    pub fn child(&self, name: &str) -> Self {
        self.with_path(self.provider.join(self.path(), name))
    }

    pub fn parent(&self) -> Option<Self> {
        self.provider.parent(self.path()).map(|p| self.with_path(p))
    }

    pub fn file_name(&self) -> Option<String> {
        self.provider.file_name(self.path())
    }

    /// Whether both targets live on the same backend instance (same server, bucket…).
    pub fn same_backend(&self, other: &Target) -> bool {
        self.location.scheme == other.location.scheme
            && self.location.authority == other.location.authority
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.location.fmt(f)
    }
}

impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Target").field(&self.location).finish()
    }
}
