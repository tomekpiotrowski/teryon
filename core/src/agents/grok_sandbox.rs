//! The sandbox Grok works in, kept as a profile in Grok's own `sandbox.toml`.
//!
//! Nobody sits at Grok's prompt in Teryon, and a command Grok wants approved
//! ends its turn. So Grok runs without asking, and the operating system holds
//! it in instead: an author may write in the project folder, the temporary
//! folders, and the folders named in settings, such as the caches a build
//! tool fills when it fetches a dependency. A reviewer gets Grok's
//! `read-only` profile.
//!
//! Grok takes extra writable folders only from a named profile in its
//! `sandbox.toml`, not from its command line. Teryon keeps one profile per
//! project there, named after the project's path, and rewrites it before
//! each author turn so it follows the settings. The rest of the file is left
//! as the person wrote it.
//!
//! A conversation keeps the profile it began with: Grok refuses to resume
//! one under another, and restores it when none is named. So only a turn
//! that starts a conversation names its profile, and a profile is never
//! removed, as an older conversation may still be resumed under it.

use std::env::var_os;
use std::fs::{create_dir_all, read_to_string, rename, write};
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::id as process_id;
use std::sync::{Mutex, PoisonError};

use directories::BaseDirs;
use sha2::{Digest, Sha256};
use thiserror::Error;
use toml_edit::{value, Array, DocumentMut, Item, Table, TomlError};
use tracing::warn;

use crate::settings::GrokSettings;

/// The profile an author falls back to when its own cannot be written.
const FALLBACK_PROFILE: &str = "workspace";

/// The profile a reviewer works in.
pub const REVIEWER_PROFILE: &str = "read-only";

/// Keeps two turns of this process from writing `sandbox.toml` at once.
static WRITING: Mutex<()> = Mutex::new(());

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("Could not find Grok's folder, as there is no home folder.")]
    NoHomeFolder,
    #[error("Could not read {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("{path} is not valid: {source}")]
    Invalid {
        path: PathBuf,
        #[source]
        source: TomlError,
    },
    #[error("{path} has a `profiles` entry that is not a table.")]
    ProfilesNotATable { path: PathBuf },
    #[error("Could not write {path}: {source}")]
    Unwritable {
        path: PathBuf,
        #[source]
        source: IoError,
    },
}

/// Where Grok's profiles live and what an author may write besides its project.
#[derive(Clone, Debug)]
pub struct GrokSandbox {
    /// Grok's folder, or nothing when there is no home folder to find it in.
    home: Option<PathBuf>,
    /// Folders outside the project an author may write, with `~` expanded.
    writable: Vec<PathBuf>,
}

impl GrokSandbox {
    /// Reads the settings, finding Grok's folder the way Grok does when they name none.
    pub fn new(settings: &GrokSettings) -> Self {
        let home_folder = BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
        let home = settings
            .home
            .clone()
            .or_else(|| var_os("GROK_HOME").map(PathBuf::from))
            .or_else(|| home_folder.as_ref().map(|home| home.join(".grok")));
        Self {
            home,
            writable: settings
                .writable
                .iter()
                .filter_map(|folder| expand_home(folder, home_folder.as_deref()))
                .collect(),
        }
    }

    /// The profile an author's turn in `project` works in, written so Grok finds it.
    ///
    /// `outside` names folders beyond the project that the turn must write,
    /// such as the repository's history when the project is a worktree of
    /// another repository. When the profile cannot be written, the author still gets Grok's
    /// `workspace` profile: fewer folders to write in, but no approvals.
    pub fn author_profile(&self, project: &Path, outside: &[PathBuf]) -> String {
        let name = profile_name(project);
        let mut folders = self.writable.clone();
        folders.extend(outside.iter().cloned());
        match self.write_profile(&name, project, &folders) {
            Ok(()) => name,
            Err(problem) => {
                warn!("Grok gets its workspace sandbox instead of Teryon's: {problem}");
                FALLBACK_PROFILE.to_string()
            }
        }
    }

