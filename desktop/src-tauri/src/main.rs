//! Teryon's desktop app: a window around the Angular app, connected to the local service.
//!
//! The window does no work of its own. The local service owns the projects
//! and runs the agents, so they keep working when the window closes. All the
//! window needs is where the service listens and its token, which only a
//! program on this machine running as this person can read from
//! `service.json`. The one command here hands those over, starting the
//! service first when it is not running.
//!
//! When the service cannot be reached the window is told why, and whether
//! restarting it would help: a service of another version keeps running
//! after Teryon is updated or rebuilt, and this app cannot talk to it.
//!
//! The `teryon` program ships next to this one: the bundle places it there,
//! and in development cargo builds both into the same folder.

// No console window next to the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env::{current_exe, var_os};
use std::path::PathBuf;

use serde::Serialize;
use tauri::{Manager, State};
use teryon_desktop::terminal_command::{SetupReport, TerminalCommand};

use teryon::service_file::ServiceFile;
use teryon::starting::{restarted_service, running_service, StartError};
use teryon_core::paths::Folders;

/// Where the service listens, and the token it asks for.
#[derive(Serialize)]
struct ServiceConnection {
    port: u16,
    token: String,
}

impl From<ServiceFile> for ServiceConnection {
    fn from(file: ServiceFile) -> Self {
        Self {
            port: file.port,
            token: file.token,
        }
    }
}

/// Why the service cannot be reached, for the window to show.
///
/// The window offers to restart the service only when that would help,
/// because restarting stops the agents that are working.
#[derive(Serialize)]
struct Unreachable {
    /// Meant for the person.
    reason: String,
    /// Whether restarting the service would let the window connect.
    restartable: bool,
}

impl From<StartError> for Unreachable {
    fn from(problem: StartError) -> Self {
        Self {
            restartable: matches!(problem, StartError::DifferentVersionRunning),
            reason: problem.to_string(),
        }
    }
}

impl From<String> for Unreachable {
    fn from(reason: String) -> Self {
        Self {
            reason,
            restartable: false,
        }
    }
}

/// The folders Teryon keeps its things in, and the home folder when it is not the usual one.
fn folders() -> Result<(Folders, Option<PathBuf>), Unreachable> {
    let home = var_os("TERYON_HOME").map(PathBuf::from);
    let folders = match &home {
        Some(home) => Folders::inside(home),
        None => Folders::usual().map_err(|problem| problem.to_string())?,
    };
    Ok((folders, home))
}

/// Where the service is, starting it when it is not running.
#[tauri::command]
async fn service_connection() -> Result<ServiceConnection, Unreachable> {
    let (folders, home) = folders()?;
    let file = running_service(
        &teryon_program()?,
        &folders.data,
        &folders.logs,
        home.as_deref(),
    )
    .await?;
    Ok(file.into())
}

/// Stops the running service, of whatever version, and starts this version's.
///
/// Only when the person asked: it stops the agents that are working.
#[tauri::command]
async fn restart_service() -> Result<ServiceConnection, Unreachable> {
    let (folders, home) = folders()?;
    let file = restarted_service(
        &teryon_program()?,
        &folders.data,
        &folders.logs,
        home.as_deref(),
    )
    .await?;
    Ok(file.into())
}

/// The `teryon` program next to this one.
fn teryon_program() -> Result<PathBuf, String> {
    let here = current_exe()
        .map_err(|problem| format!("Could not find where Teryon is installed: {problem}"))?;
    let folder = here
        .parent()
        .ok_or_else(|| "Could not find where Teryon is installed.".to_string())?;
    let name = if cfg!(windows) {
        "teryon.exe"
    } else {
        "teryon"
    };
    Ok(folder.join(name))
}

/// Setup results remain separate from service connection errors.
#[tauri::command]
fn terminal_command_status(state: State<'_, TerminalCommand>) -> SetupReport {
    state.status()
}

/// Retry only on request; a replacement names the exact conflict the person saw.
#[tauri::command]
fn install_terminal_command(
    state: State<'_, TerminalCommand>,
    replacement: Option<String>,
) -> SetupReport {
    state.install(replacement)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(TerminalCommand::start());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            service_connection,
            restart_service,
            terminal_command_status,
            install_terminal_command
        ])
        .run(tauri::generate_context!())
        .expect("the desktop app could not start");
}
