//! Filesystem tests use isolated homes and tiny executables with real version output.

use std::fs::{read, read_to_string, remove_file, set_permissions, write};
use std::sync::{Arc, Barrier};
use std::thread::spawn;

use lets_expect::lets_expect;
use serde_json::{from_slice, json, Value};
use tempfile::{tempdir, TempDir};

use super::*;

struct Home(TempDir);

impl Home {
    fn new() -> Self {
        Self(tempdir().unwrap())
    }

    fn installer(&self, version: &str) -> Installer {
        let source = self.0.path().join(format!("bundle-{version}"));
        write(
            &source,
            format!("#!/bin/sh\nprintf 'teryon {version}\\n'\n"),
        )
        .unwrap();
        let bin = self.0.path().join(".local/bin");
        Installer {
            source,
            root: self.0.path().join("data/teryon/commands"),
            bin: bin.clone(),
            version: Version::parse(version).unwrap(),
            search: vec![bin],
            shell: PathBuf::from("/bin/bash"),
            lock_wait: LOCK_WAIT,
        }
    }
}

fn first_install() -> (bool, bool, bool, bool) {
    let home = Home::new();
    let installer = home.installer("1.0.0");
    let report = installer.install(None).unwrap();
    let link = installer.bin.join("teryon");
    let target = read_link(&link).unwrap();
    let contents = read(&installer.source).unwrap();
    remove_file(&installer.source).unwrap();
    (
        report.notices.is_empty(),
        target == installer.root.join("1.0.0/teryon"),
        read(&link).unwrap() == contents,
        verify(&target, &installer.version).is_ok(),
    )
}

fn repeated_install() -> bool {
    let home = Home::new();
    let installer = home.installer("1.0.0");
    installer.install(None).unwrap();
    let link = installer.bin.join("teryon");
    let before = (existing(&link).unwrap(), metadata(&link).unwrap().ino());
    installer.install(None).unwrap();
    before == (existing(&link).unwrap(), metadata(&link).unwrap().ino())
}

fn updated(from: &str, to: &str) -> (String, bool) {
    let home = Home::new();
    let older = home.installer(from);
    older.install(None).unwrap();
    let old_target = read_link(older.bin.join("teryon")).unwrap();
    let old_inode = metadata(&old_target).unwrap().ino();
    let next = home.installer(to);
    next.install(None).unwrap();
    let target = read_link(next.bin.join("teryon")).unwrap();
    (
        next.managed_version(&target).unwrap().to_string(),
        metadata(&old_target).unwrap().ino() == old_inode,
    )
}

fn conflict(kind: &str, replace: bool, changed: bool) -> (bool, bool, bool) {
    let home = Home::new();
    let installer = home.installer("1.0.0");
    create_dir_all(&installer.bin).unwrap();
    let command = installer.bin.join("teryon");
    let external = home.0.path().join("external");
    write(&external, "leave alone").unwrap();
    match kind {
        "file" => {
            write(&command, "original").unwrap();
        }
        "link" => symlink(&external, &command).unwrap(),
        "broken" => symlink(home.0.path().join("absent"), &command).unwrap(),
        "directory" => create_dir_all(&command).unwrap(),
        _ => unreachable!(),
    }
    let before = existing(&command).unwrap();
    let report = installer.install(None).unwrap();
    let notice = &report.notices[0];
    let preserved = existing(&command).unwrap() == before;
    if changed {
        remove_file(&command).unwrap();
        write(&command, "new installation since the notice").unwrap();
    }
    let report = if replace {
        installer.install(notice.replacement.as_deref()).unwrap()
    } else {
        report
    };
    (
        preserved,
        report.notices.is_empty(),
        read_to_string(&external).unwrap() == "leave alone"
            && (!changed
                || read_to_string(&command).unwrap() == "new installation since the notice"),
    )
}