    fn write_profile(
        &self,
        name: &str,
        project: &Path,
        folders: &[PathBuf],
    ) -> Result<(), SandboxError> {
        let home = self.home.as_ref().ok_or(SandboxError::NoHomeFolder)?;
        let path = home.join("sandbox.toml");
        let _writing = WRITING.lock().unwrap_or_else(PoisonError::into_inner);
        let text = match read_to_string(&path) {
            Ok(text) => text,
            Err(problem) if problem.kind() == ErrorKind::NotFound => String::new(),
            Err(source) => return Err(SandboxError::Unreadable { path, source }),
        };
        let updated =
            with_profile(&text, name, project, folders).map_err(|problem| match problem {
                ProfileProblem::Invalid(source) => SandboxError::Invalid {
                    path: path.clone(),
                    source,
                },
                ProfileProblem::ProfilesNotATable => {
                    SandboxError::ProfilesNotATable { path: path.clone() }
                }
            })?;
        if updated == text {
            return Ok(());
        }
        // Written beside the file and moved over it, so Grok starting at the
        // same moment reads either the old file or the new one, never half.
        let unwritable = |source| SandboxError::Unwritable {
            path: path.clone(),
            source,
        };
        create_dir_all(home).map_err(unwritable)?;
        let draft = home.join(format!("sandbox.toml.teryon-{}", process_id()));
        write(&draft, updated).map_err(unwritable)?;
        rename(&draft, &path).map_err(unwritable)
    }
}

/// The name of a project's profile: `teryon-` and a short digest of its path.
///
/// A path, unlike a folder name, cannot belong to two projects at once.
pub fn profile_name(project: &Path) -> String {
    let digest = Sha256::digest(project.to_string_lossy().as_bytes());
    let short: String = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("teryon-{short}")
}

/// Why a profile could not be put into `sandbox.toml`.
#[derive(Debug)]
enum ProfileProblem {
    Invalid(TomlError),
    ProfilesNotATable,
}

/// `sandbox.toml` with the profile `name` replaced, everything else kept as it was.
fn with_profile(
    text: &str,
    name: &str,
    project: &Path,
    folders: &[PathBuf],
) -> Result<String, ProfileProblem> {
    let mut document: DocumentMut = text.parse().map_err(ProfileProblem::Invalid)?;
    let profiles = document
        .entry("profiles")
        .or_insert_with(|| {
            let mut profiles = Table::new();
            profiles.set_implicit(true);
            Item::Table(profiles)
        })
        .as_table_mut()
        .ok_or(ProfileProblem::ProfilesNotATable)?;

    let mut profile = Table::new();
    profile.decor_mut().set_prefix(format!(
        "\n# Written by Teryon for {}. Teryon rewrites it before each Grok turn there.\n",
        project.display()
    ));
    profile["extends"] = value(FALLBACK_PROFILE);
    let folders: Array = folders
        .iter()
        .map(|folder| folder.to_string_lossy().into_owned())
        .collect();
    profile["read_write"] = value(folders);
    profiles.insert(name, Item::Table(profile));
    Ok(document.to_string())
}

