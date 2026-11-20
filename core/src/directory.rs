//! Checks a directory someone named, and lists the directories around it.
//!
//! A project is a directory on this machine. On your own machine any
//! directory you can read can be one, so the only question is whether it is
//! there. The answer is its real location, without `..` or symbolic links, so
//! the same directory named two ways is recognised as one.
//!
//! Choosing that directory is a matter of seeing the ones next to it. What
//! was typed may be a whole directory or only the start of a name, and the
//! listing says which, so the screen can narrow the list as the letters arrive.

use std::fs::{canonicalize, metadata, read_dir, DirEntry};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DirectoryError {
    #[error("The directory does not exist.")]
    Missing,
    #[error("That is a file, not a directory.")]
    NotADirectory,
    #[error("The directory could not be read.")]
    Unreadable,
    #[error("Could not find the home folder.")]
    NoHome,
}

/// One directory's children, for choosing where a project lives.
///
/// `remainder` is the part of what was typed that is not a directory yet.
/// The screen narrows the list with it, and only asks again when the
/// directory itself changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DirectoryListing {
    /// The real directory whose children these are.
    pub directory: String,
    /// That directory's parent. Nothing at the top of the filesystem.
    pub parent: Option<String>,
    /// The child directories, by name, in case-insensitive order.
    ///
    /// Names starting with `.` are included. The screen hides them until
    /// what is typed starts with `.`, so a home folder is not a wall of
    /// configuration.
    pub children: Vec<String>,
    /// What was typed after `directory`. Empty when the typed path is this directory.
    pub remainder: String,
}

/// The real location of a directory, given as text that may have spaces around it.
///
/// `~` and a relative path start at the home directory, the same reading
/// [`browse`] uses, so the directory that gets added is the one the picker showed.
/// An empty path names nothing.
pub fn existing_directory(text: &str) -> Result<PathBuf, DirectoryError> {
    let home = BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    resolve_directory(text, home.as_deref())
}

/// [`existing_directory`] against a chosen home directory, so tests need no real one.
fn resolve_directory(text: &str, home: Option<&Path>) -> Result<PathBuf, DirectoryError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(DirectoryError::Missing);
    }
    let (directory, remainder) = locate(trimmed, home)?;
    if remainder.is_empty() {
        Ok(directory)
    } else {
        Err(DirectoryError::Missing)
    }
}

/// The directories inside what was typed, for the new-project screen.
///
/// An empty path, `~`, or a path that is not absolute starts at the home
/// directory. A path that does not exist yet lists the nearest directory
/// that does, and [`DirectoryListing::remainder`] is the rest.
pub fn browse(text: &str) -> Result<DirectoryListing, DirectoryError> {
    let home = BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    browse_with(text, home.as_deref())
}

/// [`browse`] against a chosen home directory, so tests need no real one.
fn browse_with(text: &str, home: Option<&Path>) -> Result<DirectoryListing, DirectoryError> {
    let (directory, remainder) = locate(text.trim(), home)?;
    list_children(&directory, &remainder.join("/"))
}

/// The existing directory a typed path lands in, and the parts that are not there yet.
///
/// The walk starts from the home side. `..` then steps back out of a name that
/// does not exist, so `~/nope/../projects` is `~/projects`. A directory that is
/// there is remembered at its real location, and a later `..` leaves that place.
fn locate(text: &str, home: Option<&Path>) -> Result<(PathBuf, Vec<String>), DirectoryError> {
    let expanded = expand(text.trim(), home)?;
    let mut cursor = PathBuf::new();
    let mut remainder = Vec::new();
    for component in expanded.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => cursor.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if remainder.pop().is_none() {
                    cursor.pop();
                }
            }
            Component::Normal(name) => {
                if !remainder.is_empty() {
                    remainder.push(name.to_string_lossy().into_owned());
                    continue;
                }
                let next = cursor.join(name);
                match kind_of(&next) {
                    Kind::Directory => {
                        cursor = match canonicalize(&next) {
                            Ok(real) => real,
                            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                                return Err(DirectoryError::Unreadable);
                            }
                            Err(_) => return Err(DirectoryError::Missing),
                        };
                    }
                    Kind::File => return Err(DirectoryError::NotADirectory),
                    Kind::Unreadable => return Err(DirectoryError::Unreadable),
                    Kind::Missing => remainder.push(name.to_string_lossy().into_owned()),
                }
            }
        }
    }
    if cursor.as_os_str().is_empty() {
        return Err(DirectoryError::Missing);
    }
    Ok((cursor, remainder))
}

