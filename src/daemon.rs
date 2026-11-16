//! Resident capture state machine. The socket thread feeds triggers here; the UI host, the
//! portal, and desktop notifications are injected so the whole flow runs headless in tests.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::capture::{CaptureOutcome, ScreenshotPortal};
use crate::notify::Notifier;

/// Where a captured file goes. The live host decodes the PNG before opening the window, so an
/// unreadable capture fails here and leaves the daemon idle.
pub trait CaptureHost: Send + Sync {
    fn open_editor(&self, path: &Path) -> Result<(), String>;
    fn focus_editor(&self);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerOutcome {
    Opened(PathBuf),
    Focused,
    Cancelled,
    Busy,
    Failed(String),
}

/// What the socket client hears. Both are success from its point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestOutcome {
    Accepted,
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Session {
    Idle,
    Capturing,
    Open,
}

enum Begin {
    Capture,
    Focus,
    Busy,
}

pub struct Daemon {
    portal: Arc<dyn ScreenshotPortal>,
    notifier: Arc<dyn Notifier>,
    host: Arc<dyn CaptureHost>,
    session: Mutex<Session>,
}

impl Daemon {
    pub fn new(
        portal: Arc<dyn ScreenshotPortal>,
        notifier: Arc<dyn Notifier>,
        host: Arc<dyn CaptureHost>,
    ) -> Self {
        Self {
            portal,
            notifier,
            host,
            session: Mutex::new(Session::Idle),
        }
    }

    /// Handle one trigger synchronously: portal, then open or notify. Used by the daemon
    /// tests and by the asynchronous worker.
    pub fn trigger(&self) -> TriggerOutcome {
        match self.begin() {
            Begin::Focus => {
                self.host.focus_editor();
                TriggerOutcome::Focused
            }
            Begin::Busy => TriggerOutcome::Busy,
            Begin::Capture => self.perform(),
        }
    }

    /// Accept a trigger from the socket and return immediately. The capture runs on its own
    /// thread so the client is not held while GNOME's UI is open.
    pub fn request(self: &Arc<Self>) -> RequestOutcome {
        match self.begin() {
            Begin::Focus => {
                self.host.focus_editor();
                RequestOutcome::Accepted
            }
            Begin::Busy => RequestOutcome::Busy,
            Begin::Capture => {
                let daemon = self.clone();
                std::thread::spawn(move || {
                    daemon.perform();
                });
                RequestOutcome::Accepted
            }
        }
    }

    /// The editor window closed. Its state is gone, so the next trigger captures again.
    pub fn editor_closed(&self) {
        let mut session = self.session.lock().unwrap();
        if *session == Session::Open {
            *session = Session::Idle;
        }
    }

    fn begin(&self) -> Begin {
        let mut session = self.session.lock().unwrap();
        match *session {
            Session::Open => Begin::Focus,
            Session::Capturing => Begin::Busy,
            Session::Idle => {
                *session = Session::Capturing;
                Begin::Capture
            }
        }
    }

    fn perform(&self) -> TriggerOutcome {
        let outcome = match self.portal.capture() {
            Ok(CaptureOutcome::Captured(path)) => match self.host.open_editor(&path) {
                Ok(()) => {
                    *self.session.lock().unwrap() = Session::Open;
                    return TriggerOutcome::Opened(path);
                }
                Err(error) => {
                    self.report("cannot open the capture", &error);
                    TriggerOutcome::Failed(error)
                }
            },
            Ok(CaptureOutcome::Cancelled) => TriggerOutcome::Cancelled,
            Err(error) => {
                let message = error.to_string();
                self.report("capture failed", &message);
                TriggerOutcome::Failed(message)
            }
        };
        *self.session.lock().unwrap() = Session::Idle;
        outcome
    }

    fn report(&self, context: &str, detail: &str) {
        eprintln!("gnome-annotation: {context}: {detail}");
        self.notifier
            .notify("GNOME Annotation", &format!("{context}: {detail}"));
    }
}
