//! Undo and redo, driven through the headless editor seam.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::document::{AnnotationKind, PALETTE, Rgba};
use gnome_annotation::editor::{Editor, EditorOptions, editor_on_key, editor_view};
use image::RgbaImage;
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Waker};

use support::{TempDir, decode_png, is_annotation_red, write_png};

use support::fakes::MemoryClipboardSink;
fn noop_waker() -> Waker {
    Arc::new(|| {})
}

fn headless_editor(path: &Path) -> (MemoryClipboardSink, Runtime<Editor>) {
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
    rt.frame(0.0);
    (sink, rt)
}

/// A checkerboard smaller than the 4K fixtures so undo tests stay quick.
fn write_fixture(dir: &Path) -> PathBuf {
    const W: u32 = 1600;
    const H: u32 = 1000;
    const TILE: u32 = 64;
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let light = ((x / TILE) + (y / TILE)) % 2 == 0;
            let v = if light { 72u8 } else { 36u8 };
            pixels.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let path = dir.join("capture.png");
    write_png(&path, W, H, &pixels);
    path
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

fn escape() -> KeyEvent {
    KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    }
}

/// Ctrl+Z as Linux delivers it: `Command('z')` with both `cmd` and `ctrl` set.
fn undo_key() -> KeyEvent {
    KeyEvent {
        key: Key::Command('z'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    }
}

fn redo_key() -> KeyEvent {
    KeyEvent {
        key: Key::Command('z'),
        mods: Mods {
            shift: true,
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    }
}

fn undo(rt: &mut Runtime<Editor>) {
    rt.key(undo_key());
    rt.frame(0.0);
}

fn redo(rt: &mut Runtime<Editor>) {
    rt.key(redo_key());
    rt.frame(0.0);
}

fn canvas_rect(rt: &Runtime<Editor>) -> [f32; 4] {
    rt.presented("canvas").expect("canvas is laid out").0
}

/// Source-image pixels to window points through the fitted canvas.
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

/// Drag through several source points, moving once between each, then release.
fn drag_path(rt: &mut Runtime<Editor>, points: &[[f32; 2]]) {
    let rect = canvas_rect(rt);
    let image = rt.state.image_size();
    let window = |point: [f32; 2]| window_point(rect, point, image);
    let (first, rest) = points.split_first().expect("a path has a start");
    let (x0, y0) = window(*first);
    rt.pointer_down(x0, y0);
    rt.frame(0.0);
    for point in rest {
        let (x, y) = window(*point);
        rt.pointer_move(x, y);
        rt.frame(0.0);
    }
    let (x, y) = window(*points.last().expect("a path has an end"));
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn export(rt: &mut Runtime<Editor>) -> RgbaImage {
    decode_png(&rt.state.export_png().expect("the export succeeds"))
}

fn has_annotation_red(image: &RgbaImage) -> bool {
    image
        .enumerate_pixels()
        .any(|(_, _, px)| is_annotation_red(px.0))
}

fn near(actual: [f32; 2], expected: [f32; 2]) -> bool {
    (actual[0] - expected[0]).abs() < 1.0 && (actual[1] - expected[1]).abs() < 1.0
}

fn rectangle_geometry(rt: &Runtime<Editor>) -> ([f32; 2], [f32; 2]) {
    let annotation = &rt.state.document().annotations()[0];
    let AnnotationKind::Rectangle { start, end } = annotation.kind else {
        panic!("expected a rectangle, got {:?}", annotation.kind);
    };
    (start, end)
}

/// Two clicks inside the system interval and slop, with the frames Rune needs to pair them.
fn double_click(rt: &mut Runtime<Editor>, at: [f32; 2]) {
    let rect = canvas_rect(rt);
    let image = rt.state.image_size();
    let (x, y) = window_point(rect, at, image);
    for _ in 0..2 {
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(1.0 / 120.0);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 120.0);
    }
    rt.frame(0.0);
}

/// Let exits and transitions finish so element lookups see the settled tree.
fn settle(rt: &mut Runtime<Editor>) {
    for _ in 0..60 {
        rt.frame(1.0 / 120.0);
    }
}

fn press(rt: &mut Runtime<Editor>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is present"));
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn text_field(rt: &Runtime<Editor>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("text-editor") && !e.exiting)
        .and_then(|e| e.text)
}

fn place_text(rt: &mut Runtime<Editor>, at: [f32; 2], value: &str) {
    rt.key(key_text("t"));
    rt.frame(0.0);
    click(rt, at);
    rt.paste(value);
    rt.frame(0.0);
    rt.key(enter());
    rt.frame(0.0);
}

fn is_palette_color(px: [u8; 4], color: Rgba) -> bool {
    px[3] == 255
        && (i32::from(px[0]) - i32::from(color.r)).abs() <= 4
        && (i32::from(px[1]) - i32::from(color.g)).abs() <= 4
        && (i32::from(px[2]) - i32::from(color.b)).abs() <= 4
}

fn is_halo(px: [u8; 4]) -> bool {
    px[3] == 255 && px[0] > 220 && px[1] > 220 && px[2] > 220
}

/// Whether any pixel inside the inclusive region matches `predicate`.
fn has_color_in(
    image: &RgbaImage,
    [x0, y0, x1, y1]: [u32; 4],
    predicate: impl Fn([u8; 4]) -> bool,
) -> bool {
    image
        .enumerate_pixels()
        .any(|(x, y, px)| (x0..=x1).contains(&x) && (y0..=y1).contains(&y) && predicate(px.0))
}

fn has_red_in(image: &RgbaImage, region: [u32; 4]) -> bool {
    has_color_in(image, region, is_annotation_red)
}

fn has_black_in(image: &RgbaImage, region: [u32; 4]) -> bool {
    has_color_in(image, region, |px| px == [0, 0, 0, 255])
}

#[test]
fn a_drawn_line_can_be_undone_and_redone() {
    let dir = TempDir::new("undo-line");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("l"));
    rt.frame(0.0);
    drag(&mut rt, [100.0, 120.0], [500.0, 120.0]);

    assert_eq!(rt.state.document().annotations().len(), 1);
    assert!(rt.state.can_undo(), "the draw is on the undo stack");
    assert!(!rt.state.can_redo(), "a new edit leaves no redo");
    let drawn = rt.state.document().annotations()[0].clone();
    let drawn_png = rt.state.export_png().expect("the export succeeds");
    assert!(has_annotation_red(&decode_png(&drawn_png)));

    undo(&mut rt);
    assert!(rt.state.document().is_empty(), "undo removes the line");
    assert!(!rt.state.can_undo(), "the stack is empty");
    assert!(rt.state.can_redo(), "the line is on the redo stack");
    assert!(
        !has_annotation_red(&export(&mut rt)),
        "the undone line leaves no pixels"
    );

    redo(&mut rt);
    assert_eq!(
        rt.state.document().annotations(),
        &[drawn],
        "redo restores the same annotation, id included"
    );
    assert_eq!(
        rt.state.export_png().expect("the export succeeds"),
        drawn_png,
        "redo restores the exported pixels"
    );
}

#[test]
fn moving_a_rectangle_is_undoable_and_redoable() {
    let dir = TempDir::new("undo-move");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("r"));
    rt.frame(0.0);
    drag(&mut rt, [200.0, 200.0], [500.0, 400.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [350.0, 300.0]);
    assert!(rt.state.selected().is_some(), "the rectangle is selected");

    drag(&mut rt, [350.0, 300.0], [650.0, 300.0]);
    let (start, end) = rectangle_geometry(&rt);
    assert!(near(start, [500.0, 200.0]), "the move commits: {start:?}");
    assert!(near(end, [800.0, 400.0]), "the move commits: {end:?}");
    assert!(rt.state.can_undo(), "the move is recorded once");

    let moved = export(&mut rt);
    let moved_bounds = support::pixel_bounds(&moved, is_annotation_red).expect("ink is exported");
    assert!(
        moved_bounds[0] >= 495,
        "the exported rectangle moved right: {moved_bounds:?}"
    );

    undo(&mut rt);
    let (start, end) = rectangle_geometry(&rt);
    assert!(
        near(start, [200.0, 200.0]) && near(end, [500.0, 400.0]),
        "undo restores the original geometry: {start:?} {end:?}"
    );
    assert_eq!(rt.state.selected(), None, "undo clears the selection");
    assert!(rt.state.can_redo(), "the move is on the redo stack");
    let original = export(&mut rt);
    let original_bounds =
        support::pixel_bounds(&original, is_annotation_red).expect("ink is exported");
    assert!(
        original_bounds[0] <= 205,
        "the export returns to the original position: {original_bounds:?}"
    );

    redo(&mut rt);
    let (start, end) = rectangle_geometry(&rt);
    assert!(
        near(start, [500.0, 200.0]) && near(end, [800.0, 400.0]),
        "redo restores the moved geometry: {start:?} {end:?}"
    );
    let redone = export(&mut rt);
    let redone_bounds = support::pixel_bounds(&redone, is_annotation_red).expect("ink is exported");
    assert_eq!(
        redone_bounds, moved_bounds,
        "redo restores the exported position"
    );
}

#[test]
fn resizing_a_rectangle_is_undoable_and_redoable() {
    let dir = TempDir::new("undo-resize");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("r"));
    rt.frame(0.0);
    drag(&mut rt, [200.0, 200.0], [500.0, 400.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [350.0, 300.0]);
    drag(&mut rt, [500.0, 400.0], [800.0, 600.0]);

    let (start, end) = rectangle_geometry(&rt);
    assert!(
        near(start, [200.0, 200.0]) && near(end, [800.0, 600.0]),
        "the resize commits: {start:?} {end:?}"
    );
    let resized_bounds =
        support::pixel_bounds(&export(&mut rt), is_annotation_red).expect("ink is exported");
    assert!(
        resized_bounds[2] >= 795 && resized_bounds[3] >= 595,
        "the export grows with the resize: {resized_bounds:?}"
    );

    undo(&mut rt);
    let (start, end) = rectangle_geometry(&rt);
    assert!(
        near(start, [200.0, 200.0]) && near(end, [500.0, 400.0]),
        "undo restores the original size: {start:?} {end:?}"
    );
    let original_bounds =
        support::pixel_bounds(&export(&mut rt), is_annotation_red).expect("ink is exported");
    assert!(
        original_bounds[2] <= 505 && original_bounds[3] <= 405,
        "the export returns to the original size: {original_bounds:?}"
    );

    redo(&mut rt);
    let (start, end) = rectangle_geometry(&rt);
    assert!(
        near(start, [200.0, 200.0]) && near(end, [800.0, 600.0]),
        "redo restores the resized geometry: {start:?} {end:?}"
    );
    assert_eq!(
        support::pixel_bounds(&export(&mut rt), is_annotation_red),
        Some(resized_bounds),
        "redo restores the exported size"
    );
}

#[test]
fn a_text_commit_can_be_undone_and_redone_with_its_style_and_halo() {
    let dir = TempDir::new("undo-text");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    let fit = rt.state.fit_scale();
    press(&mut rt, "color-4");
    press(&mut rt, "text-size-2");
    place_text(&mut rt, [300.0, 300.0], "Undo");

    let annotation = rt
        .state
        .document()
        .annotations()
        .first()
        .expect("the text is committed")
        .clone();
    let AnnotationKind::Text {
        position,
        text,
        size_px,
    } = &annotation.kind
    else {
        panic!("expected text, got {:?}", annotation.kind);
    };
    assert!(near(*position, [300.0, 300.0]));
    assert_eq!(text, "Undo");
    assert!(
        (size_px - 28.0 / fit).abs() < 0.01,
        "the large size is stored at the fit scale: {size_px}"
    );
    assert_eq!(annotation.color, PALETTE[4], "the blue palette colour");
    assert!(
        (annotation.stroke_px - 4.0 / fit).abs() < 0.01,
        "the default width is stored: {}",
        annotation.stroke_px
    );

    let committed_png = rt.state.export_png().expect("the export succeeds");
    let committed = decode_png(&committed_png);
    let ink = support::pixel_bounds(&committed, |px| is_palette_color(px, PALETTE[4]))
        .expect("the blue text is exported");
    assert!(
        committed.enumerate_pixels().any(|(_, _, px)| is_halo(px.0)),
        "the exported text carries its halo"
    );

    undo(&mut rt);
    assert!(rt.state.document().is_empty(), "undo removes the text");
    assert!(rt.state.can_redo());
    let undone = export(&mut rt);
    assert!(
        !undone
            .enumerate_pixels()
            .any(|(_, _, px)| is_palette_color(px.0, PALETTE[4])),
        "the blue fill is gone: {ink:?}"
    );
    assert!(
        !undone.enumerate_pixels().any(|(_, _, px)| is_halo(px.0)),
        "the halo is gone with the text"
    );

    redo(&mut rt);
    assert_eq!(
        rt.state.document().annotations(),
        &[annotation],
        "redo restores the same text annotation"
    );
    assert_eq!(
        rt.state.export_png().expect("the export succeeds"),
        committed_png,
        "redo restores the fill and halo pixels"
    );
}

#[test]
fn a_text_re_edit_can_be_undone_and_redone() {
    let dir = TempDir::new("undo-text-reedit");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [300.0, 300.0], "Hello");
    let original = rt.state.document().annotations()[0].clone();
    let id = original.id;
    let before_png = rt.state.export_png().expect("the export succeeds");

    rt.key(key_text("v"));
    rt.frame(0.0);
    double_click(&mut rt, [330.0, 315.0]);
    settle(&mut rt);
    assert_eq!(
        text_field(&rt).as_deref(),
        Some("Hello"),
        "the field reopens"
    );

    press(&mut rt, "text-size-2");
    rt.paste("!");
    rt.frame(0.0);
    rt.key(enter());
    rt.frame(0.0);
    let after_png = rt.state.export_png().expect("the export succeeds");
    assert_ne!(before_png, after_png, "the re-edit changes the pixels");
    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 1, "the re-edit updates in place");
    assert_eq!(annotations[0].id, id, "the id is unchanged");
    let AnnotationKind::Text {
        text,
        size_px,
        position,
    } = &annotations[0].kind
    else {
        panic!("expected text, got {:?}", annotations[0].kind);
    };
    assert_eq!(text, "Hello!");
    assert!(
        (size_px - 28.0 / rt.state.fit_scale()).abs() < 0.01,
        "the re-edit takes the large size: {size_px}"
    );
    assert!(near(*position, [300.0, 300.0]), "the anchor stays put");
    let edited = annotations[0].clone();

    undo(&mut rt);
    assert_eq!(
        rt.state.document().annotations(),
        &[original],
        "undo restores the string, size, position, colour, and id"
    );
    assert_eq!(
        rt.state.export_png().expect("the export succeeds"),
        before_png,
        "undo restores the original pixels"
    );

    redo(&mut rt);
    assert_eq!(
        rt.state.document().annotations(),
        &[edited],
        "redo restores the edited annotation"
    );
    assert_eq!(
        rt.state.export_png().expect("the export succeeds"),
        after_png,
        "redo restores the edited pixels"
    );
}

