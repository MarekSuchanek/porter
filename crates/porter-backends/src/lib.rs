//! Storage backends and the registry that picks one for a [`Location`].

pub mod local;

use std::sync::Arc;

use porter_core::{Error, Location, Provider, Result};

pub use local::LocalProvider;

/// Returns the provider able to serve `location`.
pub fn provider_for(location: &Location) -> Result<Arc<dyn Provider>> {
    match location.scheme.as_str() {
        Location::LOCAL_SCHEME => Ok(Arc::new(LocalProvider)),
        other => Err(Error::invalid_location(
            location.to_string(),
            format!("no backend for scheme `{other}` yet"),
        )),
    }
}
