//! Shape tools and the annotation palette, driven through the headless editor seam.

mod support;

use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::clipboard::MemoryClipboardSink;
use gnome_annotation::document::{AnnotationKind, PALETTE, Rgba, STROKE_WIDTHS_PT};
use gnome_annotation::editor::{Editor, EditorOptions, Tool, editor_on_key, editor_view};
use image::RgbaImage;
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Shell, Waker};

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

fn enter() -> KeyEvent {
    KeyEvent {
        key: Key::Enter,
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

fn press(rt: &mut Runtime<Editor>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is present"));
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_up(x, y);
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

/// Press Enter, assert the editor closed, and decode the PNG the clipboard sink received.
fn export(sink: &MemoryClipboardSink, rt: &mut Runtime<Editor>) -> RgbaImage {
    rt.key(enter());
    rt.frame(0.0);
    assert_eq!(
        rt.shell_request,
        Some(Shell::Close),
        "Enter copies and closes"
    );
    let writes = sink.writes();
    assert_eq!(writes.len(), 1, "one PNG reached the clipboard");
    let image = decode_png(&writes[0]);
    assert_eq!(image.dimensions(), (3840, 2160));
    image
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

#[test]
fn line_is_drawn_by_dragging_and_exported() {
    let dir = TempDir::new("line");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("l"));
    rt.frame(0.0);
    assert!(
        drag(&mut rt, [600.0, 400.0], [1800.0, 900.0]),
        "the line is previewed while dragging"
    );

    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 1, "one annotation was committed");
    assert!(
        matches!(annotations[0].kind, AnnotationKind::Line { .. }),
        "the annotation is a line: {:?}",
        annotations[0].kind
    );

    let image = export(&sink, &mut rt);
    let midpoint = image.get_pixel(1200, 650).0;
    assert!(
        is_annotation_red(midpoint),
        "the line paints its midpoint: {midpoint:?}"
    );
}

fn assert_near(actual: [f32; 2], expected: [f32; 2]) {
    assert!(
        (actual[0] - expected[0]).abs() < 1.0 && (actual[1] - expected[1]).abs() < 1.0,
        "expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn rectangle_is_drawn_from_any_corner_and_exported() {
    let dir = TempDir::new("rectangle");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("r"));
    rt.frame(0.0);
    assert!(
        drag(&mut rt, [1200.0, 900.0], [600.0, 400.0]),
        "the rectangle is previewed while dragging"
    );

    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 1);
    let AnnotationKind::Rectangle { start, end } = annotations[0].kind else {
        panic!("expected a rectangle, got {:?}", annotations[0].kind);
    };
    assert_near(start, [600.0, 400.0]);
    assert_near(end, [1200.0, 900.0]);

    let image = export(&sink, &mut rt);
    let top_edge = image.get_pixel(900, 400).0;
    assert!(
        is_annotation_red(top_edge),
        "the rectangle outlines its top edge: {top_edge:?}"
    );
    let interior = image.get_pixel(900, 650).0;
    assert_eq!(
        interior,
        [36, 36, 36, 255],
        "the rectangle is an outline, not a fill"
    );
}

#[test]
fn ellipse_is_drawn_inside_its_bounding_box_and_exported() {
    let dir = TempDir::new("ellipse");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("o"));
    rt.frame(0.0);
    assert!(
        drag(&mut rt, [600.0, 400.0], [1200.0, 900.0]),
        "the ellipse is previewed while dragging"
    );

    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 1);
    assert!(
        matches!(annotations[0].kind, AnnotationKind::Ellipse { .. }),
        "expected an ellipse, got {:?}",
        annotations[0].kind
    );

    let image = export(&sink, &mut rt);
    let rightmost = image.get_pixel(1200, 650).0;
    assert!(
        is_annotation_red(rightmost),
        "the ellipse touches the right of its box: {rightmost:?}"
    );
    let centre = image.get_pixel(900, 650).0;
    assert_eq!(
        centre,
        [36, 36, 36, 255],
        "the ellipse is an outline, not a fill"
    );
}

