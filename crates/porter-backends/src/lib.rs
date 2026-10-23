//! Storage backends and the registry that picks one for a [`Location`].

pub mod local;

use std::sync::Arc;

use porter_core::{Error, Location, Provider, Result, Target};

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

/// Parses user input into a [`Target`]; local paths are made absolute.
pub fn target(input: &str) -> Result<Target> {
    let mut location = Location::parse(input)?;
    if location.is_local() {
        let abs = std::path::absolute(&location.path).map_err(|e| Error::io(input, e))?;
        location.path = abs.display().to_string();
    }
    let provider = provider_for(&location)?;
    Ok(Target::new(location, provider))
}