fn failed_update(failure: &str) -> (bool, bool) {
    let home = Home::new();
    let old = home.installer("1.0.0");
    old.install(None).unwrap();
    let link = old.bin.join("teryon");
    let before = read_link(&link).unwrap();
    let next = home.installer("2.0.0");
    match failure {
        "missing" => remove_file(&next.source).unwrap(),
        "read" => {
            remove_file(&next.source).unwrap();
            create_dir_all(&next.source).unwrap();
        }
        "version" => write(&next.source, "#!/bin/sh\necho 'teryon 9.0.0'\n").unwrap(),
        "execute" => write(&next.source, "not an executable\n").unwrap(),
        "exit" => write(&next.source, "#!/bin/sh\nexit 1\n").unwrap(),
        "bin_permissions" => set_permissions(&next.bin, Permissions::from_mode(0o500)).unwrap(),
        "data_permissions" => set_permissions(&next.root, Permissions::from_mode(0o500)).unwrap(),
        _ => unreachable!(),
    }
    let failed = next.install(None).is_err();
    set_permissions(&next.bin, Permissions::from_mode(0o700)).unwrap();
    set_permissions(&next.root, Permissions::from_mode(0o700)).unwrap();
    (
        failed,
        read_link(&link).unwrap() == before && verify(&before, &old.version).is_ok(),
    )
}

fn concurrent() -> (bool, String) {
    let home = Home::new();
    let installers: Vec<_> = ["1.0.0", "3.0.0", "2.0.0", "3.0.0", "1.0.0"]
        .iter()
        .map(|version| home.installer(version))
        .collect();
    let barrier = Arc::new(Barrier::new(installers.len()));
    let handles: Vec<_> = installers
        .into_iter()
        .map(|installer| {
            let barrier = barrier.clone();
            spawn(move || {
                barrier.wait();
                installer.install(None).unwrap().notices.is_empty()
            })
        })
        .collect();
    let successful = handles.into_iter().all(|handle| handle.join().unwrap());
    let installer = home.installer("3.0.0");
    let version = installer
        .managed_version(&read_link(installer.bin.join("teryon")).unwrap())
        .unwrap();
    (successful, version.to_string())
}

fn search_path(case: &str) -> Vec<NoticeKind> {
    let home = Home::new();
    let mut installer = home.installer("1.0.0");
    let other = home.0.path().join("other");
    create_dir_all(&other).unwrap();
    let command = other.join("teryon");
    write(&command, "other").unwrap();
    set_permissions(&command, Permissions::from_mode(0o755)).unwrap();
    installer.search = match case {
        "bundle_prefix" | "bundle_only" => {
            let bundled = home.0.path().join("appdir/usr/bin");
            create_dir_all(&bundled).unwrap();
            let source = bundled.join("teryon");
            rename(&installer.source, &source).unwrap();
            set_permissions(&source, Permissions::from_mode(0o755)).unwrap();
            installer.source = source;
            if case == "bundle_prefix" {
                vec![bundled, installer.bin.clone()]
            } else {
                vec![bundled]
            }
        }
        "empty" => vec![],
        "shadowed" => vec![other, installer.bin.clone()],
        "shadowed_missing" => vec![other],
        "later" => vec![installer.bin.clone(), other],
        "nonexecutable" => {
            set_permissions(&command, Permissions::from_mode(0o644)).unwrap();
            vec![other, installer.bin.clone()]
        }
        "directory" => {
            remove_file(&command).unwrap();
            create_dir_all(&command).unwrap();
            vec![other, installer.bin.clone()]
        }
        "equivalent" => {
            create_dir_all(&installer.bin).unwrap();
            symlink(&installer.bin, home.0.path().join("alias")).unwrap();
            vec![home.0.path().join("alias")]
        }
        _ => unreachable!(),
    };
    installer
        .install(None)
        .unwrap()
        .notices
        .into_iter()
        .map(|notice| notice.kind)
        .collect()
}

fn ownership(target: &str) -> bool {
    let home = Home::new();
    let installer = home.installer("1.0.0");
    installer
        .managed_version(&installer.root.join(target))
        .is_some()
}

fn detection(case: &str) -> bool {
    let home = Home::new();
    let appdir = home.0.path().join("mounted");
    create_dir_all(&appdir).unwrap();
    let mut image = home.0.path().join("Teryon.AppImage");
    write(&image, "image").unwrap();
    let mut exe = appdir.join("teryon-desktop");
    write(&exe, "desktop").unwrap();
    match case {
        "outside" => {
            exe = home.0.path().join("installed-desktop");
            write(&exe, "other").unwrap();
        }
        "relative" => image = PathBuf::from("Teryon.AppImage"),
        "missing" => remove_file(&image).unwrap(),
        "empty" => image = PathBuf::new(),
        "valid" => (),
        _ => unreachable!(),
    }
    is_appimage(&exe, &image, &appdir)
}