#[test]
fn redaction_is_opaque_black_with_no_outline() {
    let dir = TempDir::new("redaction");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("x"));
    rt.frame(0.0);
    assert!(
        drag(&mut rt, [600.0, 400.0], [1200.0, 900.0]),
        "the redaction is previewed while dragging"
    );

    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 1);
    assert!(
        matches!(annotations[0].kind, AnnotationKind::Redaction { .. }),
        "expected a redaction, got {:?}",
        annotations[0].kind
    );
    assert_eq!(annotations[0].color, Rgba::BLACK);
    assert_eq!(annotations[0].stroke_px, 0.0);

    let image = export(&sink, &mut rt);
    let corner = image.get_pixel(610, 410).0;
    assert_eq!(corner, [0, 0, 0, 255], "redaction fills opaque black");
    let centre = image.get_pixel(900, 650).0;
    assert_eq!(centre, [0, 0, 0, 255], "redaction is opaque");
    let just_outside = image.get_pixel(1205, 650).0;
    assert_eq!(
        just_outside,
        [72, 72, 72, 255],
        "redaction has no outline outside its rectangle"
    );
}

#[test]
fn a_later_redaction_covers_an_earlier_arrow() {
    let dir = TempDir::new("z-order");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("a"));
    rt.frame(0.0);
    assert!(drag(&mut rt, [600.0, 400.0], [1800.0, 400.0]));

    rt.key(key_text("x"));
    rt.frame(0.0);
    assert!(drag(&mut rt, [1000.0, 300.0], [1400.0, 500.0]));

    let annotations = rt.state.document().annotations();
    assert!(matches!(annotations[0].kind, AnnotationKind::Arrow { .. }));
    assert!(matches!(
        annotations[1].kind,
        AnnotationKind::Redaction { .. }
    ));

    let image = export(&sink, &mut rt);
    let arrow_visible = image.get_pixel(800, 400).0;
    assert!(
        is_annotation_red(arrow_visible),
        "the arrow shows outside the redaction: {arrow_visible:?}"
    );
    let covered = image.get_pixel(1200, 400).0;
    assert_eq!(
        covered,
        [0, 0, 0, 255],
        "the later redaction paints above the earlier arrow"
    );
}

#[test]
fn drags_past_the_image_edge_are_cropped_to_the_capture() {
    let dir = TempDir::new("clipping");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("r"));
    rt.frame(0.0);
    assert!(
        drag(&mut rt, [3000.0, 1000.0], [4200.0, 2400.0]),
        "the draft is shown before it is cropped"
    );

    rt.key(key_text("l"));
    rt.frame(0.0);
    assert!(drag(&mut rt, [3000.0, 500.0], [4200.0, 500.0]));

    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 2);
    let AnnotationKind::Rectangle { start, end } = annotations[0].kind else {
        panic!("expected a rectangle, got {:?}", annotations[0].kind);
    };
    assert_near(start, [3000.0, 1000.0]);
    assert_near(end, [3840.0, 2160.0]);
    let AnnotationKind::Line { start, end } = annotations[1].kind else {
        panic!("expected a line, got {:?}", annotations[1].kind);
    };
    assert_near(start, [3000.0, 500.0]);
    assert_near(end, [3840.0, 500.0]);

    let image = export(&sink, &mut rt);
    let right_edge = image.get_pixel(3835, 1500).0;
    assert!(
        is_annotation_red(right_edge),
        "the rectangle reaches the image edge: {right_edge:?}"
    );
    let line_end = image.get_pixel(3830, 500).0;
    assert!(
        is_annotation_red(line_end),
        "the line is cropped at the image edge: {line_end:?}"
    );
    assert_eq!(
        image.get_pixel(10, 1500).0,
        [36, 36, 36, 255],
        "nothing wraps to the opposite edge"
    );
}

fn near(px: [u8; 4], color: Rgba) -> bool {
    px[3] == 255
        && (i32::from(px[0]) - i32::from(color.r)).abs() <= 4
        && (i32::from(px[1]) - i32::from(color.g)).abs() <= 4
        && (i32::from(px[2]) - i32::from(color.b)).abs() <= 4
}

