//! Background command setup, independent of the window and service connection.
//!
//! The mutex protects only a small status snapshot. Filesystem work, lock waits,
//! and child processes always run on a dedicated worker thread.

use std::sync::{Arc, Mutex};
use std::thread::Builder;

use serde::Serialize;

#[cfg(target_os = "linux")]
use crate::command_install::Installer;

/// The last completed result, or a result being refreshed in the background.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SetupReport {
    pub pending: bool,
    pub notices: Vec<SetupNotice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    Conflict,
    Repair,
    Path,
    Shadowed,
    Failed,
    /// The setup record could not be saved. The installation itself may have succeeded.
    Diagnostic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SetupNotice {
    pub kind: NoticeKind,
    pub message: String,
    pub detail: Option<String>,
    /// The existing file's identity. Replacement is refused if that file changes.
    pub replacement: Option<String>,
}

impl SetupReport {
    pub fn failed(message: String) -> Self {
        Self {
            pending: false,
            notices: vec![SetupNotice {
                kind: NoticeKind::Failed,
                message,
                detail: None,
                replacement: None,
            }],
        }
    }
}

type Operation = dyn Fn(Option<&str>) -> SetupReport + Send + Sync;

/// Remembers setup results without making the interface wait for installation.
pub struct TerminalCommand {
    report: Arc<Mutex<SetupReport>>,
    operation: Arc<Operation>,
}

impl TerminalCommand {
    pub fn start() -> Self {
        Self::with_operation(Arc::new(install_detected))
    }

    fn with_operation(operation: Arc<Operation>) -> Self {
        let setup = Self {
            report: Arc::new(Mutex::new(SetupReport::default())),
            operation,
        };
        setup.install(None);
        setup
    }

    pub fn status(&self) -> SetupReport {
        self.report
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Starts one worker, or returns the current state if one is already running.
    pub fn install(&self, replacement: Option<String>) -> SetupReport {
        {
            let mut report = self.report.lock().unwrap_or_else(|e| e.into_inner());
            if report.pending {
                return report.clone();
            }
            report.pending = true;
        }
        let report = self.report.clone();
        let operation = self.operation.clone();
        if let Err(problem) = Builder::new()
            .name("terminal-command-setup".to_string())
            .spawn(move || {
                let completed = operation(replacement.as_deref());
                *report.lock().unwrap_or_else(|e| e.into_inner()) = completed;
            })
        {
            *self.report.lock().unwrap_or_else(|e| e.into_inner()) = SetupReport::failed(format!(
                "Could not start command setup in the background: {problem}"
            ));
        }
        self.status()
    }
}

fn install_detected(replacement: Option<&str>) -> SetupReport {
    #[cfg(target_os = "linux")]
    {
        match Installer::detect() {
            Ok(Some(installer)) => installer.report(replacement),
            Ok(None) => SetupReport::default(),
            Err(problem) => Installer::diagnostics(SetupReport::failed(problem.to_string())),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = replacement;
        SetupReport::default()
    }
}

#[cfg(test)]
mod tests;