#[test]
fn emptying_a_re_edited_text_can_be_undone_and_redone() {
    let dir = TempDir::new("undo-text-empty");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [300.0, 300.0], "Hello");
    let text_png = rt.state.export_png().expect("the export succeeds");
    rt.key(key_text("v"));
    rt.frame(0.0);
    double_click(&mut rt, [330.0, 315.0]);
    settle(&mut rt);
    for _ in 0..5 {
        rt.key(KeyEvent {
            key: Key::Backspace,
            mods: Mods::default(),
        });
        rt.frame(0.0);
    }
    assert_eq!(text_field(&rt).as_deref(), Some(""), "the field is empty");
    rt.key(enter());
    rt.frame(0.0);
    assert!(
        rt.state.document().is_empty(),
        "an empty re-edit removes it"
    );
    assert!(rt.state.can_undo(), "the removal is one edit");

    undo(&mut rt);
    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 1, "undo brings the text back");
    let AnnotationKind::Text { text, .. } = &annotations[0].kind else {
        panic!("expected text, got {:?}", annotations[0].kind);
    };
    assert_eq!(text, "Hello");
    assert_eq!(
        rt.state.export_png().expect("the export succeeds"),
        text_png,
        "undo restores the removed text pixels"
    );

    redo(&mut rt);
    assert!(
        rt.state.document().is_empty(),
        "redo removes the text again"
    );
    assert!(
        !has_annotation_red(&export(&mut rt)),
        "the redone removal leaves no pixels"
    );
}

