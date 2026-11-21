//! Everything Teryon does on a machine, without any network or interface.
//!
//! The local service, the terminal app and the desktop app all build on this.
//! It runs agents, reads files and history from git, and keeps projects and
//! threads in plain files. Who asked, and over which connection, is the
//! business of whoever uses it.

pub mod agents;
pub mod changes;
pub mod directory;
pub mod files;
pub mod history;
pub mod paths;
pub mod plans;
pub mod protocol;
pub mod settings;
pub mod store;
pub mod usage;
pub mod workflow;
pub mod workspace;
