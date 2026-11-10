//! Selection and adjustment, driven through the headless editor seam.

mod support;

use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::clipboard::MemoryClipboardSink;
use gnome_annotation::editor::{Editor, EditorOptions, editor_on_key, editor_view};
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Waker};

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

/// Drag through the canvas with the active tool.
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

fn draw_rectangle(rt: &mut Runtime<Editor>, from: [f32; 2], to: [f32; 2]) {
    rt.key(key_text("r"));
    rt.frame(0.0);
    drag(rt, from, to);
}

fn select_topmost_shapes(rt: &mut Runtime<Editor>) {
    // An earlier, larger rectangle and a later, smaller one on top of it.
    draw_rectangle(rt, [600.0, 400.0], [1800.0, 1200.0]);
    draw_rectangle(rt, [900.0, 600.0], [1500.0, 1000.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
}

#[test]
fn clicking_empty_canvas_clears_the_selection() {
    let dir = TempDir::new("select-clear");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);
    select_topmost_shapes(&mut rt);

    click(&mut rt, [1200.0, 800.0]);
    assert!(rt.state.selected().is_some(), "a rectangle is selected");

    click(&mut rt, [3000.0, 1800.0]);
    assert_eq!(rt.state.selected(), None, "empty canvas clears the selection");
}

#[test]
fn clicking_an_annotation_selects_the_topmost_one_under_the_pointer() {
    let dir = TempDir::new("select-topmost");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);
    select_topmost_shapes(&mut rt);

    let ids: Vec<u64> = rt
        .state
        .document()
        .annotations()
        .iter()
        .map(|annotation| annotation.id)
        .collect();
    assert_eq!(ids.len(), 2);

    click(&mut rt, [1200.0, 800.0]);
    assert_eq!(
        rt.state.selected(),
        Some(ids[1]),
        "the overlap selects the later, topmost rectangle"
    );

    click(&mut rt, [700.0, 500.0]);
    assert_eq!(
        rt.state.selected(),
        Some(ids[0]),
        "a point only the earlier rectangle covers selects it"
    );
}