#[test]
fn deleting_a_text_annotation_can_be_undone_and_redone() {
    let dir = TempDir::new("undo-delete");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [300.0, 300.0], "Hello");
    let annotation = rt.state.document().annotations()[0].clone();
    let text_png = rt.state.export_png().expect("the export succeeds");
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [330.0, 315.0]);
    assert_eq!(rt.state.selected(), Some(annotation.id));

    rt.key(KeyEvent {
        key: Key::Delete,
        mods: Mods::default(),
    });
    rt.frame(0.0);
    assert!(rt.state.document().is_empty(), "delete removes the text");
    assert!(rt.state.can_undo(), "the delete is one edit");

    undo(&mut rt);
    assert_eq!(
        rt.state.document().annotations(),
        std::slice::from_ref(&annotation),
        "undo restores the same text, id included"
    );
    assert_eq!(
        rt.state.export_png().expect("the export succeeds"),
        text_png,
        "undo restores the deleted pixels"
    );

    redo(&mut rt);
    assert!(rt.state.document().is_empty(), "redo deletes it again");
    assert!(
        !has_annotation_red(&export(&mut rt)),
        "the redone delete leaves no pixels"
    );
}

#[test]
fn drawing_each_kind_is_undoable_and_redoable() {
    let dir = TempDir::new("undo-draws");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    let line = [100u32, 120, 500, 180];
    let arrow = [100u32, 240, 500, 300];
    let rectangle = [100u32, 360, 500, 480];
    let ellipse = [100u32, 540, 500, 660];
    let redaction = [100u32, 720, 500, 840];
    let text = [900u32, 100, 1150, 260];

    for (tool, from, to) in [
        ("l", [100.0, 120.0], [500.0, 120.0]),
        ("a", [100.0, 240.0], [500.0, 240.0]),
        ("r", [100.0, 360.0], [500.0, 480.0]),
        ("o", [100.0, 540.0], [500.0, 660.0]),
        ("x", [100.0, 720.0], [500.0, 840.0]),
    ] {
        rt.key(key_text(tool));
        rt.frame(0.0);
        drag(&mut rt, from, to);
    }
    place_text(&mut rt, [900.0, 150.0], "Undo");

    assert_eq!(
        rt.state.document().annotations().len(),
        6,
        "one annotation of each kind is committed"
    );
    let original = rt.state.document().annotations().to_vec();
    let full_png = rt.state.export_png().expect("the export succeeds");
    let full = decode_png(&full_png);
    assert!(has_red_in(&full, [250, 100, 350, 140]), "line ink");
    assert!(has_red_in(&full, [250, 220, 350, 260]), "arrow ink");
    assert!(has_red_in(&full, [250, 340, 350, 380]), "rectangle ink");
    assert!(has_red_in(&full, [480, 560, 520, 640]), "ellipse ink");
    assert!(has_black_in(&full, [250, 740, 350, 820]), "redaction ink");
    assert!(has_red_in(&full, text), "text ink");

    for (remaining, region, label) in [
        (5, text, "text"),
        (4, redaction, "redaction"),
        (3, ellipse, "ellipse"),
        (2, rectangle, "rectangle"),
        (1, arrow, "arrow"),
    ] {
        undo(&mut rt);
        assert_eq!(
            rt.state.document().annotations().len(),
            remaining,
            "undo removes the {label}"
        );
        let image = export(&mut rt);
        let still_there = if label == "redaction" {
            has_black_in(&image, region)
        } else {
            has_red_in(&image, region)
        };
        assert!(!still_there, "the undone {label} leaves no ink");
    }

    undo(&mut rt);
    assert!(rt.state.document().is_empty(), "the last draw is undone");
    assert!(
        !has_annotation_red(&export(&mut rt)),
        "an empty document exports no ink"
    );
    assert!(!rt.state.can_undo(), "the undo stack is empty");
    assert!(rt.state.can_redo(), "all six draws are redoable");

    for (restored, region, label) in [
        (1, line, "line"),
        (2, arrow, "arrow"),
        (3, rectangle, "rectangle"),
        (4, ellipse, "ellipse"),
        (5, redaction, "redaction"),
        (6, text, "text"),
    ] {
        redo(&mut rt);
        assert_eq!(
            rt.state.document().annotations().len(),
            restored,
            "redo restores the {label}"
        );
        let image = export(&mut rt);
        let ink = if label == "redaction" {
            has_black_in(&image, region)
        } else {
            has_red_in(&image, region)
        };
        assert!(ink, "the redone {label} paints its pixels");
    }

    assert_eq!(
        rt.state.document().annotations(),
        original.as_slice(),
        "redo restores every kind, style, and id"
    );
    assert_eq!(
        rt.state.export_png().expect("the export succeeds"),
        full_png,
        "redo restores the full export"
    );
}

