//! The screenshot portal seam: a trait the editor flow consumes, with a fake for tests.

use std::path::PathBuf;

use gnome_annotation::capture::{
    CaptureError, CaptureOutcome, ScreenshotPortal, capture_path, uri_to_path,
};

struct FakePortal(Result<CaptureOutcome, CaptureError>);

impl ScreenshotPortal for FakePortal {
    fn capture(&self) -> Result<CaptureOutcome, CaptureError> {
        self.0.clone()
    }
}

#[test]
fn a_successful_capture_returns_the_portal_file() {
    let portal = FakePortal(Ok(CaptureOutcome::Captured(PathBuf::from("/tmp/shot.png"))));
    assert_eq!(
        capture_path(&portal).unwrap(),
        Some(PathBuf::from("/tmp/shot.png"))
    );
}

#[test]
fn a_cancelled_capture_returns_nothing() {
    let portal = FakePortal(Ok(CaptureOutcome::Cancelled));
    assert_eq!(capture_path(&portal).unwrap(), None);
}

#[test]
fn a_portal_error_is_reported() {
    let portal = FakePortal(Err(CaptureError("no screenshot portal".into())));
    let error = capture_path(&portal).unwrap_err();
    assert_eq!(error.to_string(), "no screenshot portal");
}

#[test]
fn portal_uris_become_local_paths() {
    assert_eq!(
        uri_to_path("file:///home/me/Shot%201.png").unwrap(),
        PathBuf::from("/home/me/Shot 1.png")
    );
    assert_eq!(
        uri_to_path("file://localhost/tmp/a%2fb.png").unwrap(),
        PathBuf::from("/tmp/a/b.png")
    );
    assert!(uri_to_path("https://example.com/a.png").is_err());
    assert!(uri_to_path("file://server/share/a.png").is_err());
}
