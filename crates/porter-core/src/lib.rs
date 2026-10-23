//! Core types shared by every Porter backend and front end.

mod entry;
mod error;
mod location;
mod mode;
mod provider;
mod target;

pub use entry::{Entry, EntryKind, FsInfo, Permissions};
pub use error::{Error, Result};
pub use location::Location;
pub use mode::{Clause, ModeSpec};
pub use provider::{Capabilities, EntryStream, NameFilter, Provider, Reader, WalkOptions, Writer};
pub use target::Target;
