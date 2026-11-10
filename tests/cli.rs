//! CLI modes, the one-shot capture flow, and the daemon/trigger dispatch, with a fake portal
//! and recording launchers.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gnome_annotation::capture::{CaptureError, CaptureOutcome, ScreenshotPortal};
use gnome_annotation::cli::{CliCommand, parse, run_with};

struct FakePortal(Result<CaptureOutcome, CaptureError>);

impl ScreenshotPortal for FakePortal {
    fn capture(&self) -> Result<CaptureOutcome, CaptureError> {
        self.0.clone()
    }
}

fn recorder() -> (
    Arc<Mutex<Vec<PathBuf>>>,
    Box<dyn FnMut(PathBuf) -> Result<(), String>>,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    (
        seen,
        Box::new(move |path| {
            sink.lock().unwrap().push(path);
            Ok(())
        }),
    )
}

fn unused_daemon() -> Result<(), String> {
    panic!("daemon mode is not expected in this test")
}

fn unused_trigger() -> Result<(), String> {
    panic!("trigger mode is not expected in this test")
}

#[test]
fn parsing_covers_every_mode() {
    assert_eq!(parse(&[]).unwrap(), CliCommand::Trigger);
    assert_eq!(parse(&["--help".into()]).unwrap(), CliCommand::Help);
    assert_eq!(
        parse(&["--editor".into(), "/tmp/a.png".into()]).unwrap(),
        CliCommand::Editor(PathBuf::from("/tmp/a.png"))
    );
    assert_eq!(parse(&["--capture".into()]).unwrap(), CliCommand::Capture);
    assert_eq!(parse(&["--daemon".into()]).unwrap(), CliCommand::Daemon);
    assert_eq!(parse(&["--trigger".into()]).unwrap(), CliCommand::Trigger);
    assert!(parse(&["--editor".into()]).is_err());
    assert!(parse(&["--bogus".into()]).is_err());
}

#[test]
fn editor_mode_opens_the_explicit_image() {
    let (seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Cancelled));
    let code = run_with(
        ["--editor".into(), "/tmp/a.png".into()],
        &portal,
        launch,
        unused_daemon,
        unused_trigger,
    );
    assert_eq!(code, 0);
    assert_eq!(*seen.lock().unwrap(), [PathBuf::from("/tmp/a.png")]);
}

#[test]
fn capture_mode_opens_the_portal_file() {
    let (seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/shot.png"))));
    let code = run_with(
        ["--capture".into()],
        &portal,
        launch,
        unused_daemon,
        unused_trigger,
    );
    assert_eq!(code, 0);
    assert_eq!(*seen.lock().unwrap(), [PathBuf::from("/tmp/shot.png")]);
}

#[test]
fn a_cancelled_capture_is_silent_and_opens_nothing() {
    let (seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Cancelled));
    let code = run_with(
        ["--capture".into()],
        &portal,
        launch,
        unused_daemon,
        unused_trigger,
    );
    assert_eq!(code, 0);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_portal_error_exits_non_zero_and_opens_nothing() {
    let (seen, launch) = recorder();
    let portal = FakePortal(Err(CaptureError("no screenshot portal".into())));
    let code = run_with(
        ["--capture".into()],
        &portal,
        launch,
        unused_daemon,
        unused_trigger,
    );
    assert_eq!(code, 1);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_bad_command_line_exits_two() {
    let (_seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Cancelled));
    assert_eq!(
        run_with(
            ["--bogus".into()],
            &portal,
            launch,
            unused_daemon,
            unused_trigger
        ),
        2
    );
}

#[test]
fn no_arguments_triggers_the_daemon() {
    let (_seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Cancelled));
    let mut triggered = false;
    let code = run_with(Vec::<String>::new(), &portal, launch, unused_daemon, || {
        triggered = true;
        Ok(())
    });
    assert_eq!(code, 0);
    assert!(triggered);
}

#[test]
fn daemon_mode_runs_the_daemon() {
    let (_seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Cancelled));
    let mut started = false;
    let code = run_with(
        ["--daemon".into()],
        &portal,
        launch,
        || {
            started = true;
            Ok(())
        },
        unused_trigger,
    );
    assert_eq!(code, 0);
    assert!(started);
}

#[test]
fn a_trigger_failure_exits_non_zero() {
    let (_seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Cancelled));
    let code = run_with(Vec::<String>::new(), &portal, launch, unused_daemon, || {
        Err("cannot reach the daemon".into())
    });
    assert_eq!(code, 1);
}
