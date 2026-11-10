//! The resident daemon's Rune state: idle renders an empty root, an open editor renders the
//! shared editor view, and the editor's handlers reach through the slot.

mod support;

use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::clipboard::MemoryClipboardSink;
use gnome_annotation::editor::{
    Editor, EditorOptions, HostsEditor, Tool, editor_on_key, editor_view,
};
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Tx, Waker};
use support::{TempDir, write_4k_fixture};

struct Slot {
    editor: Option<Editor>,
}

impl HostsEditor for Slot {
    fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref()
    }

    fn editor_mut(&mut self) -> Option<&mut Editor> {
        self.editor.as_mut()
    }
}

fn noop_waker() -> Waker {
    Arc::new(|| {})
}

fn open_slot(path: &std::path::Path) -> Slot {
    let sink = Arc::new(MemoryClipboardSink::new());
    let mut editor = Editor::new(EditorOptions::new(path, sink)).expect("load the capture");
    editor.setup(noop_waker());
    Slot {
        editor: Some(editor),
    }
}

fn runtime(state: Slot) -> Runtime<Slot> {
    let mut rt = Runtime::with_text(
        state,
        editor_view,
        (1200.0, 800.0),
        Env::new(true, 1.0),
        TextSystem::new(FONTS, ("Inter", "Inter")),
    );
    fn on_key(state: &mut Slot, event: &KeyEvent, tx: &mut Tx) -> bool {
        match state.editor.as_mut() {
            Some(editor) => editor_on_key(editor, event, tx),
            None => false,
        }
    }
    rt.on_key = Some(on_key);
    rt.frame(0.0);
    rt
}

#[test]
fn an_idle_daemon_draws_no_editor() {
    let rt = runtime(Slot { editor: None });
    assert!(
        rt.presented("canvas").is_none(),
        "an idle daemon has no editor canvas"
    );
    assert!(rt.find("tool-arrow").is_none());
    assert!(rt.find("empty").is_some(), "the idle root is drawn");
}

#[test]
fn an_open_editor_draws_the_shared_view() {
    let dir = TempDir::new("daemon-view");
    let fixture = write_4k_fixture(dir.path());
    let rt = runtime(open_slot(&fixture));
    let (canvas, _) = rt
        .presented("canvas")
        .expect("the editor canvas is laid out");
    assert!(
        canvas[2] < 3840.0 && canvas[3] < 2160.0,
        "fitted: {canvas:?}"
    );
    assert!(rt.find("tool-arrow").is_some());
}

#[test]
fn the_editor_handlers_reach_the_slot() {
    let dir = TempDir::new("daemon-handlers");
    let fixture = write_4k_fixture(dir.path());
    let mut rt = runtime(open_slot(&fixture));

    let (x, y) = rt.find("tool-arrow").expect("the arrow tool button");
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    assert_eq!(rt.state.editor.as_ref().unwrap().tool(), Tool::Arrow);

    rt.key(KeyEvent {
        key: Key::Text("t".into()),
        mods: Mods::default(),
    });
    rt.frame(0.0);
    assert_eq!(rt.state.editor.as_ref().unwrap().tool(), Tool::Text);
}

#[test]
fn the_editor_state_is_send_so_the_capture_worker_can_hand_it_over() {
    fn assert_send<T: Send>() {}
    assert_send::<Editor>();
}