fn broken_copy(kind: &str) -> bool {
    let home = Home::new();
    let installer = home.installer("1.0.0");
    installer.install(None).unwrap();
    let target = read_link(installer.bin.join("teryon")).unwrap();
    match kind {
        "missing" => remove_file(&target).unwrap(),
        "permissions" => set_permissions(&target, Permissions::from_mode(0o644)).unwrap(),
        "symlink" => {
            remove_file(&target).unwrap();
            symlink(&installer.source, &target).unwrap();
        }
        _ => unreachable!(),
    }
    installer.install(None).unwrap().notices.is_empty()
        && verify(&target, &installer.version).is_ok()
}

/// Hold a writable handle long enough to exercise Linux's execution refusal.
fn busy_executable(released: bool) -> bool {
    let home = Home::new();
    let installer = home.installer("1.0.0");
    installer.install(None).unwrap();
    let target = read_link(installer.bin.join("teryon")).unwrap();
    let writer = OpenOptions::new().write(true).open(&target).unwrap();
    if released {
        let release = spawn(move || {
            sleep(Duration::from_millis(30));
            drop(writer);
        });
        let checked = verify(&target, &installer.version).is_ok();
        release.join().unwrap();
        checked
    } else {
        let refused = matches!(verify(&target, &installer.version), Err(InstallError::Io { source: error, .. }) if error.kind() == ErrorKind::ExecutableFileBusy);
        drop(writer);
        refused
    }
}

