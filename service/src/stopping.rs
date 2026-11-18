//! Stops the running service, of whatever version.
//!
//! `teryon stop` uses this, and so does the desktop app when the person
//! restarts a service of another version. Only the token and the request to
//! stop must be understood by the service, so a service from before Teryon
//! was updated can still be stopped by the new version.

use std::path::Path;
use std::time::Duration;

use thiserror::Error;
use tokio::time::sleep;

use crate::client::{Client, ClientError};
use crate::service_file::{ServiceFile, ServiceFileError};

/// How long the service gets to stop its agents and end.
///
/// The service gives its agents ten seconds, so this leaves it room.
const STOP_PATIENCE: Duration = Duration::from_secs(30);
const CHECK_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
pub enum StopError {
    #[error("Teryon is taking long to stop.")]
    TooSlow,
    #[error(transparent)]
    ServiceFile(#[from] ServiceFileError),
}

/// How stopping went.
#[derive(Debug, PartialEq, Eq)]
pub enum Stopped {
    /// The service stopped its agents and ended.
    Stopped,
    /// Nobody answered, so there was nothing to stop.
    NotRunning,
}

/// Asks the service to stop and waits until it has stopped every agent and ended.
///
/// The service removes `service.json` as the last thing it does, so the file
/// going, or being replaced by another service's, is the sign it has ended.
pub async fn stop_service(data_folder: &Path) -> Result<Stopped, StopError> {
    let Some(file) = ServiceFile::read(data_folder)? else {
        return Ok(Stopped::NotRunning);
    };
    match Client::ask_to_stop(&file).await {
        Ok(()) => {}
        Err(ClientError::ServiceFile(problem)) => return Err(StopError::ServiceFile(problem)),
        Err(_) => return Ok(Stopped::NotRunning),
    }
    let attempts = STOP_PATIENCE.as_millis() / CHECK_EVERY.as_millis();
    for _ in 0..attempts {
        if !matches!(ServiceFile::read(data_folder), Ok(Some(current)) if current == file) {
            return Ok(Stopped::Stopped);
        }
        sleep(CHECK_EVERY).await;
    }
    Err(StopError::TooSlow)
}
