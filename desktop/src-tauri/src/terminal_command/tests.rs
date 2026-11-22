use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, sync_channel};
use std::thread::{sleep, spawn};
use std::time::{Duration, Instant};

use lets_expect::lets_expect;

use super::*;

fn finished(setup: &TerminalCommand) -> SetupReport {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let report = setup.status();
        if !report.pending {
            return report;
        }
        assert!(Instant::now() < deadline, "setup worker did not finish");
        sleep(Duration::from_millis(1));
    }
}

fn background() -> (bool, bool, usize) {
    let (release, gate) = sync_channel(1);
    let gate = Mutex::new(gate);
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let operation: Arc<Operation> = Arc::new(move |_| {
        called.fetch_add(1, Ordering::SeqCst);
        gate.lock().unwrap().recv().unwrap();
        SetupReport::failed("completed failure".to_string())
    });
    let (created, receive) = channel();
    let launcher = spawn(move || {
        created
            .send(TerminalCommand::with_operation(operation))
            .unwrap()
    });
    // A synchronous implementation must fail the assertion instead of hanging this test.
    let setup = receive.recv_timeout(Duration::from_secs(1));
    let returned = setup.is_ok();
    let accessible = setup
        .as_ref()
        .is_ok_and(|setup| setup.status().pending && setup.install(None).pending);
    release.send(()).unwrap();
    if let Ok(setup) = setup {
        assert_eq!(finished(&setup).notices[0].message, "completed failure");
        assert_eq!(setup.status(), setup.status());
    }
    launcher.join().unwrap();
    (returned, accessible, calls.load(Ordering::SeqCst))
}

fn no_installer() -> bool {
    let setup = TerminalCommand::with_operation(Arc::new(|_| SetupReport::default()));
    finished(&setup) == SetupReport::default() && {
        setup.install(Some("ignored on unsupported platforms".to_string()));
        finished(&setup) == SetupReport::default()
    }
}

fn retry() -> (String, bool) {
    let (sent, received) = channel();
    let setup = TerminalCommand::with_operation(Arc::new(move |replacement| {
        sent.send(replacement.map(str::to_string)).unwrap();
        if replacement.is_some() {
            SetupReport::default()
        } else {
            SetupReport::failed("first failure".to_string())
        }
    }));
    let first = finished(&setup).notices[0].message.clone();
    setup.install(Some("observed-file".to_string()));
    let completed = finished(&setup);
    (
        first,
        completed == SetupReport::default()
            && received.recv().unwrap().is_none()
            && received.recv().unwrap().as_deref() == Some("observed-file"),
    )
}

lets_expect! {
    expect(install_detected(None)) { to do_nothing_outside_an_appimage { equal(SetupReport::default()) } }
    expect(background()) { to keep_startup_and_status_available_while_work_is_blocked { equal((true, true, 1)) } }
    expect(no_installer()) { to support_packages_and_platforms_without_an_installer { be_true } }
    expect(retry()) { to retain_failure_then_publish_the_retried_result { equal(("first failure".to_string(), true)) } }
}
