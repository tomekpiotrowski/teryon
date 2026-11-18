//! Checks a directory someone named.
//!
//! A project is a directory on this machine. On your own machine any
//! directory you can read can be one, so the only question is whether it is
//! there. The answer is its real location, without `..` or symbolic links, so
//! the same directory named two ways is recognised as one.

use std::fs::canonicalize;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DirectoryError {
    #[error("The directory does not exist.")]
    Missing,
    #[error("That is a file, not a directory.")]
    NotADirectory,
}

/// The real location of a directory, given as text that may have spaces around it.
pub fn existing_directory(text: &str) -> Result<PathBuf, DirectoryError> {
    let real = canonicalize(text.trim()).map_err(|_| DirectoryError::Missing)?;
    if real.is_dir() {
        Ok(real)
    } else {
        Err(DirectoryError::NotADirectory)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir, write};

    use lets_expect::lets_expect;
    use tempfile::{tempdir, TempDir};

    use super::*;

    /// A folder holding a project directory and a file.
    struct Folders {
        root: TempDir,
    }

    impl Folders {
        fn new() -> Self {
            let root = tempdir().expect("temporary folder");
            create_dir(root.path().join("project")).expect("project folder");
            write(root.path().join("file.txt"), "text").expect("file");
            Self { root }
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
        expect(existing_directory(&directory)) {
            let folders = Folders::new();

            when a_directory {
                let directory = folders.path("project");
                to be_accepted_at_its_real_location { equal(Ok(folders.real("project"))) }
            }

            when a_directory_with_spaces_around_it {
                let directory = format!("  {}  ", folders.path("project"));
                to be_accepted { equal(Ok(folders.real("project"))) }
            }

            when a_directory_reached_through_dot_dot {
                let directory = folders.path("project/../project");
                to be_accepted_at_its_real_location { equal(Ok(folders.real("project"))) }
            }

            when a_missing_directory {
                let directory = folders.path("nothing");
                to be_refused_as_missing { equal(Err(DirectoryError::Missing)) }
            }

            when a_file {
                let directory = folders.path("file.txt");
                to be_refused_as_a_file { equal(Err(DirectoryError::NotADirectory)) }
            }

            when nothing_given {
                let directory = "   ".to_string();
                to be_refused_as_missing { equal(Err(DirectoryError::Missing)) }
            }
        }
    }
}