#[test]
fn a_new_edit_after_undo_clears_the_redo_stack() {
    let dir = TempDir::new("undo-redo-cleared");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("r"));
    rt.frame(0.0);
    drag(&mut rt, [200.0, 200.0], [500.0, 400.0]);
    undo(&mut rt);
    assert!(rt.state.document().is_empty());
    assert!(rt.state.can_redo(), "the draw can be redone");

    drag(&mut rt, [600.0, 600.0], [900.0, 800.0]);
    assert!(
        !rt.state.can_redo(),
        "the new draw discards the undone branch"
    );
    redo(&mut rt);
    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 1, "redo does nothing after a new edit");
    let (start, end) = rectangle_geometry(&rt);
    assert!(
        near(start, [600.0, 600.0]) && near(end, [900.0, 800.0]),
        "the new rectangle is the one that remains: {start:?} {end:?}"
    );

    undo(&mut rt);
    assert!(
        rt.state.document().is_empty(),
        "undo removes the new rectangle"
    );
    assert!(!rt.state.can_undo(), "the old branch is gone");
}

#[test]
fn undo_and_redo_restore_the_color_and_width_that_were_drawn() {
    let dir = TempDir::new("undo-style");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    let fit = rt.state.fit_scale();
    press(&mut rt, "color-4");
    press(&mut rt, "width-2");
    rt.key(key_text("l"));
    rt.frame(0.0);
    drag(&mut rt, [100.0, 100.0], [500.0, 300.0]);

    let drawn = rt.state.document().annotations()[0].clone();
    assert_eq!(drawn.color, PALETTE[4], "the line is blue");
    assert!(
        (drawn.stroke_px - 8.0 / fit).abs() < 0.01,
        "the line is thick: {}",
        drawn.stroke_px
    );

    undo(&mut rt);
    assert!(rt.state.document().is_empty());
    press(&mut rt, "color-3");
    press(&mut rt, "width-0");
    assert_eq!(rt.state.color(), PALETTE[3]);
    assert_eq!(rt.state.width_pt(), 2.0);
    assert!(
        rt.state.can_redo(),
        "changing the palette does not discard the undone edit"
    );

    redo(&mut rt);
    assert_eq!(
        rt.state.document().annotations(),
        &[drawn],
        "redo restores the drawn colour and width, not the palette's current ones"
    );

    undo(&mut rt);
    rt.key(key_text("l"));
    rt.frame(0.0);
    drag(&mut rt, [100.0, 500.0], [500.0, 700.0]);
    let fresh = rt.state.document().annotations()[0].clone();
    assert_eq!(fresh.color, PALETTE[3], "the new line uses the new palette");
    assert!(
        (fresh.stroke_px - 2.0 / fit).abs() < 0.01,
        "the new line uses the new width: {}",
        fresh.stroke_px
    );
    assert!(!rt.state.can_redo(), "the new line clears the redo stack");
    redo(&mut rt);
    assert_eq!(
        rt.state.document().annotations(),
        std::slice::from_ref(&fresh),
        "redo cannot bring back the old blue line"
    );
}