lets_expect! {
    expect(busy_executable(released)) {
        let released = true;
        to tolerate_a_briefly_inherited_writable_handle { be_true }
        when(released = false) as a_persistently_busy_file to stop_retrying { be_true }
    }
    expect(first_install()) {
        to publish_a_persistent_executable_independent_of_the_bundle { equal((true, true, true, true)) }
    }
    expect(repeated_install()) { to leave_existing_inodes_unchanged { be_true } }
    expect(updated(from, to)) {
        let from = "1.0.0";
        let to = "2.0.0";
        to update_without_touching_the_old_copy { equal(("2.0.0".to_string(), true)) }
        when(from = "3.0.0") as an_older_appimage to keep_the_newer_command { equal(("3.0.0".to_string(), true)) }
        when(from = "1.9.0", to = "1.10.0") as numeric_versions to compare_numerically { equal(("1.10.0".to_string(), true)) }
        when(from = "2.0.0", to = "2.0.0-beta.1") as a_prerelease to keep_the_release { equal(("2.0.0".to_string(), true)) }
    }
    expect(conflict(kind, replace, changed)) {
        let kind = "file";
        let replace = false;
        let changed = false;
        to preserve_an_unmanaged_file { equal((true, false, true)) }
        when(kind = "link") as a_symlink to preserve_it { equal((true, false, true)) }
        when(kind = "broken") as a_dangling_link to preserve_it { equal((true, false, true)) }
        when(kind = "directory", replace = true) as a_directory to not_offer_replacement { equal((true, false, true)) }
        when(replace = true) as explicit_replacement to publish_the_command { equal((true, true, true)) }
        when(kind = "link", replace = true) as link_replacement to leave_its_target_alone { equal((true, true, true)) }
        when(kind = "broken", replace = true) as broken_link_replacement to publish_the_command { equal((true, true, true)) }
        when(replace = true, changed = true) as a_stale_approval to preserve_the_new_file { equal((true, false, true)) }
    }
    expect(failed_update(failure)) {
        let failure = "missing";
        to preserve_the_previous_command { equal((true, true)) }
        when(failure = "read") as read_failure to preserve_the_previous_command { equal((true, true)) }
        when(failure = "version") as wrong_version to preserve_the_previous_command { equal((true, true)) }
        when(failure = "execute") as an_unusable_copy to preserve_the_previous_command { equal((true, true)) }
        when(failure = "exit") as an_unsuccessful_command to preserve_the_previous_command { equal((true, true)) }
        when(failure = "bin_permissions") as an_unwritable_bin to preserve_the_previous_command { equal((true, true)) }
        when(failure = "data_permissions") as unwritable_data to preserve_the_previous_command { equal((true, true)) }
    }
    expect(concurrent()) { to serialize_launches_and_keep_the_newest_version { equal((true, "3.0.0".to_string())) } }
    expect(search_path(case)) {
        let case = "empty";
        to explain_the_missing_search_path { equal(vec![NoticeKind::Path]) }
        when(case = "bundle_prefix") as the_appimage_launcher_prefix to ignore_it { equal(vec![]) }
        when(case = "bundle_only") as only_the_launcher_prefix to still_report_the_missing_shell_path { equal(vec![NoticeKind::Path]) }
        when(case = "shadowed") as an_earlier_command to identify_it { equal(vec![NoticeKind::Shadowed, NoticeKind::Path]) }
        when(case = "shadowed_missing") as both_problems to explain_both { equal(vec![NoticeKind::Shadowed, NoticeKind::Path]) }
        when(case = "later") as a_later_command to leave_it_alone { equal(vec![]) }
        when(case = "nonexecutable") as a_nonexecutable_file to ignore_it { equal(vec![]) }
        when(case = "directory") as a_directory to ignore_it { equal(vec![]) }
        when(case = "equivalent") as a_link_to_the_same_folder to recognize_it { equal(vec![]) }
    }
    expect(ownership(target)) {
        let target = "1.0.0/teryon";
        to recognize_the_exact_managed_target { be_true }
        when(target = "1.0.0/other") as another_name to refuse_ownership { be_false }
        when(target = "garbage/teryon") as an_invalid_version to refuse_ownership { be_false }
        when(target = "1.0.0/../1.0.0/teryon") as traversal to refuse_ownership { be_false }
        when(target = "1.0.0/teryon/extra") as extra_components to refuse_ownership { be_false }
    }
    expect(detection(case)) {
        let case = "valid";
        to recognize_the_appimage_environment { be_true }
        when(case = "outside") as inherited_environment to skip_installation { be_false }
        when(case = "relative") as a_relative_image_path to skip_installation { be_false }
        when(case = "missing") as a_missing_image to skip_installation { be_false }
        when(case = "empty") as an_empty_variable to skip_installation { be_false }
    }
    expect(broken_copy(kind)) {
        let kind = "missing";
        to repair_a_missing_copy { be_true }
        when(kind = "permissions") as nonexecutable to repair_permissions { be_true }
        when(kind = "symlink") as a_replaced_copy to replace_the_link_without_following_it { be_true }
    }
    expect(path_instructions(Path::new(shell)).1) {
        let shell = "/bin/bash";
        to give_bash_instructions { equal(Some("export PATH=\"$HOME/.local/bin:$PATH\"".to_string())) }
        when(shell = "/bin/zsh") as zsh to give_zsh_instructions { equal(Some("export PATH=\"$HOME/.local/bin:$PATH\"".to_string())) }
        when(shell = "/usr/bin/fish") as fish to give_fish_instructions { equal(Some("fish_add_path --prepend --move \"$HOME/.local/bin\"".to_string())) }
        when(shell = "") as unknown to avoid_guessing_the_shell { equal(None) }
    }
}

fn damaged_newer(kind: &str, approve: bool) -> (bool, String) {
    let home = Home::new();
    let newer = home.installer("2.0.0");
    newer.install(None).unwrap();
    let target = newer.root.join("2.0.0/teryon");
    match kind {
        "missing" => remove_file(&target).unwrap(),
        "permissions" => set_permissions(&target, Permissions::from_mode(0o644)).unwrap(),
        "damaged" | "repaired" => write(&target, "broken executable").unwrap(),
        _ => unreachable!(),
    }
    let older = home.installer("1.0.0");
    let report = older.install(None).unwrap();
    let recovery = report.notices[0].kind == NoticeKind::Repair
        && report.notices[0].message.contains("2.0.0")
        && report.notices[0]
            .detail
            .as_ref()
            .unwrap()
            .contains("2.0.0/teryon");
    if kind == "repaired" {
        newer.install(None).unwrap();
    }
    if approve {
        assert!(older
            .install(report.notices[0].replacement.as_deref())
            .unwrap()
            .notices
            .is_empty());
    }
    let selected = older
        .managed_version(&read_link(older.bin.join("teryon")).unwrap())
        .unwrap();
    if approve {
        verify(
            &older.root.join(selected.to_string()).join("teryon"),
            &selected,
        )
        .unwrap();
    }
    (recovery, selected.to_string())
}

