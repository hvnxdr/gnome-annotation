//! Test doubles for the daemon's system boundaries: the screenshot portal, desktop
//! notifications, and the UI host that opens and focuses the editor.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gnome_annotation::capture::{CaptureError, CaptureOutcome, ScreenshotPortal};
use gnome_annotation::daemon::CaptureHost;
use gnome_annotation::notify::Notifier;

#[derive(Default)]
pub struct FakePortal {
    outcomes: Mutex<VecDeque<Result<CaptureOutcome, CaptureError>>>,
    calls: AtomicUsize,
}

impl FakePortal {
    pub fn new(outcomes: Vec<Result<CaptureOutcome, CaptureError>>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into()),
            calls: AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
}

impl ScreenshotPortal for FakePortal {
    fn capture(&self) -> Result<CaptureOutcome, CaptureError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(CaptureOutcome::Cancelled))
    }
}

#[derive(Default)]
pub struct RecordingNotifier {
    messages: Mutex<Vec<(String, String)>>,
}

impl RecordingNotifier {
    pub fn messages(&self) -> Vec<(String, String)> {
        self.messages.lock().unwrap().clone()
    }
}

impl Notifier for RecordingNotifier {
    fn notify(&self, summary: &str, body: &str) {
        self.messages
            .lock()
            .unwrap()
            .push((summary.to_owned(), body.to_owned()));
    }
}

#[derive(Default)]
pub struct RecordingHost {
    opened: Mutex<Vec<PathBuf>>,
    focused: AtomicUsize,
    fail_open: Mutex<Option<String>>,
}

impl RecordingHost {
    pub fn opened(&self) -> Vec<PathBuf> {
        self.opened.lock().unwrap().clone()
    }

    pub fn focused(&self) -> usize {
        self.focused.load(Ordering::Acquire)
    }

    /// Make later opens fail, as an unreadable PNG would.
    pub fn fail_open(&self, error: Option<&str>) {
        *self.fail_open.lock().unwrap() = error.map(str::to_owned);
    }
}

impl CaptureHost for RecordingHost {
    fn open_editor(&self, path: &Path) -> Result<(), String> {
        if let Some(error) = self.fail_open.lock().unwrap().clone() {
            return Err(error);
        }
        self.opened.lock().unwrap().push(path.to_owned());
        Ok(())
    }

    fn focus_editor(&self) {
        self.focused.fetch_add(1, Ordering::AcqRel);
    }
}

pub type SharedRecordingHost = Arc<RecordingHost>;