#[test]
fn selection_and_cancelled_text_do_not_create_history() {
    let dir = TempDir::new("undo-noop-selection");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    click(&mut rt, [400.0, 400.0]);
    assert!(
        !rt.state.can_undo(),
        "a click on empty canvas is not an edit"
    );

    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [400.0, 400.0]);
    rt.paste("Oops");
    rt.frame(0.0);
    rt.key(escape());
    rt.frame(0.0);
    assert!(!rt.state.can_undo(), "a cancelled text is not an edit");

    click(&mut rt, [400.0, 400.0]);
    rt.key(enter());
    rt.frame(0.0);
    assert!(!rt.state.can_undo(), "an empty text commit is not an edit");

    rt.key(key_text("l"));
    rt.frame(0.0);
    drag(&mut rt, [100.0, 120.0], [500.0, 120.0]);
    assert!(rt.state.can_undo(), "the draw is the first edit");

    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [300.0, 120.0]);
    assert!(rt.state.selected().is_some(), "the line is selected");
    undo(&mut rt);
    assert!(
        rt.state.document().is_empty(),
        "undo removes the line; selecting it added no entry"
    );
    assert!(!rt.state.can_undo(), "only the draw was recorded");
}

#[test]
fn zero_size_and_cancelled_drags_do_not_create_history() {
    let dir = TempDir::new("undo-noop-drags");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    for tool in ["l", "a", "r", "o", "x"] {
        rt.key(key_text(tool));
        rt.frame(0.0);
        click(&mut rt, [600.0, 500.0]);
        assert!(
            rt.state.document().is_empty(),
            "a zero-size {tool} drag commits nothing"
        );
        assert!(!rt.state.can_undo(), "a zero-size {tool} drag is no edit");
    }

    // Escape cancels a held draw before release.
    rt.key(key_text("l"));
    rt.frame(0.0);
    let rect = canvas_rect(&rt);
    let image = rt.state.image_size();
    let (x0, y0) = window_point(rect, [200.0, 200.0], image);
    let (x1, y1) = window_point(rect, [500.0, 500.0], image);
    rt.pointer_down(x0, y0);
    rt.frame(0.0);
    rt.pointer_move(x1, y1);
    rt.frame(0.0);
    assert!(
        rt.presented("draft").is_some(),
        "the draft is live before Escape"
    );
    rt.key(escape());
    rt.frame(0.0);
    rt.pointer_up(x1, y1);
    rt.frame(0.0);
    assert_eq!(
        rt.shell_request, None,
        "Escape cancelled the drag instead of closing"
    );
    assert!(
        rt.state.document().is_empty(),
        "the cancelled draw commits nothing"
    );
    assert!(!rt.state.can_undo(), "the cancelled draw is no edit");
}

