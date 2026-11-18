//! The Teryon server: what a group shares, on a machine of its own.
//!
//! For now this is erno's own part only: accounts, administration and health.
//! Everything one person does on their own machine lives in the local
//! service (`service/` and `core/`), which needs no server. Shared projects,
//! a person's machines and the connection to them come here later.

mod migrations;
pub mod settings;

use axum::routing::get;
use axum::Router;
use erno::{app::App, app_info::AppInfo, boot::BootConfig, jobs::job_registry::JobRegistry};

pub use migrations::Migrator;
pub use settings::Settings;

/// The application state every handler receives.
pub type Teryon = App<Settings>;

pub fn boot_config() -> BootConfig<Settings> {
    BootConfig::new(
        AppInfo::new("teryon", env!("CARGO_PKG_VERSION"), ""),
        router,
        JobRegistry::new(),
        vec![],
    )
}

async fn health() -> &'static str {
    "OK"
}

/// Teryon's own routes, mounted by erno under `/api`.
fn router(app: Teryon) -> Router {
    Router::new().route("/health", get(health)).with_state(app)
}
