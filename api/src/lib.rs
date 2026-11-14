pub mod agents;
mod migrations;
pub mod settings;
pub mod workspace;

use std::sync::Arc;

use erno::{app::App, app_info::AppInfo, boot::BootConfig, jobs::job_registry::JobRegistry};

pub use migrations::Migrator;
pub use settings::Settings;

use crate::workspace::recovery::release_interrupted_threads;
use crate::workspace::{project, thread, thread_entry};

/// The application state every handler receives.
pub type Teryon = App<Settings>;

pub fn boot_config() -> BootConfig<Settings> {
    BootConfig::new(
        AppInfo::new("teryon", env!("CARGO_PKG_VERSION"), ""),
        workspace::router,
        JobRegistry::new(),
        vec![],
    )
    .with_sync::<project::Entity>()
    .with_sync::<thread::Entity>()
    .with_sync::<thread_entry::Entity>()
    .with_post_migrate(Arc::new(|db| {
        Box::pin(async move {
            release_interrupted_threads(&db)
                .await
                .map(|_| ())
                .map_err(|problem| problem.to_string())
        })
    }))
}
