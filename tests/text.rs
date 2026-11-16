//! Text annotations, driven through the headless editor seam: placement, live typing,
//! commit and cancel, re-editing, sizes, halo, movement, and exported pixels.

mod support;

use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::clipboard::MemoryClipboardSink;
use gnome_annotation::document::{AnnotationKind, PALETTE};
use gnome_annotation::editor::{Editor, EditorOptions, Tool, editor_on_key, editor_view};
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Shell, Waker};

use support::{TempDir, decode_png, is_annotation_red, pixel_bounds, write_4k_fixture};

fn noop_waker() -> Waker {
    Arc::new(|| {})
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
    rt.frame(0.0);
    (sink, rt)
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

fn canvas_rect(rt: &Runtime<Editor>) -> [f32; 4] {
    rt.presented("canvas").expect("canvas is laid out").0
}

/// Source-image pixels to window points through the fitted canvas.
fn window_point(rect: [f32; 4], source: [f32; 2], image: [u32; 2]) -> (f32, f32) {
    let scale = rect[2] / image[0] as f32;
    (rect[0] + source[0] * scale, rect[1] + source[1] * scale)
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

fn text_size_px(rt: &Runtime<Editor>) -> f32 {
    let annotation = rt.state.document().annotations().last().expect("text");
    let AnnotationKind::Text { size_px, .. } = annotation.kind else {
        panic!("expected text, got {:?}", annotation.kind);
    };
    size_px
}

#[test]
fn text_sizes_are_chosen_in_points_and_stored_at_the_fit_scale() {
    let dir = TempDir::new("text-sizes");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    let fit = rt.state.fit_scale();
    assert!(fit < 1.0, "the 4K capture is fitted into the window: {fit}");

    // The default is medium: 18 points at the current fit scale.
    place_text(&mut rt, [600.0, 400.0], "M");
    let medium = text_size_px(&rt);
    assert!(
        (medium - 18.0 / fit).abs() < 0.01,
        "medium is 18 pt at fit {fit}: stored {medium}"
    );

    // The small and large buttons apply to the next text.
    press(&mut rt, "text-size-0");
    assert_eq!(rt.state.text_size_pt(), 12.0);
    place_text(&mut rt, [600.0, 800.0], "S");
    let small = text_size_px(&rt);
    assert!(
        (small - 12.0 / fit).abs() < 0.01,
        "small is 12 pt at fit {fit}: stored {small}"
    );

    press(&mut rt, "text-size-2");
    assert_eq!(rt.state.text_size_pt(), 28.0);
    place_text(&mut rt, [600.0, 1200.0], "L");
    let large = text_size_px(&rt);
    assert!(
        (large - 28.0 / fit).abs() < 0.01,
        "large is 28 pt at fit {fit}: stored {large}"
    );

    // A later window size changes the fit, never the stored sizes.
    rt.set_viewport((900.0, 700.0), 1.0);
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(rt.state.fit_scale() < fit);
    let sizes: Vec<f32> = rt
        .state
        .document()
        .annotations()
        .iter()
        .map(|annotation| {
            let AnnotationKind::Text { size_px, .. } = annotation.kind else {
                panic!("expected text");
            };
            size_px
        })
        .collect();
    assert_eq!(sizes, vec![medium, small, large]);

    // Exported glyph height grows with the chosen size.
    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    let height_in_rows = |rows: std::ops::Range<u32>| {
        let mut bounds = [u32::MAX, u32::MAX, 0, 0];
        for (x, y, px) in image.enumerate_pixels() {
            if rows.contains(&y) && is_annotation_red(px.0) {
                bounds[0] = bounds[0].min(x);
                bounds[1] = bounds[1].min(y);
                bounds[2] = bounds[2].max(x);
                bounds[3] = bounds[3].max(y);
            }
        }
        assert!(bounds[0] != u32::MAX, "text ink in rows {rows:?}");
        bounds[3] - bounds[1]
    };
    let small_height = height_in_rows(700..1000);
    let medium_height = height_in_rows(300..600);
    let large_height = height_in_rows(1100..1600);
    assert!(
        small_height < medium_height && medium_height < large_height,
        "glyph height grows with the size: small={small_height} medium={medium_height} large={large_height}"
    );
}

#[test]
fn enter_commits_and_closes_only_the_text_field() {
    let dir = TempDir::new("text-commit");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [800.0, 1200.0]);
    assert_eq!(
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("text-editor"))
            .map(|e| e.focused),
        Some(true),
        "the click places a focused caret"
    );
    rt.paste("Label");
    rt.frame(0.0);
    assert_eq!(text_field(&rt).as_deref(), Some("Label"), "typing is live");

    rt.key(enter());
    rt.frame(0.0);
    assert!(
        text_field(&rt).is_none(),
        "Enter commits and drops the field"
    );
    assert_eq!(rt.state.document().annotations().len(), 1);
    assert!(matches!(
        rt.state.document().annotations()[0].kind,
        AnnotationKind::Text { .. }
    ));

    rt.key(enter());
    rt.frame(0.0);
    assert_eq!(
        rt.shell_request,
        Some(Shell::Close),
        "a second Enter copies and closes"
    );
    let image = decode_png(&sink.writes()[0]);
    assert!(
        pixel_bounds(&image, is_annotation_red).is_some(),
        "the committed label is exported"
    );
}

