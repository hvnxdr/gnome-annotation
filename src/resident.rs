//! The resident daemon: a windowless Rune app, the socket that feeds it triggers, and the
//! adapter that turns a captured PNG into an editor window. `run_trigger` is the short-lived
//! client side of the same binary.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rune::{AppHandle, AppSpec, Tx, Waker};

use crate::assets::{FONTS, UI_FAMILY};
use crate::capture::PortalScreenshot;
use crate::clipboard::{ClipboardSink, RuneClipboard};
use crate::daemon::{CaptureHost, Daemon};
use crate::editor::{
    APP_ID, APP_TITLE, DEFAULT_WINDOW, Editor, EditorOptions, HostsEditor, MIN_WINDOW,
    editor_on_key, editor_view,
};
use crate::ipc::{IpcError, SocketServer, TriggerClient, socket_path};
use crate::notify::DbusNotifier;

/// The daemon's app state. The editor exists only while its window is open, so closing the
/// window drops the 4K source, pixmap, and annotation layer.
struct DaemonState {
    editor: Option<Editor>,
    waker: Arc<Mutex<Option<Waker>>>,
    daemon: Arc<Daemon>,
}

impl HostsEditor for DaemonState {
    fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref()
    }

    fn editor_mut(&mut self) -> Option<&mut Editor> {
        self.editor.as_mut()
    }
}

/// The bridge the daemon calls from its capture worker. The decode happens off the UI thread;
/// the prepared editor is moved into Rune's state and the window opens on the next turn.
struct RuneHost {
    handle: AppHandle<DaemonState>,
    clipboard: Arc<dyn ClipboardSink>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl RuneHost {
    fn wait_for_waker(&self) -> Option<Waker> {
        for _ in 0..200 {
            if let Some(waker) = self.waker.lock().unwrap().clone() {
                return Some(waker);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl CaptureHost for RuneHost {
    fn open_editor(&self, path: &Path) -> Result<(), String> {
        let waker = self
            .wait_for_waker()
            .ok_or_else(|| "the daemon is not ready to open a window".to_owned())?;
        let started = Instant::now();
        let options = EditorOptions::new(path, self.clipboard.clone());
        let mut editor = Editor::new(options).map_err(|error| error.to_string())?;
        eprintln!(
            "gnome-annotation: decoded and prepared the capture in {:.1} ms",
            editor.timings().decode_ms
        );
        let handle = self.handle.clone();
        handle.update(move |state, _tx| {
            editor.setup(waker);
            state.editor = Some(editor);
        });
        handle.open();
        eprintln!(
            "gnome-annotation: editor requested {:.1} ms after the portal returned",
            started.elapsed().as_secs_f64() * 1000.0
        );
        Ok(())
    }

    fn focus_editor(&self) {
        self.handle.open();
    }
}

fn daemon_setup(state: &mut DaemonState, waker: Waker) {
    *state.waker.lock().unwrap() = Some(waker);
}

fn daemon_on_close(state: &mut DaemonState, _tx: &mut Tx) {
    if state.editor.take().is_some() {
        eprintln!("gnome-annotation: editor closed; the daemon is idle");
    }
    state.daemon.editor_closed();
}

fn daemon_on_key(state: &mut DaemonState, event: &rune::KeyEvent, tx: &mut Tx) -> bool {
    match state.editor.as_mut() {
        Some(editor) => editor_on_key(editor, event, tx),
        None => false,
    }
}

pub fn run_daemon() -> Result<(), String> {
    rune::set_app_id(APP_ID);
    let waker: Arc<Mutex<Option<Waker>>> = Arc::default();
    let mut spec = AppSpec::new(APP_TITLE, DEFAULT_WINDOW, || unreachable!(), editor_view);
    spec.min_size = Some(MIN_WINDOW);
    spec.fonts = FONTS;
    spec.families = (UI_FAMILY, UI_FAMILY);
    spec.setup = Some(daemon_setup);
    spec.on_key = Some(daemon_on_key);

    let app = rune::App::new(spec).windowless().on_close(daemon_on_close);
    let host = Arc::new(RuneHost {
        handle: app.handle(),
        clipboard: Arc::new(RuneClipboard),
        waker: waker.clone(),
    });
    let daemon = Arc::new(Daemon::new(
        Arc::new(PortalScreenshot),
        Arc::new(DbusNotifier),
        host,
    ));

    let server = match SocketServer::bind(daemon.clone()) {
        Ok(server) => server,
        Err(IpcError::AlreadyRunning) => {
            eprintln!("gnome-annotation: another daemon is already running");
            return Ok(());
        }
        Err(error) => return Err(format!("cannot start the daemon socket: {error}")),
    };

    let state = DaemonState {
        editor: None,
        waker,
        daemon,
    };
    let result = app
        .init(move || state)
        .try_run()
        .map_err(|error| format!("cannot start the daemon: {error}"));
    drop(server);
    result
}

pub fn run_trigger() -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("cannot find the gnome-annotation binary: {error}"))?;
    TriggerClient::new(socket_path(), exe)
        .run()
        .map(|_| ())
        .map_err(|error| error.to_string())
}
