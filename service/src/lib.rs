//! The local service: the one process per person per machine that owns local projects.
//!
//! The desktop app, the terminal app and, through a Teryon server, the web
//! and mobile apps are all clients of it. It runs agents through the core
//! library and tells every client about every change as it happens.

pub mod client;
pub mod guard;
pub mod local;
pub mod logging;
pub mod service;
pub mod service_file;
pub mod session;
pub mod starting;
pub mod stopping;