#[test]
fn escape_cancels_without_an_annotation_or_pixels() {
    let dir = TempDir::new("text-cancel");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [800.0, 1200.0]);
    rt.paste("Oops");
    rt.frame(0.0);
    rt.key(escape());
    rt.frame(0.0);
    assert!(text_field(&rt).is_none(), "Esc drops the field");
    assert!(rt.state.document().is_empty(), "Esc commits nothing");

    rt.key(enter());
    rt.frame(0.0);
    let image = decode_png(&sink.writes()[0]);
    assert!(
        pixel_bounds(&image, is_annotation_red).is_none(),
        "the cancelled text leaves no pixels"
    );
}

#[test]
fn double_click_reopens_text_and_commits_the_same_annotation() {
    let dir = TempDir::new("text-reedit");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [900.0, 1200.0], "Label");
    let id = rt.state.document().annotations()[0].id;
    let before = rt.state.export_png().expect("the first export succeeds");

    rt.key(key_text("v"));
    rt.frame(0.0);
    double_click(&mut rt, [950.0, 1230.0]);
    settle(&mut rt);

    let field = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("text-editor"))
        .expect("double-click reopens the field");
    assert_eq!(
        field.text.as_deref(),
        Some("Label"),
        "the field is pre-filled"
    );
    assert!(field.focused, "the reopened field takes focus");
    assert_eq!(
        rt.state.document().annotations().len(),
        1,
        "re-edit does not duplicate the annotation"
    );
    assert!(
        rt.presented("annotations").is_none(),
        "the committed overlay leaves the edited text out while the field is live"
    );

    rt.paste("2");
    rt.frame(0.0);
    assert_eq!(
        text_field(&rt).as_deref(),
        Some("Label2"),
        "editing updates the field live"
    );

    rt.key(enter());
    rt.frame(0.0);
    assert!(text_field(&rt).is_none(), "Enter closes the field");
    let annotations = rt.state.document().annotations();
    assert_eq!(
        annotations.len(),
        1,
        "the annotation was updated, not added"
    );
    assert_eq!(annotations[0].id, id, "the annotation keeps its id");
    let AnnotationKind::Text { text, .. } = &annotations[0].kind else {
        panic!("expected text");
    };
    assert_eq!(text, "Label2");
    assert!(
        rt.presented("annotations").is_some(),
        "the overlay comes back with the updated text"
    );

    let after = rt.state.export_png().expect("the second export succeeds");
    assert_ne!(before, after, "the exported pixels change with the edit");
    let image = decode_png(&after);
    assert!(
        pixel_bounds(&image, is_annotation_red).is_some(),
        "the edited label is exported"
    );
}

#[test]
fn the_text_tool_double_click_reopens_committed_text_instead_of_stacking_a_label() {
    let dir = TempDir::new("text-reedit-text-tool");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [900.0, 1200.0], "Label");
    let id = rt.state.document().annotations()[0].id;
    assert!(text_field(&rt).is_none(), "the label is committed");
    assert_eq!(rt.state.tool(), Tool::Text, "the text tool is still active");

    // The first click may open an empty field. The double press must cancel it and reopen
    // the committed text instead of committing an empty label and starting another.
    double_click(&mut rt, [950.0, 1230.0]);
    settle(&mut rt);

    let field = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("text-editor") && !e.exiting)
        .expect("double-click reopens the field with the text tool");
    assert_eq!(
        field.text.as_deref(),
        Some("Label"),
        "the field is pre-filled with the committed string"
    );
    assert!(field.focused, "the reopened field takes focus");
    assert_eq!(
        rt.state.document().annotations().len(),
        1,
        "the text tool did not stack a second annotation"
    );

    rt.paste("2");
    rt.frame(0.0);
    assert_eq!(text_field(&rt).as_deref(), Some("Label2"));
    rt.key(enter());
    rt.frame(0.0);

    let annotations = rt.state.document().annotations();
    assert_eq!(
        annotations.len(),
        1,
        "the edit updates the existing annotation"
    );
    assert_eq!(annotations[0].id, id, "the annotation keeps its id");
    let AnnotationKind::Text { text, .. } = &annotations[0].kind else {
        panic!("expected text");
    };
    assert_eq!(text, "Label2");
}

