//! Test doubles for the app's system boundaries: the screenshot portal, desktop
//! notifications, the UI host that opens and focuses the editor, the clipboard, and Save As.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gnome_annotation::capture::{CaptureError, CaptureOutcome, ScreenshotPortal};
use gnome_annotation::clipboard::{ClipboardError, ClipboardSink};
use gnome_annotation::daemon::CaptureHost;
use gnome_annotation::notify::Notifier;
use gnome_annotation::save::{SaveError, SaveOutcome, SaveSink};
use rune::Tx;

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

/// A clipboard sink that keeps every PNG it receives.
#[derive(Clone, Default)]
pub struct MemoryClipboardSink {
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
    fail: Arc<Mutex<Option<String>>>,
}

impl MemoryClipboardSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn writes(&self) -> Vec<Vec<u8>> {
        self.writes.lock().unwrap().clone()
    }

    /// Make later writes fail, to exercise the editor's error path.
    pub fn fail_writes(&self, error: Option<&str>) {
        *self.fail.lock().unwrap() = error.map(str::to_owned);
    }
}

impl ClipboardSink for MemoryClipboardSink {
    fn write_png(&self, png: &[u8], _tx: &mut Tx) -> Result<(), ClipboardError> {
        if let Some(error) = self.fail.lock().unwrap().clone() {
            return Err(ClipboardError(error));
        }
        self.writes.lock().unwrap().push(png.to_vec());
        Ok(())
    }
}

/// One PNG a memory save accepted, with the name the dialog would have suggested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedPng {
    pub suggested_name: String,
    pub png: Vec<u8>,
}

/// A save sink that keeps every PNG it receives, or cancels or fails on request.
#[derive(Clone, Default)]
pub struct MemorySaveSink {
    saves: Arc<Mutex<Vec<SavedPng>>>,
    cancel: Arc<Mutex<bool>>,
    fail: Arc<Mutex<Option<String>>>,
}

impl MemorySaveSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn saves(&self) -> Vec<SavedPng> {
        self.saves.lock().unwrap().clone()
    }

    /// Answer the next saves like a cancelled chooser: nothing is written and the editor stays.
    pub fn cancel_saves(&self) {
        *self.cancel.lock().unwrap() = true;
    }

    /// Make later saves fail, to exercise the editor's error path.
    pub fn fail_writes(&self, error: Option<&str>) {
        *self.fail.lock().unwrap() = error.map(str::to_owned);
    }
}

impl SaveSink for MemorySaveSink {
    fn save(&self, suggested_name: &str, png: &[u8]) -> Result<SaveOutcome, SaveError> {
        if let Some(error) = self.fail.lock().unwrap().clone() {
            return Err(SaveError(error));
        }
        if *self.cancel.lock().unwrap() {
            return Ok(SaveOutcome::Cancelled);
        }
        self.saves.lock().unwrap().push(SavedPng {
            suggested_name: suggested_name.to_owned(),
            png: png.to_vec(),
        });
        Ok(SaveOutcome::Saved)
    }
}