#[test]
fn a_gesture_that_ends_where_it_began_does_not_create_history() {
    let dir = TempDir::new("undo-noop-gesture");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("r"));
    rt.frame(0.0);
    drag(&mut rt, [600.0, 200.0], [900.0, 400.0]);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [750.0, 300.0]);
    assert!(rt.state.selected().is_some());

    drag_path(&mut rt, &[[750.0, 300.0], [1000.0, 600.0], [750.0, 300.0]]);
    let (start, end) = rectangle_geometry(&rt);
    assert!(
        near(start, [600.0, 200.0]) && near(end, [900.0, 400.0]),
        "the rectangle returns to its start: {start:?} {end:?}"
    );

    undo(&mut rt);
    assert!(
        rt.state.document().is_empty(),
        "the move back to the start added no entry"
    );
    assert!(!rt.state.can_undo(), "only the draw was recorded");
}

#[test]
fn history_belongs_to_one_editor_and_does_not_outlive_it() {
    let dir = TempDir::new("undo-per-editor");
    let fixture = write_fixture(dir.path());

    {
        let (_sink, mut rt) = headless_editor(&fixture);
        rt.key(key_text("r"));
        rt.frame(0.0);
        drag(&mut rt, [200.0, 200.0], [500.0, 400.0]);
        assert!(rt.state.can_undo(), "the first editor has history");
    }

    let (_sink, mut rt) = headless_editor(&fixture);
    assert!(
        !rt.state.can_undo(),
        "a new editor cannot undo the previous editor's work"
    );
    assert!(!rt.state.can_redo());
    undo(&mut rt);
    redo(&mut rt);
    assert!(
        rt.state.document().is_empty(),
        "undo and redo do nothing with no history"
    );
}