/// What a path is, for deciding whether it can be listed.
enum Kind {
    Directory,
    File,
    Missing,
    Unreadable,
}

/// Whether `path` is a directory, a file, missing, or there but unreadable.
///
/// A link to a directory counts as a directory, so it can be stepped into.
fn kind_of(path: &Path) -> Kind {
    match path.metadata() {
        Ok(meta) if meta.is_dir() => Kind::Directory,
        Ok(_) => Kind::File,
        Err(error) if error.kind() == ErrorKind::PermissionDenied => Kind::Unreadable,
        Err(_) => Kind::Missing,
    }
}

/// `~` and a relative path are taken from `home`. An absolute path is itself.
fn expand(text: &str, home: Option<&Path>) -> Result<PathBuf, DirectoryError> {
    if text.is_empty() || text == "~" {
        return home.map(Path::to_path_buf).ok_or(DirectoryError::NoHome);
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        let home = home.ok_or(DirectoryError::NoHome)?;
        return Ok(if rest.is_empty() {
            home.to_path_buf()
        } else {
            home.join(rest)
        });
    }
    let path = PathBuf::from(text);
    if path.is_absolute() {
        return Ok(path);
    }
    home.map(|folder| folder.join(path))
        .ok_or(DirectoryError::NoHome)
}

/// The child directories of `directory`, skipping names that are not text.
fn list_children(directory: &Path, remainder: &str) -> Result<DirectoryListing, DirectoryError> {
    let mut children = Vec::new();
    let entries = read_dir(directory).map_err(|_| DirectoryError::Unreadable)?;
    for entry in entries.flatten() {
        let Some(name) = directory_name(&entry) else {
            continue;
        };
        children.push(name);
    }
    children.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then(left.cmp(right))
    });
    let parent = directory
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .filter(|path| !path.is_empty());
    Ok(DirectoryListing {
        directory: directory.to_string_lossy().into_owned(),
        parent,
        children,
        remainder: remainder.to_string(),
    })
}