#[test]
fn committing_an_empty_re_edit_removes_the_annotation() {
    let dir = TempDir::new("text-empty");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [900.0, 1200.0], "Label");
    rt.key(key_text("v"));
    rt.frame(0.0);
    double_click(&mut rt, [950.0, 1230.0]);
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
    settle(&mut rt);
    assert!(
        rt.state.document().is_empty(),
        "an empty re-edit removes the annotation"
    );
    assert!(
        rt.presented("annotations").is_none(),
        "the empty annotation leaves the overlay"
    );
    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    assert!(
        pixel_bounds(&image, is_annotation_red).is_none(),
        "no pixels remain after the removal"
    );
}

#[test]
fn escape_restores_the_original_after_re_editing() {
    let dir = TempDir::new("text-reedit-cancel");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [900.0, 1200.0], "Label");
    let before = rt.state.export_png().expect("the first export succeeds");
    rt.key(key_text("v"));
    rt.frame(0.0);
    double_click(&mut rt, [950.0, 1230.0]);
    rt.paste("2");
    rt.frame(0.0);
    rt.key(escape());
    rt.frame(0.0);

    assert!(text_field(&rt).is_none(), "Esc closes the field");
    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 1, "no annotation is added or removed");
    let AnnotationKind::Text { text, .. } = &annotations[0].kind else {
        panic!("expected text");
    };
    assert_eq!(text, "Label", "Esc keeps the original string");
    let after = rt.state.export_png().expect("the second export succeeds");
    assert_eq!(
        before, after,
        "Esc restores the committed overlay without a trace of the edit"
    );
}

#[test]
fn long_text_stays_on_one_line_until_shift_enter() {
    let dir = TempDir::new("text-nowrap");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    let long = "M".repeat(120);
    place_text(&mut rt, [200.0, 300.0], &long);
    let one_line_px = text_size_px(&rt);

    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [200.0, 1200.0]);
    rt.paste("M");
    rt.frame(0.0);
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods {
            shift: true,
            ..Mods::default()
        },
    });
    rt.frame(0.0);
    rt.paste("M");
    rt.frame(0.0);
    rt.key(enter());
    rt.frame(0.0);

    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    let ink_height = |rows: std::ops::Range<u32>| {
        let mut top = u32::MAX;
        let mut bottom = 0;
        for (_, y, px) in image.enumerate_pixels() {
            if rows.contains(&y) && is_annotation_red(px.0) {
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
        assert!(top != u32::MAX, "ink in rows {rows:?}");
        bottom - top + 1
    };
    let one_line = ink_height(200..900);
    let two_lines = ink_height(1100..1900);
    assert!(
        (one_line as f32) < one_line_px * 1.5,
        "120 characters stay one line tall: {one_line} px for a {one_line_px} px font"
    );
    assert!(
        (two_lines as f32) > one_line_px * 1.5,
        "an explicit newline makes two lines: {two_lines} px"
    );
}

#[test]
fn text_is_selected_moved_and_deleted_like_a_shape() {
    let dir = TempDir::new("text-move-delete");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [900.0, 1200.0], "Label");
    let id = rt.state.document().annotations()[0].id;
    rt.key(key_text("v"));
    rt.frame(0.0);
    click(&mut rt, [950.0, 1230.0]);
    assert_eq!(rt.state.selected(), Some(id), "the text is selected");

    let rect = canvas_rect(&rt);
    let image = rt.state.image_size();
    let (x0, y0) = window_point(rect, [950.0, 1230.0], image);
    let (x1, y1) = window_point(rect, [1250.0, 1330.0], image);
    rt.pointer_down(x0, y0);
    rt.frame(0.0);
    rt.pointer_move(x1, y1);
    rt.frame(0.0);
    rt.pointer_up(x1, y1);
    rt.frame(0.0);

    let annotation = &rt.state.document().annotations()[0];
    let AnnotationKind::Text { position, .. } = &annotation.kind else {
        panic!("expected text");
    };
    assert!(
        (position[0] - 1200.0).abs() < 1.0 && (position[1] - 1300.0).abs() < 1.0,
        "the text anchor moves with the drag: {position:?}"
    );
    let moved = decode_png(&rt.state.export_png().expect("the export succeeds"));
    let ink = pixel_bounds(&moved, is_annotation_red).expect("the label is exported");
    assert!(
        ink[0] >= 1150,
        "the exported ink moved with the anchor: {ink:?}"
    );

    rt.key(KeyEvent {
        key: Key::Delete,
        mods: Mods::default(),
    });
    rt.frame(0.0);
    assert_eq!(rt.state.selected(), None, "delete clears the selection");
    assert!(rt.state.document().is_empty(), "the text is removed");
    let empty = decode_png(&rt.state.export_png().expect("the export succeeds"));
    assert!(
        pixel_bounds(&empty, is_annotation_red).is_none(),
        "the deleted text leaves no pixels"
    );
}

