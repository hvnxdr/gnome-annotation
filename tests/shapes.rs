//! Shape tools and the annotation palette, driven through the headless editor seam.

mod support;

use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::clipboard::MemoryClipboardSink;
use gnome_annotation::editor::{Editor, EditorOptions, Tool, editor_on_key, editor_view};
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Shell, Waker};

use support::{TempDir, write_4k_fixture};

fn noop_waker() -> Waker {
    Arc::new(|| {})
}

fn key_text(c: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Text(c.to_string()),
        mods: Mods::default(),
    }
}

fn headless_editor(path: &std::path::Path) -> (MemoryClipboardSink, Runtime<Editor>) {
    let sink = MemoryClipboardSink::new();
    let options = EditorOptions::new(path, Arc::new(sink.clone()));
    let mut editor = Editor::new(options).expect("load capture");
    editor.setup(noop_waker());
    let mut rt = Runtime::with_text(
        editor,
        editor_view,
        (1200.0, 800.0),
        Env::new(true, 1.0),
        TextSystem::new(FONTS, ("Inter", "Inter")),
    );
    rt.on_key = Some(editor_on_key);
    rt.frame(0.0);
    (sink, rt)
}

fn canvas_rect(rt: &Runtime<Editor>) -> [f32; 4] {
    rt.presented("canvas").expect("canvas is laid out").0
}

/// Source-image pixels to window points through the fitted canvas.
fn window_point(rect: [f32; 4], source: [f32; 2], image: [u32; 2]) -> (f32, f32) {
    let scale = rect[2] / image[0] as f32;
    (rect[0] + source[0] * scale, rect[1] + source[1] * scale)
}

/// Drag through the canvas from one source point to another and report whether the draft
/// was present before release.
fn drag(rt: &mut Runtime<Editor>, from: [f32; 2], to: [f32; 2]) -> bool {
    let rect = canvas_rect(rt);
    let image = rt.state.image_size();
    let (x0, y0) = window_point(rect, from, image);
    let (x1, y1) = window_point(rect, to, image);
    rt.pointer_down(x0, y0);
    rt.frame(0.0);
    rt.pointer_move(x1, y1);
    rt.frame(0.0);
    let preview = rt
        .elements()
        .iter()
        .any(|e| e.key.as_deref() == Some("draft"));
    rt.pointer_up(x1, y1);
    rt.frame(0.0);
    preview
}

#[test]
fn tool_keys_select_every_tool() {
    let dir = TempDir::new("shape-keys");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    for (key, expected) in [
        ("v", Tool::Select),
        ("l", Tool::Line),
        ("a", Tool::Arrow),
        ("r", Tool::Rectangle),
        ("o", Tool::Ellipse),
        ("x", Tool::Redaction),
        ("t", Tool::Text),
    ] {
        rt.key(key_text(key));
        rt.frame(0.0);
        assert_eq!(rt.state.tool(), expected, "key {key} selects {expected:?}");
    }
}