fn changed_before_swap(existed: bool) -> bool {
    let home = Home::new();
    let installer = home.installer("1.0.0");
    create_dir_all(&installer.bin).unwrap();
    let command = installer.bin.join("teryon");
    if existed {
        write(&command, "first command").unwrap();
    }
    let before = existing(&command).unwrap();
    write(&command, "installed during copying").unwrap();
    matches!(
        publish_link(&command, &installer.source, before.as_ref()),
        Err(InstallError::Changed)
    ) && read_to_string(&command).unwrap() == "installed during copying"
}

fn lock_timeout() -> bool {
    let home = Home::new();
    let mut installer = home.installer("1.0.0");
    installer.lock_wait = Duration::from_millis(200);
    create_dir_all(&installer.root).unwrap();
    let path = installer.root.join("install.lock");
    let held = File::create(&path).unwrap();
    held.lock().unwrap();
    let start = Instant::now();
    let problem = installer.install(None).unwrap_err();
    let message = problem.to_string();
    matches!(problem, InstallError::Io { source, .. } if source.kind() == ErrorKind::TimedOut)
        && start.elapsed() < Duration::from_secs(2)
        && message.contains(path.to_str().unwrap())
        && message.contains("wait for another installation")
        && !installer.bin.join("teryon").exists()
}

fn lock_budget() -> bool {
    let home = Home::new();
    home.installer("1.0.0").lock_wait == LOCK_WAIT
        && PUBLISH_BUDGET >= Duration::from_secs(120)
        && VERSION_CHECK == Duration::from_secs(2)
        && VERSION_CHECKS == 3
        && LOCK_WAIT == PUBLISH_BUDGET + VERSION_CHECK * VERSION_CHECKS
}

/// A peer that holds the lock longer than the old two-second cap still finishes.
fn slow_peer() -> bool {
    let home = Home::new();
    let peer = home.installer("1.0.0");
    create_dir_all(&peer.root).unwrap();
    let held = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(peer.root.join("install.lock"))
        .unwrap();
    held.lock().unwrap();
    let waiter = home.installer("1.0.0");
    let publish = spawn(move || {
        sleep(Duration::from_secs(3));
        let target = peer.root.join("1.0.0").join("teryon");
        peer.publish_executable(&target).unwrap();
        create_dir_all(&peer.bin).unwrap();
        publish_link(&peer.bin.join("teryon"), &target, None).unwrap();
        drop(held);
    });
    let start = Instant::now();
    let report = waiter.install(None).unwrap();
    publish.join().unwrap();
    let target = read_link(waiter.bin.join("teryon")).unwrap();
    report.notices.is_empty()
        && start.elapsed() >= Duration::from_secs(2)
        && start.elapsed() < LOCK_WAIT
        && verify(&target, &waiter.version).is_ok()
}

fn record_failure(install_failed: bool) -> bool {
    let home = Home::new();
    let logs = home.0.path().join("logs");
    write(&logs, "not a directory").unwrap();
    let report = if install_failed {
        SetupReport::failed(
            "Could not copy the bundled command at /example/teryon: permission denied".to_string(),
        )
    } else {
        SetupReport::default()
    };
    let report = Installer::record_at(report, &logs);
    let diagnostic = report
        .notices
        .iter()
        .find(|notice| notice.kind == NoticeKind::Diagnostic)
        .unwrap();
    let kinds: Vec<_> = report.notices.iter().map(|notice| &notice.kind).collect();
    let expected: Vec<&NoticeKind> = if install_failed {
        vec![&NoticeKind::Failed, &NoticeKind::Diagnostic]
    } else {
        vec![&NoticeKind::Diagnostic]
    };
    kinds == expected
        && diagnostic.replacement.is_none()
        && diagnostic.message.contains(logs.to_str().unwrap())
        && diagnostic.message.contains("create the diagnostic folder")
        && diagnostic.message.contains("setup finished") != install_failed
}