#[test]
fn a_size_button_resizes_the_live_field_and_applies_at_commit() {
    let dir = TempDir::new("text-size-edit");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [900.0, 1200.0], "Label");
    let id = rt.state.document().annotations()[0].id;
    let original_size = text_size_px(&rt);
    let fit = rt.state.fit_scale();

    rt.key(key_text("v"));
    rt.frame(0.0);
    double_click(&mut rt, [950.0, 1230.0]);
    settle(&mut rt);

    press(&mut rt, "text-size-2");
    assert_eq!(rt.state.text_size_pt(), 28.0);
    let field = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("text-editor") && !e.exiting)
        .expect("the field stays open");
    assert!(
        field.focused,
        "a size button keeps the text field focused, so typing can continue"
    );
    assert_eq!(
        field.text.as_deref(),
        Some("Label"),
        "the string is untouched"
    );
    assert_eq!(
        rt.state.document().annotations()[0].id,
        id,
        "the size change does not commit or add an annotation"
    );

    rt.key(enter());
    rt.frame(0.0);
    let annotation = &rt.state.document().annotations()[0];
    let AnnotationKind::Text { size_px, .. } = annotation.kind else {
        panic!("expected text");
    };
    assert!(
        (size_px - 28.0 / fit).abs() < 0.01,
        "the chosen size applies at commit: stored {size_px}"
    );
    assert!(
        size_px > original_size,
        "the label grew from {original_size} to {size_px}"
    );
}

#[test]
fn re_editing_after_a_resize_keeps_the_stored_size() {
    let dir = TempDir::new("text-size-preserve");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [900.0, 1200.0], "Label");
    let stored = text_size_px(&rt);
    rt.set_viewport((900.0, 700.0), 1.0);
    rt.frame(0.0);
    rt.frame(0.0);

    rt.key(key_text("v"));
    rt.frame(0.0);
    double_click(&mut rt, [950.0, 1230.0]);
    settle(&mut rt);
    assert_eq!(
        text_field(&rt).as_deref(),
        Some("Label"),
        "the text is reopened at the smaller window"
    );
    rt.key(enter());
    rt.frame(0.0);
    assert_eq!(
        text_size_px(&rt),
        stored,
        "an unchanged re-edit keeps the original source-pixel size"
    );
}

#[test]
fn exported_text_lands_at_its_anchor_with_a_crisp_fill_and_halo() {
    let dir = TempDir::new("text-pixels");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    place_text(&mut rt, [1000.0, 800.0], "M");
    let png = rt.state.export_png().expect("the export succeeds");
    let image = decode_png(&png);
    let ink = pixel_bounds(&image, is_annotation_red).expect("the text ink is exported");
    assert!(
        (600..=1010).contains(&ink[0]) && (790..=820).contains(&ink[1]),
        "the ink starts at the anchor: {ink:?}"
    );

    let mut fill = false;
    let mut halo = false;
    for (x, y, px) in image.enumerate_pixels() {
        if x + 4 < ink[0] || x > ink[2] + 4 || y + 4 < ink[1] || y > ink[3] + 4 {
            continue;
        }
        let [r, g, b, a] = px.0;
        fill |= r > 150 && g < 90 && b < 90 && a == 255;
        halo |= r > 220 && g > 220 && b > 220 && a == 255;
    }
    assert!(fill, "opaque fill pixels sit at the anchor");
    assert!(halo, "a white halo surrounds the dark fill");
}

