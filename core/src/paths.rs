//! Where Teryon keeps its own files on this machine.
//!
//! Each platform has its own place for a program's data, configuration and
//! logs, and Teryon follows it, so `projects.json` ends up where a person on
//! that platform would look for it. Tests and anyone who wants everything in
//! one place can name a single folder instead.

use std::path::{Path, PathBuf};

use directories::BaseDirs;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("Could not find the home folder, so Teryon does not know where to keep its files.")]
pub struct NoHomeFolder;

/// The folders Teryon writes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folders {
    /// `projects.json`, `service.json` and `servers.json`.
    pub data: PathBuf,
    /// `config.toml`.
    pub config: PathBuf,
    /// The service's log files.
    pub logs: PathBuf,
}

impl Folders {
    /// The platform's usual places.
    ///
    /// - Linux: `~/.local/share/teryon`, `~/.config/teryon`, `~/.local/state/teryon/logs`,
    ///   each moved by the matching `XDG_*_HOME` variable
    /// - macOS: `~/Library/Application Support/teryon` for all three, logs in `logs/`
    /// - Windows: `%APPDATA%\teryon` for all three, logs in `logs\`
    pub fn usual() -> Result<Self, NoHomeFolder> {
        let base = BaseDirs::new().ok_or(NoHomeFolder)?;
        let state = base.state_dir().unwrap_or(base.data_local_dir());
        Ok(Self {
            data: base.data_dir().join("teryon"),
            config: base.config_dir().join("teryon"),
            logs: state.join("teryon").join("logs"),
        })
    }

    /// Everything inside one folder.
    pub fn inside(folder: &Path) -> Self {
        Self {
            data: folder.join("data"),
            config: folder.join("config"),
            logs: folder.join("logs"),
        }
    }
}