#[test]
fn a_focused_text_field_keeps_ctrl_z_for_its_own_history() {
    let dir = TempDir::new("undo-text-focus");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("l"));
    rt.frame(0.0);
    drag(&mut rt, [100.0, 120.0], [500.0, 120.0]);
    assert!(rt.state.can_undo(), "the line is undoable");

    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [400.0, 400.0]);
    rt.paste("Draft");
    rt.frame(0.0);
    assert_eq!(text_field(&rt).as_deref(), Some("Draft"));

    undo(&mut rt);
    assert!(
        text_field(&rt).is_some(),
        "the focused field stays open on Ctrl+Z"
    );
    assert_eq!(
        rt.state.document().annotations().len(),
        1,
        "the field's Ctrl+Z did not undo the document"
    );
    assert!(rt.state.can_undo(), "the document history is untouched");

    rt.key(escape());
    rt.frame(0.0);
    undo(&mut rt);
    assert!(
        rt.state.document().is_empty(),
        "with the field closed, Ctrl+Z undoes the line"
    );
}

#[test]
fn undo_clears_an_in_flight_draft_and_gesture() {
    let dir = TempDir::new("undo-in-flight");
    let fixture = write_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("l"));
    rt.frame(0.0);
    drag(&mut rt, [100.0, 120.0], [500.0, 120.0]);
    assert!(rt.state.can_undo());

    // Hold a new draft, then undo the committed line.
    rt.key(key_text("r"));
    rt.frame(0.0);
    let rect = canvas_rect(&rt);
    let image = rt.state.image_size();
    let (x0, y0) = window_point(rect, [200.0, 200.0], image);
    let (x1, y1) = window_point(rect, [500.0, 400.0], image);
    rt.pointer_down(x0, y0);
    rt.frame(0.0);
    rt.pointer_move(x1, y1);
    rt.frame(0.0);
    assert!(rt.find("draft").is_some());

    undo(&mut rt);
    assert!(rt.state.document().is_empty(), "the line is undone");
    assert!(rt.find("draft").is_none(), "undo drops the in-flight draft");
    assert_eq!(rt.shell_request, None, "the editor stays open");
    rt.pointer_up(x1, y1);
    rt.frame(0.0);
    assert!(
        rt.state.document().is_empty(),
        "releasing the cancelled draft commits nothing"
    );

    // Hold a move, then undo the rectangle it belongs to.
    redo(&mut rt);
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [300.0, 120.0]);
    let (x0, y0) = window_point(rect, [300.0, 120.0], image);
    let (x1, y1) = window_point(rect, [400.0, 220.0], image);
    rt.pointer_down(x0, y0);
    rt.frame(0.0);
    rt.pointer_move(x1, y1);
    rt.frame(0.0);
    assert!(rt.find("live").is_some(), "the move is previewed");

    undo(&mut rt);
    assert!(rt.state.document().is_empty(), "the line is undone again");
    assert!(
        rt.find("live").is_none(),
        "undo drops the in-flight gesture"
    );
    assert_eq!(rt.state.selected(), None, "undo clears the selection");
    rt.pointer_up(x1, y1);
    rt.frame(0.0);
    assert!(
        rt.state.document().is_empty(),
        "releasing the cancelled move commits nothing"
    );
}
