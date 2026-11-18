//! The Teryon server's own configuration, beside erno's.
//!
//! The server holds nothing of its own yet: projects and agents live on each
//! person's machine, in the local service. Organizations, machines and shared
//! projects come here when server mode is built.

use serde::Deserialize;

/// Everything Teryon adds to erno's configuration. Nothing yet.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Settings {}
