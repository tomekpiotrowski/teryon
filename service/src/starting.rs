//! Makes sure the service is running, starting it when it is not.
//!
//! The desktop app and the terminal app both call this before connecting. A
//! person alone on their machine never starts the service by hand: whichever
//! app they open first starts it, and it keeps running after that app closes,
//! so agents keep working.
//!
//! A service of another version may be running, left over from before Teryon
//! was updated or rebuilt. This program cannot understand its messages, so
//! that is reported rather than waited on, and [`restarted_service`] replaces
//! it once the person agrees, since restarting stops the agents that are
//! working.
//!
//! The service is started detached from whoever started it: in a process
//! group of its own on Linux and macOS, so closing a terminal does not end
//! it, and without a console on Windows.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use thiserror::Error;
use tokio::time::sleep;

use crate::client::Client;
use crate::service_file::{ServiceFile, ServiceFileError};
use crate::stopping::{stop_service, StopError};

/// How long a freshly started service gets to start listening.
const START_PATIENCE: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
pub enum StartError {
    #[error("Could not start Teryon's service ({program}): {problem}")]
    NotStarted {
        program: String,
        problem: std::io::Error,
    },
    #[error("Teryon's service did not start in time. Its log is in {logs}.")]
    TooSlow { logs: String },
    #[error("A different version of Teryon is running, and this app cannot talk to it. Restart Teryon to use this version. Restarting stops the agents that are working.")]
    DifferentVersionRunning,
    #[error("The service this app started ({program}) is a different version than the app. Install or build them together.")]
    DifferentVersionStarted { program: String },
    #[error(transparent)]
    ServiceFile(#[from] ServiceFileError),
    #[error(transparent)]
    Stop(#[from] StopError),
}

/// What a client needs to connect to a service that answers, starting one when needed.
///
/// `program` is the `teryon` program to start; `home` is passed on as
/// `--home` when the folders are not the usual ones.
pub async fn running_service(
    program: &Path,
    data_folder: &Path,
    logs_folder: &Path,
    home: Option<&Path>,
) -> Result<ServiceFile, StartError> {
    if let Some(file) = answering(data_folder).await? {
        return if file.speaks_this_version() {
            Ok(file)
        } else {
            Err(StartError::DifferentVersionRunning)
        };
    }

    let mut command = Command::new(program);
    if let Some(home) = home {
        command.arg("--home").arg(home);
    }
    command
        .arg("service")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    command.spawn().map_err(|problem| StartError::NotStarted {
        program: program.display().to_string(),
        problem,
    })?;

    let attempts = START_PATIENCE.as_millis() / CHECK_EVERY.as_millis();
    for _ in 0..attempts {
        sleep(CHECK_EVERY).await;
        if let Some(file) = answering(data_folder).await? {
            return if file.speaks_this_version() {
                Ok(file)
            } else {
                Err(StartError::DifferentVersionStarted {
                    program: program.display().to_string(),
                })
            };
        }
    }
    Err(StartError::TooSlow {
        logs: logs_folder.display().to_string(),
    })
}

/// Stops the running service, of whatever version, and starts this version's.
///
/// Stopping it stops the agents that are working, so this is only for when
/// the person asked.
pub async fn restarted_service(
    program: &Path,
    data_folder: &Path,
    logs_folder: &Path,
    home: Option<&Path>,
) -> Result<ServiceFile, StartError> {
    stop_service(data_folder).await?;
    running_service(program, data_folder, logs_folder, home).await
}

/// The service named in `service.json`, of any version, when it answers with that token.
async fn answering(data_folder: &Path) -> Result<Option<ServiceFile>, StartError> {
    let Some(file) = ServiceFile::read(data_folder)? else {
        return Ok(None);
    };
    Ok(Client::answers(&file).await.then_some(file))
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}