/// A folder from the settings, with a leading `~` made the home folder.
///
/// Grok does not expand `~` in a profile. A `~` folder is left out when
/// there is no home folder.
fn expand_home(folder: &str, home: Option<&Path>) -> Option<PathBuf> {
    match folder.strip_prefix('~') {
        Some("") => home.map(Path::to_path_buf),
        Some(rest) if rest.starts_with('/') => home.map(|home| home.join(&rest[1..])),
        _ => Some(PathBuf::from(folder)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{read_to_string, write};

    use lets_expect::lets_expect;
    use tempfile::{tempdir, TempDir};

    use super::*;

    const PROJECT: &str = "/work/app";

    fn folders(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    fn edited(text: &str, list: &[&str]) -> Result<String, String> {
        with_profile(text, "teryon-abc", Path::new(PROJECT), &folders(list))
            .map_err(|problem| format!("{problem:?}"))
    }

    /// Writes a project's profile into a fresh Grok folder holding `existing`, and reads the file back.
    fn written(existing: Option<&str>) -> (String, String) {
        let home = tempdir().unwrap();
        if let Some(existing) = existing {
            write(home.path().join("sandbox.toml"), existing).unwrap();
        }
        let sandbox = sandbox_in(&home, &["~/.cargo", "/opt/cache"]);
        let profile = sandbox.author_profile(Path::new(PROJECT), &folders(&["/work/.git"]));
        let text = read_to_string(home.path().join("sandbox.toml")).unwrap_or_default();
        (profile, text)
    }

    fn sandbox_in(home: &TempDir, writable: &[&str]) -> GrokSandbox {
        GrokSandbox::new(&GrokSettings {
            program: "grok".to_string(),
            home: Some(home.path().to_path_buf()),
            writable: writable.iter().map(ToString::to_string).collect(),
            models: None,
        })
    }

    const TEMPLATE: &str = "\n# Written by Teryon for /work/app. Teryon rewrites it before each Grok turn there.\n[profiles.teryon-abc]\nextends = \"workspace\"\n";

    lets_expect! {
        // requirement: an author's profile extends Grok's workspace profile with the folders it may write
        expect(edited(text, &["/home/me/.cargo", "/work/.git"])) {
            let text = "";

            when(text = "") as there_is_no_file_yet to write_only_the_profile {
                equal(Ok(format!("{TEMPLATE}read_write = [\"/home/me/.cargo\", \"/work/.git\"]\n")))
            }

            // requirement: the person's own profiles and comments in Grok's sandbox file are kept
            when(text = "# mine\n[profiles.tight]\nextends = \"strict\"\n") as the_person_has_profiles to keep_them {
                equal(Ok(format!("# mine\n[profiles.tight]\nextends = \"strict\"\n{TEMPLATE}read_write = [\"/home/me/.cargo\", \"/work/.git\"]\n")))
            }

            // requirement: a project's profile is replaced, not added to, so it follows the settings
            when(text = "[profiles.teryon-abc]\nextends = \"workspace\"\nread_write = [\"/old\"]\n") as the_profile_exists to replace_it {
                equal(Ok(format!("{TEMPLATE}read_write = [\"/home/me/.cargo\", \"/work/.git\"]\n")))
            }

            when(text = "[profiles") as the_file_is_not_toml to be_refused { be_err }
            when(text = "profiles = 1\n") as profiles_is_not_a_table to be_refused { equal(Err("ProfilesNotATable".to_string())) }
        }

        expect(written(existing)) {
            let existing: Option<&str> = None;

            // requirement: the profile is written to Grok's folder and named after the project
            to name_the_project_profile {
                have(0.clone()) equal(profile_name(Path::new(PROJECT))),
                have(1.contains("\"/opt/cache\", \"/work/.git\"")) be_true,
                have(1.contains("/.cargo\"")) be_true,
                have(1.contains("\"~")) be_false
            }

            // requirement: when Teryon cannot write its profile, the author still runs sandboxed, in Grok's workspace profile
            when(existing = Some("[profiles")) as the_file_is_broken to fall_back_to_the_workspace_profile {
                have(0.clone()) equal("workspace".to_string()),
                have(1.clone()) equal("[profiles".to_string())
            }
        }

        expect(profile_name(Path::new(path))) {
            let path = PROJECT;

            to be_short_and_marked_as_teryons {
                have(starts_with("teryon-")) be_true,
                have(len()) equal(19)
            }
            when(path = "/work/other") as another_project to differ { not_equal(profile_name(Path::new(PROJECT))) }
        }

        expect(expand_home(folder, Some(Path::new("/home/me")))) {
            let folder = "~/.cargo";

            to put_the_home_folder_first { equal(Some(PathBuf::from("/home/me/.cargo"))) }
            when(folder = "~") as the_home_folder_itself to be_it { equal(Some(PathBuf::from("/home/me"))) }
            when(folder = "/opt/cache") as a_full_path to be_kept { equal(Some(PathBuf::from("/opt/cache"))) }
            when(folder = "~other/x") as another_persons_home to be_kept_as_written { equal(Some(PathBuf::from("~other/x"))) }
        }

        expect(expand_home("~/.cargo", None)) as no_home_folder {
            to leave_the_folder_out { equal(None) }
        }
    }
}
