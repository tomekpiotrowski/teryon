//! A persistent command for AppImages, published without truncating a running executable.
//!
//! A lock serializes launches, versioned copies keep old executables intact,
//! and only an exact link into our installation directory counts as managed.

use std::env::{current_exe, split_paths, var_os};
use std::fs::{
    canonicalize, create_dir_all, metadata, read_link, rename, symlink_metadata, File, OpenOptions,
    Permissions, TryLockError,
};
use std::io::{copy, Error as IoError, ErrorKind, Read, Seek, SeekFrom};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use directories::BaseDirs;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use semver::{Error as VersionError, Version};
use serde_json::to_writer;
use tempfile::{tempdir_in, tempfile, NamedTempFile};
use teryon_core::paths::Folders;
use thiserror::Error;

use crate::terminal_command::{NoticeKind, SetupNotice, SetupReport};

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("Could not {action} at {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        source: IoError,
    },
    #[error("Could not find your home folder to install the terminal command.")]
    NoHome,
    #[error("Teryon's release version is invalid: {0}")]
    Version(#[from] VersionError),
    #[error("The copied command could not run independently of the AppImage: {0}")]
    Unusable(String),
    #[error("The existing command changed during setup. Check again before replacing it.")]
    Changed,
}

/// Attach the operation and destination to every filesystem failure.
trait IoContext<T> {
    fn at(self, action: &'static str, path: &Path) -> Result<T, InstallError>;
}

impl<T> IoContext<T> for Result<T, IoError> {
    fn at(self, action: &'static str, path: &Path) -> Result<T, InstallError> {
        self.map_err(|source| InstallError::Io {
            action,
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Stop a version probe that does not finish.
const VERSION_CHECK: Duration = Duration::from_secs(2);

/// Probes one install may run: a newer copy, the copy being replaced, and the staged file.
const VERSION_CHECKS: u32 = 3;

/// Copy the executable out of the AppImage and sync it to disk.
///
/// The bundled command is tens of megabytes inside a compressed AppImage, and
/// that read is much slower than the tests' tiny scripts. Two minutes covers
/// the copy and the disk sync on a slow mount.
const PUBLISH_BUDGET: Duration = Duration::from_secs(120);

/// Wait for one other install to finish, then report what it left behind.
///
/// A shorter wait makes a second window show a busy failure while the first
/// launch is still copying. The bound still ends a wait on a stuck holder, so
/// setup does not stay pending without a notice.
const LOCK_WAIT: Duration =
    Duration::from_secs(PUBLISH_BUDGET.as_secs() + VERSION_CHECK.as_secs() * VERSION_CHECKS as u64);

/// All inputs are explicit so installation tests never change the person's setup.
#[derive(Clone)]
pub struct Installer {
    source: PathBuf,
    root: PathBuf,
    bin: PathBuf,
    version: Version,
    search: Vec<PathBuf>,
    shell: PathBuf,
    lock_wait: Duration,
}

impl Installer {
    /// AppImage sets both variables; inherited variables must also match this executable.
    pub fn detect() -> Result<Option<Self>, InstallError> {
        let (Some(image), Some(appdir)) = (var_os("APPIMAGE"), var_os("APPDIR")) else {
            return Ok(None);
        };
        let executable =
            current_exe().at("locate the running AppImage", Path::new("/proc/self/exe"))?;
        if !is_appimage(&executable, Path::new(&image), Path::new(&appdir)) {
            return Ok(None);
        }
        let base = BaseDirs::new().ok_or(InstallError::NoHome)?;
        Ok(Some(Self {
            source: executable.with_file_name("teryon"),
            root: base.data_dir().join("teryon/commands"),
            bin: base.home_dir().join(".local/bin"),
            version: Version::parse(env!("CARGO_PKG_VERSION"))?,
            search: var_os("PATH")
                .map(|path| split_paths(&path).collect())
                .unwrap_or_default(),
            shell: var_os("SHELL").map(PathBuf::from).unwrap_or_default(),
            lock_wait: LOCK_WAIT,
        }))
    }

    pub fn report(&self, replacement: Option<&str>) -> SetupReport {
        let report = match self.install(replacement) {
            Ok(report) => report,
            Err(problem) => SetupReport::failed(problem.to_string()),
        };
        Self::diagnostics(report)
    }

    /// Save the latest completed result in the same logs directory as the service.
    ///
    /// A record that cannot be saved is reported on its own. Saving it is not
    /// part of publishing the command, so a retry of the installation cannot help,
    /// and a successful install must not be shown as failed.
    pub fn diagnostics(report: SetupReport) -> SetupReport {
        let saved = match var_os("TERYON_HOME")
            .map(|home| Folders::inside(Path::new(&home)))
            .map(Ok)
            .unwrap_or_else(Folders::usual)
        {
            Ok(folders) => save_report(&folders.logs, &report).map_err(|error| error.to_string()),
            Err(problem) => Err(problem.to_string()),
        };
        Self::note_record(report, saved)
    }

    fn note_record(mut report: SetupReport, saved: Result<(), String>) -> SetupReport {
        if let Err(problem) = saved {
            let failed = report
                .notices
                .iter()
                .any(|notice| notice.kind == NoticeKind::Failed);
            let message = if failed {
                format!("Could not save the setup record: {problem}")
            } else {
                format!(
                    "The terminal command setup finished, but its record could not be saved: {problem}"
                )
            };
            report.notices.push(SetupNotice {
                kind: NoticeKind::Diagnostic,
                message,
                detail: None,
                replacement: None,
            });
        }
        report
    }

    #[cfg(test)]
    fn record_at(report: SetupReport, logs: &Path) -> SetupReport {
        let saved = save_report(logs, &report).map_err(|error| error.to_string());
        Self::note_record(report, saved)
    }

    fn install(&self, replacement: Option<&str>) -> Result<SetupReport, InstallError> {
        create_dir_all(&self.root).at("create the installation folder", &self.root)?;
        let lock_path = self.root.join("install.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .at("open the installation lock", &lock_path)?;
        acquire_lock(&lock, self.lock_wait)
            .at("wait for another installation to finish", &lock_path)?;
        create_dir_all(&self.bin).at("create the command folder", &self.bin)?;
        let command = self.bin.join("teryon");
        let before = existing(&command).at("inspect the existing command", &command)?;
        let managed = read_link(&command)
            .ok()
            .and_then(|target| self.managed_version(&target));
        if let Some(file) = &before {
            if managed.is_none() && (file.directory || replacement != Some(file.identity.as_str()))
            {
                return Ok(SetupReport {
                    pending: false,
                    notices: vec![SetupNotice {
                        kind: NoticeKind::Conflict,
                        message: format!(
                            "{} already exists. Teryon left it unchanged.{}",
                            command.display(),
                            if file.directory {
                                " Move this directory before trying again."
                            } else {
                                " Replace it to use the AppImage's command and automatic updates."
                            }
                        ),
                        detail: None,
                        replacement: (!file.directory).then(|| file.identity.clone()),
                    }],
                });
            }
        }
        // A healthy newer release stays selected. Recovery to an older release
        // requires an explicit action tied to the link the person was shown.
        let mut version = self.version.clone();
        if let Some(newer) = managed.filter(|v| v.cmp_precedence(&self.version).is_gt()) {
            let target = self.root.join(newer.to_string()).join("teryon");
            match verify(&target, &newer) {
                Ok(()) => version = newer,
                Err(problem) => {
                    let identity = before.as_ref().map(|file| file.identity.as_str());
                    if replacement.is_none() || replacement != identity {
                        return Ok(SetupReport { pending: false, notices: vec![SetupNotice {
                            kind: NoticeKind::Repair,
                            message: format!("The installed command for version {newer} is unavailable. Open that release or a newer one to repair it, or explicitly use this AppImage's version {}.", self.version),
                            detail: Some(problem.to_string()),
                            replacement: identity.map(str::to_string),
                        }] });
                    }
                }
            }
        }
        let target = self.root.join(version.to_string()).join("teryon");
        if version == self.version {
            self.publish_executable(&target)?;
        }
        if read_link(&command).ok().as_ref() != Some(&target) {
            publish_link(&command, &target, before.as_ref())?;
        }
        Ok(self.path_report())
    }

    fn managed_version(&self, target: &Path) -> Option<Version> {
        let relative = target.strip_prefix(&self.root).ok()?;
        let mut parts = relative.iter();
        let version = Version::parse(parts.next()?.to_str()?).ok()?;
        if parts.next()? != "teryon" || parts.next().is_some() {
            return None;
        }
        (target == self.root.join(version.to_string()).join("teryon")).then_some(version)
    }

    fn publish_executable(&self, target: &Path) -> Result<(), InstallError> {
        if verify(target, &self.version).is_ok() {
            return Ok(());
        }
        let folder = target
            .parent()
            .ok_or_else(|| InstallError::Unusable(target.display().to_string()))?;
        // Never follow a version-directory link while repairing a managed copy.
        if symlink_metadata(folder).is_ok_and(|meta| !meta.is_dir()) {
            return Err(InstallError::Unusable(format!(
                "{} is not an installation directory",
                folder.display()
            )));
        }
        create_dir_all(folder).at("create the version folder", folder)?;
        let mut staged =
            NamedTempFile::new_in(folder).at("create the temporary executable", folder)?;
        let mut source = File::open(&self.source).at("open the bundled command", &self.source)?;
        copy(&mut source, staged.as_file_mut()).at("copy the bundled command", target)?;
        staged
            .as_file()
            .set_permissions(Permissions::from_mode(0o755))
            .at("make the copied command executable", target)?;
        staged
            .as_file()
            .sync_all()
            .at("save the copied command", target)?;
        let staged = staged.into_temp_path();
        verify(&staged, &self.version)?;
        // Replacing the directory entry preserves any running executable inode.
        // Replacing a symlink here does not touch its target.
        staged
            .persist(target)
            .map_err(|error| error.error)
            .at("publish the repaired command", target)?;
        File::open(folder)
            .and_then(|file| file.sync_all())
            .at("save the version folder", folder)?;
        Ok(())
    }

    fn path_report(&self) -> SetupReport {
        let mut notices = Vec::new();
        let command = self.bin.join("teryon");
        let found = self
            .search
            .iter()
            // AppRun prepends the bundled command for its own subprocesses.
            // It says nothing about what an ordinary terminal would find.
            .filter(|folder| {
                !self
                    .source
                    .parent()
                    .is_some_and(|bundled| same_folder(folder, bundled))
            })
            .map(|folder| folder.join("teryon"))
            .find(|path| executable_file(path));
        if let Some(found) = found {
            if canonicalize(&found).ok() != canonicalize(&command).ok() {
                notices.push(SetupNotice {
                    kind: NoticeKind::Shadowed,
                    message: format!("Another terminal command takes precedence: {}. Put ~/.local/bin before its folder in your shell's search path.", found.display()),
                    detail: None,
                    replacement: None,
                });
            }
        }
        if !self
            .search
            .iter()
            .any(|folder| same_folder(folder, &self.bin))
            || !notices.is_empty()
        {
            let (message, detail) = path_instructions(&self.shell);
            notices.push(SetupNotice {
                kind: NoticeKind::Path,
                message,
                detail,
                replacement: None,
            });
        }
        SetupReport {
            pending: false,
            notices,
        }
    }
}

/// Wait out one install, then let the caller report the command that remains.
fn acquire_lock(file: &File, wait: Duration) -> Result<(), IoError> {
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                sleep(Duration::from_millis(10))
            }
            Err(TryLockError::WouldBlock) => {
                return Err(IoError::new(
                    ErrorKind::TimedOut,
                    "another Teryon installation is still busy; retry installation",
                ))
            }
            Err(TryLockError::Error(problem)) => return Err(problem),
        }
    }
}

/// Compare again after staging, immediately before publishing a link.
fn publish_link(
    command: &Path,
    target: &Path,
    before: Option<&Existing>,
) -> Result<(), InstallError> {
    let bin = command
        .parent()
        .ok_or_else(|| InstallError::Unusable(command.display().to_string()))?;
    let staging = tempdir_in(bin).at("stage the command link", bin)?;
    let link = staging.path().join("teryon");
    symlink(target, &link).at("create the staged command link", &link)?;
    if existing(command)
        .at("recheck the command before replacement", command)?
        .as_ref()
        != before
    {
        return Err(InstallError::Changed);
    }
    if before.is_none() {
        symlink(target, command).at("publish the command link", command)?;
    } else {
        rename(&link, command).at("replace the command link", command)?;
    }
    File::open(bin)
        .and_then(|file| file.sync_all())
        .at("save the command folder", bin)?;
    Ok(())
}

/// A bounded, atomic local diagnostic record also identifies completed setup.
fn save_report(logs: &Path, report: &SetupReport) -> Result<(), InstallError> {
    create_dir_all(logs).at("create the diagnostic folder", logs)?;
    let path = logs.join("terminal-command.json");
    let mut staged = NamedTempFile::new_in(logs).at("stage the setup diagnostic", &path)?;
    to_writer(staged.as_file_mut(), report)
        .map_err(IoError::other)
        .at("write the setup diagnostic", &path)?;
    staged
        .as_file()
        .sync_all()
        .at("save the setup diagnostic", &path)?;
    staged
        .persist(&path)
        .map_err(|error| error.error)
        .at("publish the setup diagnostic", &path)?;
    Ok(())
}

fn is_appimage(executable: &Path, image: &Path, appdir: &Path) -> bool {
    if !image.is_absolute() || !image.is_file() || !appdir.is_absolute() {
        return false;
    }
    match (canonicalize(executable), canonicalize(appdir)) {
        (Ok(executable), Ok(appdir)) => executable.starts_with(appdir),
        _ => false,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Existing {
    identity: String,
    directory: bool,
}

fn existing(path: &Path) -> Result<Option<Existing>, IoError> {
    let meta = match symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(Existing {
        identity: format!(
            "{}:{}:{}:{}:{}:{}:{}:{}",
            meta.dev(),
            meta.ino(),
            meta.mode(),
            meta.size(),
            meta.mtime(),
            meta.mtime_nsec(),
            meta.ctime(),
            meta.ctime_nsec()
        ),
        directory: meta.is_dir(),
    }))
}

fn executable_file(path: &Path) -> bool {
    metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

fn same_folder(left: &Path, right: &Path) -> bool {
    match (canonicalize(left), canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Check outside the AppImage's library environment before exposing the command.
fn verify(path: &Path, version: &Version) -> Result<(), InstallError> {
    if !symlink_metadata(path)
        .at("inspect the copied command", path)?
        .is_file()
        || !executable_file(path)
    {
        return Err(InstallError::Unusable(format!(
            "{} is not an executable file",
            path.display()
        )));
    }
    let output = version_output(path).at("run the command version check", path)?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout).trim() != format!("teryon {version}")
    {
        return Err(InstallError::Unusable(format!(
            "{} did not report version {version}",
            path.display()
        )));
    }
    Ok(())
}

/// A concurrent process spawn can briefly inherit a writable handle before exec closes it.
///
/// Linux then refuses execution with ETXTBSY even though our handle is closed.
/// Retry that transient error for at most 200 ms; other errors remain immediate.
fn version_output(path: &Path) -> Result<Output, IoError> {
    let mut output = tempfile()?;
    let mut command = Command::new(path);
    command
        .arg("--version")
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_PRELOAD")
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(Stdio::null())
        .process_group(0);
    let mut retries = 0;
    let mut child = loop {
        match command.spawn() {
            Err(error) if error.kind() == ErrorKind::ExecutableFileBusy && retries < 20 => {
                retries += 1;
                sleep(Duration::from_millis(10));
            }
            result => break result?,
        }
    };
    let deadline = Instant::now() + VERSION_CHECK;
    let status = (|| loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(IoError::new(
                ErrorKind::TimedOut,
                "the command did not finish its version check within the time limit",
            ));
        }
        sleep(Duration::from_millis(10));
    })();
    // Kill the whole probe group, including children of a damaged shell script.
    let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
    let _ = child.wait();
    let status = status?;
    output.seek(SeekFrom::Start(0))?;
    let mut stdout = Vec::new();
    output.take(4096).read_to_end(&mut stdout)?;
    Ok(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

fn path_instructions(shell: &Path) -> (String, Option<String>) {
    let instruction = match shell.file_name().and_then(|name| name.to_str()) {
        Some("bash") => Some((
            "Add this line to ~/.bashrc, then open a new terminal.",
            "export PATH=\"$HOME/.local/bin:$PATH\"",
        )),
        Some("zsh") => Some((
            "Add this line to ~/.zshrc, then open a new terminal.",
            "export PATH=\"$HOME/.local/bin:$PATH\"",
        )),
        Some("fish") => Some((
            "Run this once in fish, then open a new terminal.",
            "fish_add_path --prepend --move \"$HOME/.local/bin\"",
        )),
        _ => None,
    };
    match instruction {
        Some((message, command)) => (format!("Teryon's command is installed. {message} Start Teryon from that new terminal so it inherits the updated environment."), Some(command.to_string())),
        None => ("Teryon's command is installed. Add ~/.local/bin to the beginning of your shell's search path, then open a new terminal and start Teryon from it. You can also run ~/.local/bin/teryon directly.".to_string(), None),
    }
}

#[cfg(test)]
mod tests;