/// The entry's name when it is a directory. A link to a directory counts.
///
/// `DirEntry::metadata` describes the link itself, which is not a directory,
/// so the path is asked instead and the link is followed.
fn directory_name(entry: &DirEntry) -> Option<String> {
    if !metadata(entry.path()).is_ok_and(|meta| meta.is_dir()) {
        return None;
    }
    entry.file_name().into_string().ok()
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir, write};

    use lets_expect::lets_expect;
    use tempfile::{tempdir, TempDir};

    use super::*;

    /// A folder holding the directories a person might step through.
    struct Tree {
        root: TempDir,
    }

    const PROJECTS: &[&str] = &["afolder", "Bfolder", "payments", "website"];

    impl Tree {
        fn new() -> Self {
            let root = tempdir().expect("temporary folder");
            create_dir(root.path().join("projects")).expect("projects");
            for name in PROJECTS {
                create_dir(root.path().join("projects").join(name)).expect(name);
            }
            create_dir(root.path().join(".hidden")).expect("hidden");
            write(root.path().join("notes.txt"), "text").expect("file");
            #[cfg(unix)]
            std::os::unix::fs::symlink(root.path().join("projects"), root.path().join("linked"))
                .expect("link");
            Self { root }
        }

        /// The listing of `relative`, which is empty for the home folder itself.
        fn listed(&self, relative: &str, children: &[&str], remainder: &str) -> DirectoryListing {
            let path = if relative.is_empty() {
                self.root.path().to_path_buf()
            } else {
                self.root.path().join(relative)
            };
            let directory = canonicalize(path).expect("real path");
            DirectoryListing {
                directory: directory.to_string_lossy().into_owned(),
                parent: directory
                    .parent()
                    .map(|path| path.to_string_lossy().into_owned()),
                children: children.iter().map(|name| (*name).to_string()).collect(),
                remainder: remainder.to_string(),
            }
        }
    }

    fn home_children() -> &'static [&'static str] {
        #[cfg(unix)]
        {
            &[".hidden", "linked", "projects"]
        }
        #[cfg(not(unix))]
        {
            &[".hidden", "projects"]
        }
    }

    /// A directory with no permissions is refused, except where the account can read it anyway.
    fn unreadable_is_refused() -> bool {
        let root = tempdir().expect("temporary folder");
        let locked = root.path().join("locked");
        create_dir(&locked).expect("locked");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o0)).expect("lock");
            let refused = browse_with(locked.to_str().expect("path"), Some(root.path()));
            let readable = read_dir(&locked).is_ok();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
                .expect("unlock");
            readable || refused == Err(DirectoryError::Unreadable)
        }
        #[cfg(not(unix))]
        {
            let _ = locked;
            true
        }
    }

    /// `..` after a link leaves the directory the link points at, not the link's own folder.
    fn dot_dot_follows_a_link() -> bool {
        #[cfg(not(unix))]
        {
            return true;
        }
        #[cfg(unix)]
        {
            let tree = Tree::new();
            std::os::unix::fs::symlink(
                tree.root.path().join("projects/payments"),
                tree.root.path().join("jump"),
            )
            .expect("jump");
            browse_with("jump/..", Some(tree.root.path())).unwrap()
                == tree.listed("projects", PROJECTS, "")
        }
    }

    /// A link is listed as the directory it points at.
    fn link_lists_its_target() -> bool {
        #[cfg(not(unix))]
        {
            return true;
        }
        #[cfg(unix)]
        {
            let tree = Tree::new();
            browse_with("linked/payments", Some(tree.root.path())).unwrap()
                == tree.listed("projects/payments", &[], "")
        }
    }

    /// The top of the filesystem has nothing above it.
    fn the_top_has_no_parent() -> bool {
        #[cfg(not(unix))]
        {
            return true;
        }
        #[cfg(unix)]
        {
            browse_with("/", None).unwrap().parent.is_none()
        }
    }

    lets_expect! {
        expect(existing_directory(&directory)) {
            let folders = Tree::new();

            when a_directory {
                let directory = folders.root.path().join("projects").to_string_lossy().into_owned();
                to be_accepted_at_its_real_location {
                    equal(Ok(canonicalize(folders.root.path().join("projects")).unwrap()))
                }
            }

            when a_directory_with_spaces_around_it {
                let directory = format!("  {}  ", folders.root.path().join("projects").to_string_lossy());
                to be_accepted { equal(Ok(canonicalize(folders.root.path().join("projects")).unwrap())) }
            }

            when a_directory_reached_through_dot_dot {
                let directory = folders.root.path().join("projects/../projects").to_string_lossy().into_owned();
                to be_accepted_at_its_real_location {
                    equal(Ok(canonicalize(folders.root.path().join("projects")).unwrap()))
                }
            }

            when a_missing_directory {
                let directory = folders.root.path().join("nothing").to_string_lossy().into_owned();
                to be_refused_as_missing { equal(Err(DirectoryError::Missing)) }
            }

            when a_file {
                let directory = folders.root.path().join("notes.txt").to_string_lossy().into_owned();
                to be_refused_as_a_file { equal(Err(DirectoryError::NotADirectory)) }
            }

            when nothing_given {
                let directory = "   ".to_string();
                to be_refused_as_missing { equal(Err(DirectoryError::Missing)) }
            }
        }

        expect(browse_with(&text, Some(tree.root.path()))) {
            let tree = Tree::new();

            when nothing_was_typed {
                let text = "".to_string();
                to list_the_home_folder { equal(Ok(tree.listed("", home_children(), ""))) }
            }

            when just_a_tilde {
                let text = "~".to_string();
                to list_the_home_folder { equal(Ok(tree.listed("", home_children(), ""))) }
            }

            when a_tilde_and_a_slash {
                let text = "~/".to_string();
                to list_the_home_folder { equal(Ok(tree.listed("", home_children(), ""))) }
            }

            when a_tilde_with_spaces_around_it {
                let text = "  ~  ".to_string();
                to list_the_home_folder { equal(Ok(tree.listed("", home_children(), ""))) }
            }

            when a_directory_under_the_tilde {
                let text = "~/projects/payments".to_string();
                to list_that_directory { equal(Ok(tree.listed("projects/payments", &[], ""))) }
            }

            when a_relative_directory {
                let text = "projects/website".to_string();
                to list_that_directory { equal(Ok(tree.listed("projects/website", &[], ""))) }
            }

            when a_relative_directory_with_spaces_around_it {
                let text = "  projects/payments  ".to_string();
                to list_that_directory { equal(Ok(tree.listed("projects/payments", &[], ""))) }
            }

            when a_directory_with_a_trailing_slash {
                let text = "projects/payments/".to_string();
                to list_that_directory { equal(Ok(tree.listed("projects/payments", &[], ""))) }
            }

            when a_directory_reached_through_dot_dot {
                let text = "projects/payments/../website".to_string();
                to list_the_directory_arrived_at { equal(Ok(tree.listed("projects/website", &[], ""))) }
            }

            when dot_dot_steps_out_of_a_missing_name {
                let text = "~/nope/../projects/pa".to_string();
                to list_the_directory_before_the_unfinished_name {
                    equal(Ok(tree.listed("projects", PROJECTS, "pa")))
                }
            }

            when dot_dot_steps_out_of_a_missing_name_under_a_directory {
                let text = "projects/nope/../website".to_string();
                to list_the_directory_arrived_at { equal(Ok(tree.listed("projects/website", &[], ""))) }
            }

            when the_start_of_a_name {
                let text = "projects/pay".to_string();
                to list_the_directory_and_keep_what_was_typed {
                    equal(Ok(tree.listed("projects", PROJECTS, "pay")))
                }
            }

            when more_than_one_missing_part {
                let text = "projects/nope/deeper".to_string();
                to keep_the_whole_missing_part {
                    equal(Ok(tree.listed("projects", PROJECTS, "nope/deeper")))
                }
            }

            when a_name_that_is_not_in_the_home_folder {
                let text = "afolder".to_string();
                to list_the_home_folder_and_keep_the_name {
                    equal(Ok(tree.listed("", home_children(), "afolder")))
                }
            }

            when an_absolute_directory {
                let text = tree.root.path().join("projects").to_string_lossy().into_owned();
                to list_that_directory { equal(Ok(tree.listed("projects", PROJECTS, ""))) }
            }

            when a_file {
                let text = "notes.txt".to_string();
                to be_refused_as_a_file { equal(Err(DirectoryError::NotADirectory)) }
            }

            when a_file_with_a_directory_named_after_it {
                let text = "notes.txt/inside".to_string();
                to be_refused_as_a_file { equal(Err(DirectoryError::NotADirectory)) }
            }
        }

        expect(resolve_directory(&text, Some(tree.root.path()))) {
            let tree = Tree::new();

            when a_directory_under_a_tilde {
                let text = "~/projects/payments".to_string();
                to be_that_directory {
                    equal(Ok(canonicalize(tree.root.path().join("projects/payments")).unwrap()))
                }
            }

            when a_relative_directory {
                let text = "projects/website".to_string();
                to be_that_directory {
                    equal(Ok(canonicalize(tree.root.path().join("projects/website")).unwrap()))
                }
            }

            when a_tilde_through_a_missing_name {
                let text = "  ~/nope/../projects  ".to_string();
                to be_the_directory_arrived_at {
                    equal(Ok(canonicalize(tree.root.path().join("projects")).unwrap()))
                }
            }

            when just_a_tilde {
                let text = "~".to_string();
                to be_the_home_folder { equal(Ok(canonicalize(tree.root.path()).unwrap())) }
            }

            when an_unfinished_name {
                let text = "~/projects/pay".to_string();
                to be_refused_as_missing { equal(Err(DirectoryError::Missing)) }
            }

            when a_missing_directory {
                let text = "~/nope".to_string();
                to be_refused_as_missing { equal(Err(DirectoryError::Missing)) }
            }

            when a_file {
                let text = "~/notes.txt".to_string();
                to be_refused_as_a_file { equal(Err(DirectoryError::NotADirectory)) }
            }

            when nothing_given {
                let text = "   ".to_string();
                to be_refused_as_missing { equal(Err(DirectoryError::Missing)) }
            }
        }

        expect(browse_with(&text, None)) {
            let tree = Tree::new();

            when nothing_was_typed {
                let text = "".to_string();
                to be_refused_without_a_home_folder { equal(Err(DirectoryError::NoHome)) }
            }

            when a_tilde {
                let text = "~".to_string();
                to be_refused_without_a_home_folder { equal(Err(DirectoryError::NoHome)) }
            }

            when a_relative_directory {
                let text = "projects".to_string();
                to be_refused_without_a_home_folder { equal(Err(DirectoryError::NoHome)) }
            }

            when an_absolute_directory {
                let text = tree.root.path().join("projects/payments").to_string_lossy().into_owned();
                to list_that_directory { equal(Ok(tree.listed("projects/payments", &[], ""))) }
            }
        }

        expect(browse(&text)) {
            let tree = Tree::new();

            when an_absolute_directory {
                let text = tree.root.path().join("projects/afolder").to_string_lossy().into_owned();
                to list_that_directory { equal(Ok(tree.listed("projects/afolder", &[], ""))) }
            }
        }

        expect(unreadable_is_refused()) as a_directory_that_cannot_be_read to be_true
        expect(link_lists_its_target()) as a_link_to_a_directory to be_true
        expect(dot_dot_follows_a_link()) as dot_dot_after_a_link to be_true
        expect(the_top_has_no_parent()) as the_top_of_the_filesystem to be_true
    }
}
