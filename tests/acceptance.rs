//! Desktop acceptance checks that can run without a display.
//!
//! The live portal, clipboard paste, and Print keybinding still need a human pass. These
//! tests pin the parts that a headless runtime can prove: the 4K fit, screen-space handles
//! at a high device scale, and a source-resolution copy.

mod support;

use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::editor::{Editor, EditorOptions, editor_on_key, editor_view};
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Waker};

use support::{TempDir, decode_png, write_4k_fixture};

use support::fakes::MemoryClipboardSink;
fn noop_waker() -> Waker {
    Arc::new(|| {})
}

fn key_text(c: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Text(c.to_string()),
        mods: Mods::default(),
    }
}

fn enter() -> KeyEvent {
    KeyEvent {
        key: Key::Enter,
        mods: Mods::default(),
    }
}

fn headless_editor(path: &std::path::Path, scale: f32) -> (MemoryClipboardSink, Runtime<Editor>) {
    let sink = MemoryClipboardSink::new();
    let options = EditorOptions::new(path, Arc::new(sink.clone()));
    let mut editor = Editor::new(options).expect("load capture");
    editor.setup(noop_waker());
    let mut rt = Runtime::with_text(
        editor,
        editor_view,
        (1200.0, 800.0),
        Env::new(true, scale),
        TextSystem::new(FONTS, ("Inter", "Inter")),
    );
    rt.on_key = Some(editor_on_key);
    rt.frame(0.0);
    (sink, rt)
}

fn canvas_rect(rt: &Runtime<Editor>) -> [f32; 4] {
    rt.presented("canvas").expect("canvas is laid out").0
}

fn window_point(rect: [f32; 4], source: [f32; 2], image: [u32; 2]) -> (f32, f32) {
    let scale = rect[2] / image[0] as f32;
    (rect[0] + source[0] * scale, rect[1] + source[1] * scale)
}

fn drag(rt: &mut Runtime<Editor>, from: [f32; 2], to: [f32; 2]) {
    let rect = canvas_rect(rt);
    let image = rt.state.image_size();
    let (x0, y0) = window_point(rect, from, image);
    let (x1, y1) = window_point(rect, to, image);
    rt.pointer_down(x0, y0);
    rt.frame(0.0);
    rt.pointer_move(x1, y1);
    rt.frame(0.0);
    rt.pointer_up(x1, y1);
    rt.frame(0.0);
}

fn click(rt: &mut Runtime<Editor>, at: [f32; 2]) {
    let rect = canvas_rect(rt);
    let image = rt.state.image_size();
    let (x, y) = window_point(rect, at, image);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

#[test]
fn a_4k_capture_fits_and_handles_stay_eight_points_at_two_x() {
    let dir = TempDir::new("acceptance-hidpi");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture, 2.0);

    let canvas = canvas_rect(&rt);
    assert!(
        canvas[2] <= 1200.0 && canvas[3] <= 800.0,
        "the 4K capture is fitted inside the window: {canvas:?}"
    );
    assert!(
        canvas[2] < 3840.0 && canvas[3] < 2160.0,
        "the canvas is not drawn at source size: {canvas:?}"
    );

    rt.key(key_text("r"));
    rt.frame(0.0);
    drag(&mut rt, [600.0, 400.0], [1600.0, 1200.0]);

    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [1100.0, 800.0]);
    assert!(rt.state.selected().is_some(), "the rectangle is selected");

    let handle = rt.presented("handle-0").expect("a selection handle");
    assert_eq!(
        (handle.0[2], handle.0[3]),
        (8.0, 8.0),
        "handles are screen-space points, not device pixels"
    );

    rt.key(enter());
    rt.frame(0.0);
    let writes = sink.writes();
    assert_eq!(writes.len(), 1, "one copy reached the clipboard");
    assert_eq!(
        decode_png(&writes[0]).dimensions(),
        (3840, 2160),
        "the copy keeps the source resolution at 2x"
    );
}
