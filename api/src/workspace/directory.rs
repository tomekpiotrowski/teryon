//! Decides whether a directory may become a project.
//!
//! An agent changes files in its project directory, so a project is only
//! accepted inside folders the person running Teryon has listed.
//!
//! Both sides are resolved to their real location first, so neither `..` nor a
//! symbolic link inside an allowed folder can lead out of it.

use std::fs::canonicalize;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DirectoryError {
    #[error("The directory does not exist.")]
    Missing,
    #[error("That path is not a directory.")]
    NotADirectory,
    #[error("The directory is not inside a folder Teryon is allowed to use.")]
    NotAllowed,
}

/// Returns the real location of `directory` when it may become a project.
pub fn resolve_directory(
    allowed_folders: &[PathBuf],
    directory: &str,
) -> Result<PathBuf, DirectoryError> {
    let resolved =
        canonicalize(Path::new(directory.trim())).map_err(|_| DirectoryError::Missing)?;
    if !resolved.is_dir() {
        return Err(DirectoryError::NotADirectory);
    }

    let allowed = allowed_folders
        .iter()
        .filter_map(|folder| canonicalize(folder).ok())
        .any(|folder| resolved.starts_with(folder));
    if allowed {
        Ok(resolved)
    } else {
        Err(DirectoryError::NotAllowed)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir, write};
    use std::os::unix::fs::symlink;

    use lets_expect::lets_expect;
    use tempfile::{tempdir, TempDir};

    use super::*;

    /// An allowed folder holding a project, next to a folder that is not allowed.
    struct Folders {
        root: TempDir,
    }

    impl Folders {
        fn new() -> Self {
            let root = tempdir().expect("temporary folder");
            create_dir(root.path().join("allowed")).expect("allowed folder");
            create_dir(root.path().join("allowed/project")).expect("project folder");
            create_dir(root.path().join("outside")).expect("outside folder");
            write(root.path().join("allowed/file.txt"), "text").expect("file");
            symlink(
                root.path().join("outside"),
                root.path().join("allowed/link"),
            )
            .expect("link");
            Self { root }
        }

        fn allowed(&self) -> Vec<PathBuf> {
            vec![self.root.path().join("allowed")]
        }

        fn path(&self, relative: &str) -> String {
            self.root
                .path()
                .join(relative)
                .to_string_lossy()
                .into_owned()
        }

        fn real(&self, relative: &str) -> PathBuf {
            canonicalize(self.root.path().join(relative)).expect("real path")
        }
    }

    lets_expect! {
        let folders = Folders::new();

        expect(resolve_directory(&allowed, &directory)) {
            let allowed = folders.allowed();

            when a_directory_inside_an_allowed_folder {
                let directory = folders.path("allowed/project");
                to be_accepted_at_its_real_location { equal(Ok(folders.real("allowed/project"))) }
            }

            when the_allowed_folder_itself {
                let directory = folders.path("allowed");
                to be_accepted { equal(Ok(folders.real("allowed"))) }
            }

            when surrounded_by_spaces {
                let directory = format!("  {}  ", folders.path("allowed/project"));
                to be_accepted { equal(Ok(folders.real("allowed/project"))) }
            }

            when the_directory_does_not_exist {
                let directory = folders.path("allowed/nothing");
                to be_refused_as_missing { equal(Err(DirectoryError::Missing)) }
            }

            when the_path_is_a_file {
                let directory = folders.path("allowed/file.txt");
                to be_refused_as_not_a_directory { equal(Err(DirectoryError::NotADirectory)) }
            }

            when the_directory_is_outside {
                let directory = folders.path("outside");
                to be_refused_as_not_allowed { equal(Err(DirectoryError::NotAllowed)) }
            }

            when dots_lead_outside {
                let directory = folders.path("allowed/project/../../outside");
                to be_refused_as_not_allowed { equal(Err(DirectoryError::NotAllowed)) }
            }

            when a_link_leads_outside {
                let directory = folders.path("allowed/link");
                to be_refused_as_not_allowed { equal(Err(DirectoryError::NotAllowed)) }
            }

            when no_folder_is_allowed {
                let allowed = Vec::new();
                let directory = folders.path("allowed/project");
                to be_refused_as_not_allowed { equal(Err(DirectoryError::NotAllowed)) }
            }

            when an_allowed_folder_does_not_exist {
                let allowed = Vec::from([PathBuf::from(folders.path("gone"))]);
                let directory = folders.path("allowed/project");
                to be_refused_as_not_allowed { equal(Err(DirectoryError::NotAllowed)) }
            }
        }
    }
}