#[test]
fn six_colours_and_three_widths_apply_to_new_annotations() {
    let dir = TempDir::new("palette");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("l"));
    rt.frame(0.0);

    let fit = rt.state.fit_scale();
    assert!(fit < 1.0, "the 4K capture is fitted into the window: {fit}");

    for (index, color) in PALETTE.iter().enumerate() {
        press(&mut rt, &format!("color-{index}"));
        assert_eq!(
            rt.state.color(),
            *color,
            "swatch {index} selects its colour"
        );
        let y = 300.0 + index as f32 * 220.0;
        assert!(
            drag(&mut rt, [400.0, y], [1600.0, y]),
            "colour {index} line is previewed"
        );
        let annotation = rt.state.document().annotations().last().unwrap();
        assert_eq!(annotation.color, *color, "the new line keeps the colour");
        let AnnotationKind::Line { start, end } = annotation.kind else {
            panic!("expected a line, got {:?}", annotation.kind);
        };
        assert_near(start, [400.0, y]);
        assert_near(end, [1600.0, y]);
    }

    for (index, width_pt) in STROKE_WIDTHS_PT.iter().enumerate() {
        press(&mut rt, &format!("width-{index}"));
        assert_eq!(rt.state.width_pt(), *width_pt);
        let y = 1700.0 + index as f32 * 140.0;
        assert!(drag(&mut rt, [400.0, y], [1600.0, y]));
        let annotation = rt.state.document().annotations().last().unwrap();
        let expected_px = width_pt / fit;
        assert!(
            (annotation.stroke_px - expected_px).abs() < 0.01,
            "width {width_pt} pt becomes {expected_px} px at fit {fit}, stored {}",
            annotation.stroke_px
        );
    }

    // Resizing the window changes the fit, never the stored stroke widths.
    let stored: Vec<f32> = rt
        .state
        .document()
        .annotations()
        .iter()
        .rev()
        .take(3)
        .map(|annotation| annotation.stroke_px)
        .collect();
    rt.set_viewport((900.0, 700.0), 1.0);
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(rt.state.fit_scale() < fit);
    let after: Vec<f32> = rt
        .state
        .document()
        .annotations()
        .iter()
        .rev()
        .take(3)
        .map(|annotation| annotation.stroke_px)
        .collect();
    assert_eq!(after, stored, "stored sizes survive a resize");

    let image = export(&sink, &mut rt);
    for (index, color) in PALETTE.iter().enumerate() {
        let y = 300 + index as u32 * 220;
        let px = image.get_pixel(1000, y).0;
        assert!(
            near(px, *color),
            "colour {index} is in the export at its line: {px:?}"
        );
    }
    for (index, width_pt) in STROKE_WIDTHS_PT.iter().enumerate() {
        let y = 1700 + index as u32 * 140;
        let expected = width_pt / fit;
        let painted = (y - 30..=y + 30)
            .filter(|row| near(image.get_pixel(1000, *row).0, PALETTE[5]))
            .count();
        assert!(
            (painted as f32 - expected).abs() <= 3.0,
            "width {width_pt} pt paints about {expected} px, found {painted}"
        );
    }
}

#[test]
fn the_toolbar_fits_the_default_window() {
    let dir = TempDir::new("toolbar");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, rt) = headless_editor(&fixture);

    let (copy, _) = rt.presented("copy").expect("the copy button is shown");
    assert!(
        copy[0] >= 0.0 && copy[0] + copy[2] <= 1200.0,
        "the copy button is inside the window: {copy:?}"
    );
    for key in [
        "tool-select",
        "tool-line",
        "tool-arrow",
        "tool-rect",
        "tool-ellipse",
        "tool-redaction",
        "tool-text",
        "width-0",
        "width-1",
        "width-2",
        "color-0",
        "color-5",
    ] {
        assert!(rt.presented(key).is_some(), "{key} is shown in the toolbar");
    }
}
