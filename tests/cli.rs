//! CLI modes and the one-shot capture flow, with a fake portal and a recording launcher.

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

#[test]
fn parsing_covers_editor_capture_and_help() {
    assert_eq!(parse(&[]).unwrap(), CliCommand::Help);
    assert_eq!(parse(&["--help".into()]).unwrap(), CliCommand::Help);
    assert_eq!(
        parse(&["--editor".into(), "/tmp/a.png".into()]).unwrap(),
        CliCommand::Editor(PathBuf::from("/tmp/a.png"))
    );
    assert_eq!(parse(&["--capture".into()]).unwrap(), CliCommand::Capture);
    assert!(parse(&["--editor".into()]).is_err());
    assert!(parse(&["--bogus".into()]).is_err());
}

#[test]
fn editor_mode_opens_the_explicit_image() {
    let (seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Cancelled));
    let code = run_with(["--editor".into(), "/tmp/a.png".into()], &portal, launch);
    assert_eq!(code, 0);
    assert_eq!(*seen.lock().unwrap(), [PathBuf::from("/tmp/a.png")]);
}

#[test]
fn capture_mode_opens_the_portal_file() {
    let (seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/shot.png"))));
    let code = run_with(["--capture".into()], &portal, launch);
    assert_eq!(code, 0);
    assert_eq!(*seen.lock().unwrap(), [PathBuf::from("/tmp/shot.png")]);
}

#[test]
fn a_cancelled_capture_is_silent_and_opens_nothing() {
    let (seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Cancelled));
    let code = run_with(["--capture".into()], &portal, launch);
    assert_eq!(code, 0);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_portal_error_exits_non_zero_and_opens_nothing() {
    let (seen, launch) = recorder();
    let portal = FakePortal(Err(CaptureError("no screenshot portal".into())));
    let code = run_with(["--capture".into()], &portal, launch);
    assert_eq!(code, 1);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_bad_command_line_exits_two() {
    let (_seen, launch) = recorder();
    let portal = FakePortal(Ok(CaptureOutcome::Cancelled));
    assert_eq!(run_with(["--bogus".into()], &portal, launch), 2);
}