#[test]
fn size_buttons_show_the_selected_size_and_fit_the_window() {
    let dir = TempDir::new("text-size-buttons");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    let fill = |rt: &Runtime<Editor>, key: &str| {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("{key} is present"))
            .fill
    };
    let medium_fill = fill(&rt, "text-size-1");
    for key in ["text-size-0", "text-size-1", "text-size-2"] {
        let (rect, _) = rt
            .presented(key)
            .unwrap_or_else(|| panic!("{key} is shown"));
        assert!(
            rect[0] >= 0.0 && rect[0] + rect[2] <= 1200.0,
            "{key} fits inside the window: {rect:?}"
        );
    }
    assert_ne!(
        medium_fill,
        fill(&rt, "text-size-0"),
        "the default medium button is visibly selected"
    );

    press(&mut rt, "text-size-2");
    settle(&mut rt);
    assert_eq!(rt.state.text_size_pt(), 28.0);
    assert_eq!(
        fill(&rt, "text-size-2"),
        medium_fill,
        "the selected button takes the accent fill"
    );
    assert_ne!(
        fill(&rt, "text-size-1"),
        medium_fill,
        "the previously selected button loses it"
    );
}

#[test]
fn clicking_the_canvas_commits_the_current_text_and_starts_another() {
    let dir = TempDir::new("text-chain");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [600.0, 400.0]);
    rt.paste("One");
    rt.frame(0.0);

    click(&mut rt, [600.0, 800.0]);
    assert_eq!(
        text_field(&rt).as_deref(),
        Some(""),
        "the next click starts a fresh empty field"
    );
    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 1, "the first label was committed");
    let AnnotationKind::Text { text, .. } = &annotations[0].kind else {
        panic!("expected text");
    };
    assert_eq!(text, "One");

    rt.paste("Two");
    rt.frame(0.0);
    rt.key(enter());
    rt.frame(0.0);
    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 2, "the second label commits separately");
    let AnnotationKind::Text { text, .. } = &annotations[1].kind else {
        panic!("expected text");
    };
    assert_eq!(text, "Two");
}

#[test]
fn a_toolbar_swatch_keeps_the_text_field_focused_for_escape() {
    let dir = TempDir::new("text-focus-escape");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [900.0, 1200.0]);
    rt.paste("Label");
    rt.frame(0.0);

    press(&mut rt, "color-3");
    let field = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("text-editor") && !e.exiting)
        .expect("the field is still open");
    assert!(
        field.focused,
        "the colour swatch keeps the field focused so Esc still reaches it"
    );

    rt.key(escape());
    rt.frame(0.0);
    assert!(text_field(&rt).is_none(), "Esc cancels the edit");
    assert!(
        rt.state.document().is_empty(),
        "the cancelled label is not added"
    );
}

#[test]
fn a_toolbar_swatch_keeps_the_text_field_focused_for_enter() {
    let dir = TempDir::new("text-focus-enter");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [900.0, 1200.0]);
    rt.paste("Label");
    rt.frame(0.0);

    press(&mut rt, "color-3");
    rt.key(enter());
    rt.frame(0.0);

    let annotations = rt.state.document().annotations();
    assert_eq!(annotations.len(), 1, "Enter commits the label");
    let AnnotationKind::Text { text, .. } = &annotations[0].kind else {
        panic!("expected text");
    };
    assert_eq!(text, "Label");
    assert_eq!(
        annotations[0].color, PALETTE[3],
        "the label takes the colour picked while typing"
    );
}

#[test]
fn shift_enter_adds_a_line_before_commit() {
    let dir = TempDir::new("text-newline");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [800.0, 1200.0]);
    rt.paste("First");
    rt.frame(0.0);
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods {
            shift: true,
            ..Mods::default()
        },
    });
    rt.frame(0.0);
    rt.paste("Second");
    rt.frame(0.0);
    assert_eq!(
        text_field(&rt).as_deref(),
        Some("First\nSecond"),
        "Shift+Enter adds a line instead of committing"
    );

    rt.key(enter());
    rt.frame(0.0);
    let annotation = &rt.state.document().annotations()[0];
    let AnnotationKind::Text { text, .. } = &annotation.kind else {
        panic!("expected text");
    };
    assert_eq!(text, "First\nSecond");
}
