//! `service.json`: how a client on this machine finds the running service.
//!
//! The service writes its port, its token and its process identifier here
//! once it is listening. Only the person running the service may read the
//! file, which is what makes the token a secret: every client must show it
//! before the service will talk to it.
//!
//! The file is replaced in one step, so a client never reads half of it. A
//! file left behind by a service that died is harmless: a client finds
//! nobody answering at that port with that token, and starts a new service.

use std::fs::{create_dir_all, read_to_string, remove_file, write};
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceFileError {
    #[error("Could not use {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("{path} is not valid: {source}")]
    Invalid {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// What a client needs to reach the service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceFile {
    pub port: u16,
    pub token: String,
    /// The service's process, so `teryon stop` can wait for it to end.
    pub process: u32,
}

impl ServiceFile {
    pub fn path(data_folder: &Path) -> PathBuf {
        data_folder.join("service.json")
    }

    /// Reads the file. Nothing when there is none.
    pub fn read(data_folder: &Path) -> Result<Option<Self>, ServiceFileError> {
        let path = Self::path(data_folder);
        match read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|source| ServiceFileError::Invalid { path, source }),
            Err(problem) if problem.kind() == ErrorKind::NotFound => Ok(None),
            Err(source) => Err(ServiceFileError::Io { path, source }),
        }
    }

    /// Writes the file, readable by this person only, replacing any earlier one in one step.
    pub fn write(&self, data_folder: &Path) -> Result<(), ServiceFileError> {
        create_dir_all(data_folder).map_err(|source| ServiceFileError::Io {
            path: data_folder.to_path_buf(),
            source,
        })?;
        let path = Self::path(data_folder);
        let fresh = data_folder.join("service.json.new");
        let text =
            serde_json::to_string_pretty(self).map_err(|source| ServiceFileError::Invalid {
                path: path.clone(),
                source,
            })?;
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| ServiceFileError::Io { path, source }
        };
        // Created empty and private first, so the token is never readable by others, even briefly.
        write(&fresh, "").map_err(io(&fresh))?;
        private(&fresh).map_err(io(&fresh))?;
        write(&fresh, text).map_err(io(&fresh))?;
        std::fs::rename(&fresh, &path).map_err(io(&path))
    }

    /// Removes the file, but only when it still describes this service.
    pub fn remove_if_mine(&self, data_folder: &Path) {
        if let Ok(Some(current)) = Self::read(data_folder) {
            if current == *self {
                let _ = remove_file(Self::path(data_folder));
            }
        }
    }
}

/// Makes a file readable and writable by its owner only.
///
/// On Windows the person's application data folder is already private to
/// them, so there is nothing to add.
#[cfg(unix)]
fn private(path: &Path) -> Result<(), IoError> {
    use std::fs::{set_permissions, Permissions};
    use std::os::unix::fs::PermissionsExt;
    set_permissions(path, Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn private(_path: &Path) -> Result<(), IoError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;
    use tempfile::tempdir;

    use super::*;

    fn sample() -> ServiceFile {
        ServiceFile {
            port: 7317,
            token: "secret".to_string(),
            process: 42,
        }
    }

    /// Writes the sample into a fresh folder and reads it back.
    fn round_trip() -> Option<ServiceFile> {
        let folder = tempdir().unwrap();
        sample().write(&folder.path().join("teryon")).unwrap();
        ServiceFile::read(&folder.path().join("teryon")).unwrap()
    }

    #[cfg(unix)]
    fn mode() -> u32 {
        use std::os::unix::fs::PermissionsExt;
        let folder = tempdir().unwrap();
        sample().write(folder.path()).unwrap();
        std::fs::metadata(ServiceFile::path(folder.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    }

    /// Writes the sample, lets another service replace it, then asks the sample's service to remove it.
    fn removed_by(other: Option<ServiceFile>) -> bool {
        let folder = tempdir().unwrap();
        sample().write(folder.path()).unwrap();
        if let Some(other) = other {
            other.write(folder.path()).unwrap();
        }
        sample().remove_if_mine(folder.path());
        !ServiceFile::path(folder.path()).exists()
    }

    fn another() -> Option<ServiceFile> {
        Some(ServiceFile {
            port: 7318,
            ..sample()
        })
    }

    lets_expect! {
        expect(round_trip()) as the_file_read_back to equal(Some(sample()))

        expect(ServiceFile::read(folder.path()).unwrap()) as reading_without_a_file {
            let folder = tempdir().unwrap();
            to find_nothing { be_none }
        }

        expect(removed_by(other)) as removing_the_file {
            when(other = None) as it_still_describes_this_service to remove_it { be_true }
            when(other = another()) as another_service_replaced_it to leave_it { be_false }
        }
    }

    #[cfg(unix)]
    lets_expect! {
        expect(mode()) as the_file_permissions {
            to let_only_the_owner_read_it { equal(0o600) }
        }
    }
}