fn missing_home_record() -> bool {
    let report = Installer::note_record(
        SetupReport::default(),
        Err(
            "Could not find the home folder, so Teryon does not know where to keep its files."
                .to_string(),
        ),
    );
    report.notices.len() == 1
        && report.notices[0].kind == NoticeKind::Diagnostic
        && report.notices[0].replacement.is_none()
        && report.notices[0].message.contains("setup finished")
        && report.notices[0].message.contains("home folder")
}

fn version_timeout() -> bool {
    let home = Home::new();
    let installer = home.installer("1.0.0");
    let pid_file = home.0.path().join("probe-pid");
    write(
        &installer.source,
        format!(
            "#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n",
            pid_file.display()
        ),
    )
    .unwrap();
    set_permissions(&installer.source, Permissions::from_mode(0o755)).unwrap();
    let start = Instant::now();
    let problem = verify(&installer.source, &installer.version).unwrap_err();
    let pid = read_to_string(pid_file).unwrap();
    matches!(problem, InstallError::Io { source, .. } if source.kind() == ErrorKind::TimedOut)
        && start.elapsed() < Duration::from_secs(4)
        && !Path::new("/proc").join(pid.trim()).exists()
}

fn diagnostic_record() -> bool {
    let home = Home::new();
    let logs = home.0.path().join("logs");
    let failed = SetupReport::failed(
        "Could not copy the bundled command at /example/teryon: permission denied".to_string(),
    );
    save_report(&logs, &failed).unwrap();
    let path = logs.join("terminal-command.json");
    let first: Value = from_slice(&read(&path).unwrap()).unwrap();
    save_report(&logs, &SetupReport::default()).unwrap();
    let last: Value = from_slice(&read(&path).unwrap()).unwrap();
    first["notices"][0]["message"] == failed.notices[0].message
        && first["pending"] == false
        && last["notices"] == json!([])
}

fn contextual_failure() -> bool {
    let home = Home::new();
    let installer = home.installer("1.0.0");
    remove_file(&installer.source).unwrap();
    let message = installer.install(None).unwrap_err().to_string();
    message.contains("open the bundled command")
        && message.contains(installer.source.to_str().unwrap())
}

lets_expect! {
    expect(damaged_newer(kind, approve)) {
        let kind = "missing";
        let approve = false;
        to offer_recovery_without_silently_downgrading { equal((true, "2.0.0".to_string())) }
        when(kind = "damaged") as a_damaged_copy to offer_recovery { equal((true, "2.0.0".to_string())) }
        when(kind = "permissions") as a_nonexecutable_copy to offer_recovery { equal((true, "2.0.0".to_string())) }
        when(approve = true) as explicit_recovery to restore_a_working_command { equal((true, "1.0.0".to_string())) }
        when(kind = "damaged", approve = true) as damaged_recovery to restore_a_working_command { equal((true, "1.0.0".to_string())) }
        when(kind = "permissions", approve = true) as permissions_recovery to restore_a_working_command { equal((true, "1.0.0".to_string())) }
        when(kind = "repaired", approve = true) as already_repaired to keep_the_healthy_newer_copy { equal((true, "2.0.0".to_string())) }
    }
    expect(changed_before_swap(existed)) {
        let existed = true;
        to reject_a_file_changed_after_staging { be_true }
        when(existed = false) as a_new_file to preserve_it { be_true }
    }
    expect(lock_timeout()) { to bound_lock_waiting_and_identify_the_lock { be_true } }
    expect(lock_budget()) { to cover_one_full_installation { be_true } }
    expect(slow_peer()) { to report_success_after_the_other_install_finishes { be_true } }
    expect(record_failure(install_failed)) {
        let install_failed = false;
        to keep_a_finished_setup_separate_from_a_record_problem { be_true }
        when(install_failed = true) as an_installation_failure to keep_the_retryable_problem { be_true }
    }
    expect(missing_home_record()) { to report_a_missing_home_without_an_installation_failure { be_true } }
    expect(version_timeout()) { to kill_and_reap_a_stuck_version_probe { be_true } }
    expect(diagnostic_record()) { to keep_a_completed_bounded_diagnostic_record { be_true } }
    expect(contextual_failure()) { to identify_the_step_and_path { be_true } }
}
