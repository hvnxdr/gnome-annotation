//! Selection and adjustment, driven through the headless editor seam.

mod support;

use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::clipboard::MemoryClipboardSink;
use gnome_annotation::document::AnnotationKind;
use gnome_annotation::editor::{Editor, EditorOptions, editor_on_key, editor_view};
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Waker};

use support::{TempDir, decode_png, is_annotation_red, write_4k_fixture};

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

fn draw_line(rt: &mut Runtime<Editor>, from: [f32; 2], to: [f32; 2]) {
    rt.key(key_text("l"));
    rt.frame(0.0);
    drag(rt, from, to);
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
fn dragging_a_selected_rectangle_moves_it() {
    let dir = TempDir::new("move-rect");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    draw_rectangle(&mut rt, [600.0, 400.0], [1200.0, 900.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [900.0, 650.0]);
    assert!(rt.state.selected().is_some(), "the rectangle is selected");

    let rect = canvas_rect(&rt);
    let image = rt.state.image_size();
    let (x0, y0) = window_point(rect, [900.0, 650.0], image);
    let (x1, y1) = window_point(rect, [1500.0, 1050.0], image);
    rt.pointer_down(x0, y0);
    rt.frame(0.0);
    let keys: Vec<String> = rt
        .elements()
        .iter()
        .filter_map(|element| element.key.clone())
        .collect();
    assert!(
        keys.iter().any(|key| key == "live"),
        "the moved rectangle is previewed while dragging"
    );
    assert!(
        !keys.iter().any(|key| key == "annotations"),
        "the committed overlay leaves the moving rectangle out"
    );
    rt.pointer_move(x1, y1);
    rt.frame(0.0);
    rt.pointer_up(x1, y1);
    rt.frame(0.0);
    assert!(
        rt.elements()
            .iter()
            .any(|element| element.key.as_deref() == Some("annotations")),
        "the committed overlay is rebuilt on release"
    );

    let annotation = &rt.state.document().annotations()[0];
    let AnnotationKind::Rectangle { start, end } = annotation.kind else {
        panic!("expected a rectangle, got {:?}", annotation.kind);
    };
    assert!(
        (start[0] - 1200.0).abs() < 1.0 && (start[1] - 800.0).abs() < 1.0,
        "the top-left corner moves by the drag delta: {start:?}"
    );
    assert!(
        (end[0] - 1800.0).abs() < 1.0 && (end[1] - 1300.0).abs() < 1.0,
        "the bottom-right corner moves by the drag delta: {end:?}"
    );

    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    assert!(
        !is_annotation_red(image.get_pixel(900, 400).0),
        "the rectangle no longer paints its old top edge"
    );
    assert!(
        is_annotation_red(image.get_pixel(1500, 800).0),
        "the rectangle paints its new top edge"
    );
}

#[test]
fn delete_removes_the_selected_annotation_and_clears_the_selection() {
    let dir = TempDir::new("delete");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    draw_rectangle(&mut rt, [600.0, 400.0], [1200.0, 900.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [900.0, 650.0]);
    assert!(rt.state.selected().is_some(), "the rectangle is selected");

    rt.key(KeyEvent {
        key: Key::Delete,
        mods: Mods::default(),
    });
    rt.frame(0.0);
    assert_eq!(rt.state.selected(), None, "delete clears the selection");
    assert!(
        rt.state.document().is_empty(),
        "the selected annotation is gone"
    );
    assert!(
        rt.presented("selection").is_none(),
        "the chrome is gone with the annotation"
    );

    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    assert!(
        !image
            .enumerate_pixels()
            .any(|(_, _, pixel)| is_annotation_red(pixel.0)),
        "the deleted annotation is not in the export"
    );
}

#[test]
fn backspace_also_deletes_the_selected_annotation() {
    let dir = TempDir::new("delete-backspace");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    draw_rectangle(&mut rt, [600.0, 400.0], [1200.0, 900.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [900.0, 650.0]);
    rt.key(KeyEvent {
        key: Key::Backspace,
        mods: Mods::default(),
    });
    rt.frame(0.0);
    assert!(rt.state.document().is_empty());
    assert_eq!(rt.state.selected(), None);
}

#[test]
fn selection_chrome_is_visible_and_never_exported() {
    let dir = TempDir::new("chrome");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    draw_rectangle(&mut rt, [600.0, 400.0], [1200.0, 900.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [900.0, 650.0]);

    assert!(
        rt.presented("selection").is_some(),
        "the selection layer is on screen"
    );
    for handle in 0..4 {
        assert!(
            rt.presented(&format!("handle-{handle}")).is_some(),
            "rectangle handle {handle} is on screen"
        );
    }
    let with_chrome = rt.state.export_png().expect("the export succeeds");

    click(&mut rt, [3000.0, 1800.0]);
    assert!(
        rt.presented("selection").is_none(),
        "clearing the selection drops the chrome"
    );
    let without_chrome = rt.state.export_png().expect("the export succeeds");
    assert_eq!(
        with_chrome, without_chrome,
        "the selection chrome never enters the exported PNG"
    );
}

#[test]
fn dragging_a_corner_handle_resizes_a_rectangle() {
    let dir = TempDir::new("resize-rect");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    draw_rectangle(&mut rt, [600.0, 400.0], [1200.0, 900.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [900.0, 650.0]);
    drag(&mut rt, [1200.0, 900.0], [1500.0, 1100.0]);

    let annotation = &rt.state.document().annotations()[0];
    let AnnotationKind::Rectangle { start, end } = annotation.kind else {
        panic!("expected a rectangle, got {:?}", annotation.kind);
    };
    assert!(
        (start[0] - 600.0).abs() < 1.0 && (start[1] - 400.0).abs() < 1.0,
        "the opposite corner stays fixed: {start:?}"
    );
    assert!(
        (end[0] - 1500.0).abs() < 1.0 && (end[1] - 1100.0).abs() < 1.0,
        "the dragged corner follows the pointer: {end:?}"
    );

    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    assert!(
        is_annotation_red(image.get_pixel(1495, 1000).0),
        "the rectangle reaches its new right edge"
    );
    assert!(
        !is_annotation_red(image.get_pixel(900, 900).0),
        "the old bottom edge is now interior and no longer painted"
    );
}

#[test]
fn dragging_a_corner_handle_resizes_an_ellipse() {
    let dir = TempDir::new("resize-ellipse");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("o"));
    rt.frame(0.0);
    drag(&mut rt, [600.0, 400.0], [1200.0, 900.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [900.0, 650.0]);
    drag(&mut rt, [1200.0, 900.0], [1500.0, 1100.0]);

    let annotation = &rt.state.document().annotations()[0];
    let AnnotationKind::Ellipse { start, end } = annotation.kind else {
        panic!("expected an ellipse, got {:?}", annotation.kind);
    };
    assert!(
        (start[0] - 600.0).abs() < 1.0 && (start[1] - 400.0).abs() < 1.0,
        "the opposite corner stays fixed: {start:?}"
    );
    assert!(
        (end[0] - 1500.0).abs() < 1.0 && (end[1] - 1100.0).abs() < 1.0,
        "the dragged corner follows the pointer: {end:?}"
    );

    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    assert!(
        is_annotation_red(image.get_pixel(1495, 750).0),
        "the ellipse reaches its new right edge"
    );
    assert!(
        !is_annotation_red(image.get_pixel(1200, 650).0),
        "the old right edge is no longer on the curve"
    );
}

#[test]
fn dragging_a_corner_handle_resizes_a_redaction() {
    let dir = TempDir::new("resize-redaction");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("x"));
    rt.frame(0.0);
    drag(&mut rt, [600.0, 400.0], [1200.0, 900.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [900.0, 650.0]);
    drag(&mut rt, [1200.0, 900.0], [1500.0, 1100.0]);

    let annotation = &rt.state.document().annotations()[0];
    let AnnotationKind::Redaction { start, end } = annotation.kind else {
        panic!("expected a redaction, got {:?}", annotation.kind);
    };
    assert!(
        (start[0] - 600.0).abs() < 1.0 && (start[1] - 400.0).abs() < 1.0,
        "the opposite corner stays fixed: {start:?}"
    );
    assert!(
        (end[0] - 1500.0).abs() < 1.0 && (end[1] - 1100.0).abs() < 1.0,
        "the dragged corner follows the pointer: {end:?}"
    );

    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    assert_eq!(
        image.get_pixel(1300, 1000).0,
        [0, 0, 0, 255],
        "the redaction grew past its old right edge"
    );
    assert_ne!(
        image.get_pixel(1550, 1000).0,
        [0, 0, 0, 255],
        "the redaction stops at its new right edge"
    );
}

#[test]
fn dragging_an_endpoint_handle_resizes_a_line() {
    let dir = TempDir::new("resize-line");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    draw_line(&mut rt, [600.0, 400.0], [1800.0, 900.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [1200.0, 650.0]);
    drag(&mut rt, [1800.0, 900.0], [2400.0, 300.0]);

    let annotation = &rt.state.document().annotations()[0];
    let AnnotationKind::Line { start, end } = annotation.kind else {
        panic!("expected a line, got {:?}", annotation.kind);
    };
    assert!(
        (start[0] - 600.0).abs() < 1.0 && (start[1] - 400.0).abs() < 1.0,
        "the opposite endpoint stays fixed: {start:?}"
    );
    assert!(
        (end[0] - 2400.0).abs() < 1.0 && (end[1] - 300.0).abs() < 1.0,
        "the dragged endpoint follows the pointer: {end:?}"
    );

    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    assert!(
        is_annotation_red(image.get_pixel(2395, 300).0),
        "the line reaches its new endpoint"
    );
    assert!(
        !is_annotation_red(image.get_pixel(1800, 900).0),
        "the line no longer paints its old endpoint"
    );
}

#[test]
fn dragging_an_endpoint_handle_resizes_an_arrow() {
    let dir = TempDir::new("resize-arrow");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("a"));
    rt.frame(0.0);
    drag(&mut rt, [600.0, 400.0], [1800.0, 400.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [1200.0, 400.0]);
    drag(&mut rt, [600.0, 400.0], [900.0, 900.0]);

    let annotation = &rt.state.document().annotations()[0];
    let AnnotationKind::Arrow { start, end } = annotation.kind else {
        panic!("expected an arrow, got {:?}", annotation.kind);
    };
    assert!(
        (start[0] - 900.0).abs() < 1.0 && (start[1] - 900.0).abs() < 1.0,
        "the dragged start follows the pointer: {start:?}"
    );
    assert!(
        (end[0] - 1800.0).abs() < 1.0 && (end[1] - 400.0).abs() < 1.0,
        "the opposite endpoint stays fixed: {end:?}"
    );

    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    assert!(
        is_annotation_red(image.get_pixel(1200, 733).0),
        "the arrow shaft follows the dragged start"
    );
    assert!(
        !is_annotation_red(image.get_pixel(1200, 400).0),
        "the old horizontal shaft is gone"
    );
}

#[test]
fn a_diagonal_line_is_hit_near_the_segment_not_at_its_bounding_box_corner() {
    let dir = TempDir::new("line-hit");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    draw_line(&mut rt, [600.0, 400.0], [1800.0, 900.0]);
    let id = rt.state.document().annotations()[0].id;
    rt.key(key_text("v"));
    rt.frame(0.0);

    // The segment is at y=650 where x=1200; (1200, 660) sits 10 px below the midpoint.
    click(&mut rt, [1200.0, 660.0]);
    assert_eq!(
        rt.state.selected(),
        Some(id),
        "a click near the segment selects the line"
    );

    click(&mut rt, [1750.0, 450.0]);
    assert_eq!(
        rt.state.selected(),
        None,
        "a bounding-box corner far from the segment does not select the line"
    );
}

#[test]
fn an_arrow_is_hit_near_its_shaft() {
    let dir = TempDir::new("arrow-hit");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("a"));
    rt.frame(0.0);
    drag(&mut rt, [600.0, 400.0], [1800.0, 900.0]);
    let id = rt.state.document().annotations()[0].id;
    rt.key(key_text("v"));
    rt.frame(0.0);

    click(&mut rt, [1200.0, 630.0]);
    assert_eq!(
        rt.state.selected(),
        Some(id),
        "a click near the shaft selects the arrow"
    );

    click(&mut rt, [650.0, 850.0]);
    assert_eq!(
        rt.state.selected(),
        None,
        "a corner of the arrow's box misses"
    );
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
    assert_eq!(
        rt.state.selected(),
        None,
        "empty canvas clears the selection"
    );
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
