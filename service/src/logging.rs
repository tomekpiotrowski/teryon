//! Where the service writes what it does.
//!
//! The service runs in the background with nobody watching its output, so it
//! writes to daily files in the logs folder too, keeping a week of them.
//! `TERYON_LOG` chooses how much is written, as in `TERYON_LOG=debug`.

use std::fs::create_dir_all;
use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// How many daily files are kept.
const DAYS_KEPT: usize = 7;

/// Starts writing to the terminal and to the logs folder.
///
/// The returned guard must be kept for as long as the program runs, or the
/// last lines may not reach the file.
pub fn start(logs: &Path) -> Option<WorkerGuard> {
    let _ = create_dir_all(logs);
    let filter = EnvFilter::try_from_env("TERYON_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let file = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("service")
        .filename_suffix("log")
        .max_log_files(DAYS_KEPT)
        .build(logs)
        .ok();
    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr));
    match file {
        Some(file) => {
            let (writer, guard) = tracing_appender::non_blocking(file);
            registry
                .with(fmt::layer().with_ansi(false).with_writer(writer))
                .init();
            Some(guard)
        }
        None => {
            registry.init();
            None
        }
    }
}
