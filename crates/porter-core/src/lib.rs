//! Core types shared by every Porter backend and front end.

mod entry;
mod error;
mod location;
mod provider;

pub use entry::{Entry, EntryKind, FsInfo, Permissions};
pub use error::{Error, Result};
pub use location::Location;
pub use provider::{Capabilities, EntryStream, Provider};
