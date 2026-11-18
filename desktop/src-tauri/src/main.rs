//! Teryon's desktop app: a window around the Angular app, connected to the local service.
//!
//! The window does no work of its own. The local service owns the projects
//! and runs the agents, so they keep working when the window closes. All the
//! window needs is where the service listens and its token, which only a
//! program on this machine running as this person can read from
//! `service.json`. The one command here hands those over, starting the
//! service first when it is not running.
//!
//! The `teryon` program ships next to this one: the bundle places it there,
//! and in development cargo builds both into the same folder.

// No console window next to the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env::{current_exe, var_os};
use std::path::PathBuf;

use serde::Serialize;

use teryon::starting::running_service;
use teryon_core::paths::Folders;

/// Where the service listens, and the token it asks for.
#[derive(Serialize)]
struct ServiceConnection {
    port: u16,
    token: String,
}

/// Where the service is, starting it when it is not running. The error is meant for the person.
#[tauri::command]
async fn service_connection() -> Result<ServiceConnection, String> {
    let home = var_os("TERYON_HOME").map(PathBuf::from);
    let folders = match &home {
        Some(home) => Folders::inside(home),
        None => Folders::usual().map_err(|problem| problem.to_string())?,
    };
    let file = running_service(
        &teryon_program()?,
        &folders.data,
        &folders.logs,
        home.as_deref(),
    )
    .await
    .map_err(|problem| problem.to_string())?;
    Ok(ServiceConnection {
        port: file.port,
        token: file.token,
    })
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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![service_connection])
        .run(tauri::generate_context!())
        .expect("the desktop app could not start");
}
