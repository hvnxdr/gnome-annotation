//! Responsive editor layout through the headless Rune runtime.

mod support;

use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::document::AnnotationKind;
use gnome_annotation::editor::{Editor, EditorOptions, editor_on_key, editor_view};
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Waker};

use support::fakes::MemoryClipboardSink;
use support::{TempDir, write_4k_fixture, write_png};

const STYLE_KEYS: [&str; 12] = [
    "color-0",
    "color-1",
    "color-2",
    "color-3",
    "color-4",
    "color-5",
    "width-0",
    "width-1",
    "width-2",
    "text-size-0",
    "text-size-1",
    "text-size-2",
];

fn noop_waker() -> Waker {
    Arc::new(|| {})
}

fn headless_editor(path: &std::path::Path) -> (MemoryClipboardSink, Runtime<Editor>) {
    headless_editor_at(path, (1200.0, 800.0), 1.0)
}

fn headless_editor_at(
    path: &std::path::Path,
    viewport: (f32, f32),
    scale: f32,
) -> (MemoryClipboardSink, Runtime<Editor>) {
    let sink = MemoryClipboardSink::new();
    let options = EditorOptions::new(path, Arc::new(sink.clone()));
    let mut editor = Editor::new(options).expect("load capture");
    editor.setup(noop_waker());
    let mut rt = Runtime::with_text(
        editor,
        editor_view,
        viewport,
        Env::new(true, scale),
        TextSystem::new(FONTS, ("Inter", "Inter")),
    );
    rt.on_key = Some(editor_on_key);
    rt.frame(0.0);
    rt.frame(0.0);
    (sink, rt)
}

fn settle(rt: &mut Runtime<Editor>) {
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
}

fn begin_rect_drag(rt: &mut Runtime<Editor>, x0: f32, y0: f32, x1: f32, y1: f32) {
    rt.pointer_down(x0, y0);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x1, y1);
    rt.frame(1.0 / 60.0);
}

fn click_at(rt: &mut Runtime<Editor>, x: f32, y: f32) {
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
    settle(rt);
}

fn first_annotation_kind(rt: &Runtime<Editor>) -> &AnnotationKind {
    &rt.state.document().annotations()[0].kind
}

fn first_annotation_handles(rt: &Runtime<Editor>) -> Vec<[f32; 2]> {
    first_annotation_kind(rt).handles()
}

fn live_rect(rt: &Runtime<Editor>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(key) && !element.exiting)
        .unwrap_or_else(|| panic!("{key} is present"))
        .rect
}

fn contained_by(inner: [f32; 4], outer: [f32; 4]) -> bool {
    inner[0] >= outer[0]
        && inner[1] >= outer[1]
        && inner[0] + inner[2] <= outer[0] + outer[2]
        && inner[1] + inner[3] <= outer[1] + outer[3]
}

fn intersects(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && a[0] + a[2] > b[0] && a[1] < b[1] + b[3] && a[1] + a[3] > b[1]
}

#[test]
fn toolbar_reflows_without_hiding_primary_actions() {
    let dir = TempDir::new("responsive-toolbar");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    let expanded = rt.presented("toolbar").expect("expanded toolbar").0;
    assert!(
        expanded[3] <= 56.01,
        "expanded toolbar stays one row: {expanded:?}"
    );
    assert!(rt.find("shortcut-hint").is_some());
    assert!(rt.find("toolbar-styles").is_none());

    rt.set_viewport((900.0, 700.0), 1.0);
    settle(&mut rt);
    assert!(rt.presented("toolbar-main").is_some());
    assert!(rt.presented("toolbar-styles").is_some());
    assert!(rt.find("shortcut-hint").is_none());
    for key in ["save", "copy", "color-0", "width-0", "text-size-0"] {
        let rect = live_rect(&rt, key);
        assert!(
            contained_by(rect, [0.0, 0.0, 900.0, 700.0]),
            "{key} is inside the window: {rect:?}"
        );
    }

    rt.set_viewport((480.0, 360.0), 1.0);
    settle(&mut rt);
    for key in ["save", "copy"] {
        let rect = live_rect(&rt, key);
        assert!(
            contained_by(rect, [0.0, 0.0, 480.0, 360.0]),
            "{key} is inside the window: {rect:?}"
        );
    }
    for key in STYLE_KEYS {
        let rect = live_rect(&rt, key);
        assert!(
            contained_by(rect, [0.0, 0.0, 480.0, 360.0]),
            "{key} stays visible in the narrow window: {rect:?}"
        );
    }

    rt.key(KeyEvent {
        key: Key::Text("t".into()),
        mods: Mods::default(),
    });
    settle(&mut rt);
    let strip = live_rect(&rt, "tool-strip");
    assert!(
        strip[2] > 300.0,
        "tool strip uses the free width: {strip:?}"
    );
    for (shortcut, key) in [
        ("v", "tool-select"),
        ("l", "tool-line"),
        ("a", "tool-arrow"),
        ("r", "tool-rect"),
        ("o", "tool-ellipse"),
        ("x", "tool-redaction"),
        ("t", "tool-text"),
    ] {
        rt.key(KeyEvent {
            key: Key::Text(shortcut.into()),
            mods: Mods::default(),
        });
        settle(&mut rt);
        let tool = live_rect(&rt, key);
        assert!(
            contained_by(tool, strip),
            "{shortcut} tool is fully revealed: {tool:?} in {strip:?}"
        );
    }
    assert!(rt.find("canvas").is_some());
}

#[test]
fn very_small_displays_wrap_the_style_row() {
    let dir = TempDir::new("responsive-tiny");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor_at(&fixture, (400.0, 300.0), 1.0);
    settle(&mut rt);

    let styles = live_rect(&rt, "toolbar-styles");
    assert!(styles[3] > 38.0, "style controls wrap: {styles:?}");
    for key in STYLE_KEYS {
        let rect = live_rect(&rt, key);
        assert!(
            contained_by(rect, [0.0, 0.0, 400.0, 300.0]),
            "{key} stays visible at 400 pt: {rect:?}"
        );
    }
}

#[test]
fn errors_wrap_in_a_dismissible_status_banner() {
    let dir = TempDir::new("responsive-error");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);
    let message = "The clipboard rejected this PNG because it exceeds the image size limit. Choose a smaller capture or save the annotated PNG to disk instead.";
    sink.fail_writes(Some(message));

    rt.set_viewport((480.0, 360.0), 1.0);
    settle(&mut rt);
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods::default(),
    });
    settle(&mut rt);

    assert_eq!(rt.state.error(), Some(message));
    let banner = live_rect(&rt, "error-banner");
    let error = live_rect(&rt, "error");
    assert!(
        contained_by(banner, [0.0, 0.0, 480.0, 360.0]),
        "banner fits: {banner:?}"
    );
    assert!(
        error[2] <= banner[2],
        "error text stays in the banner: {error:?}"
    );
    assert!(error[3] > 20.0, "long error wraps: {error:?}");

    let (x, y) = rt.find("error-dismiss").expect("dismiss control");
    click_at(&mut rt, x, y);
    assert_eq!(rt.state.error(), None);
    assert!(rt.find("error-banner").is_none());
}

#[test]
fn fit_scale_changes_cancel_drafts_and_selection_moves() {
    let dir = TempDir::new("responsive-gestures");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);
    let image = rt.state.image_size();

    let point = |rt: &Runtime<Editor>, source: [f32; 2]| {
        let rect = live_rect(rt, "canvas");
        let scale = rect[2] / image[0] as f32;
        (rect[0] + source[0] * scale, rect[1] + source[1] * scale)
    };

    rt.key(KeyEvent {
        key: Key::Text("r".into()),
        mods: Mods::default(),
    });
    let (x0, y0) = point(&rt, [600.0, 400.0]);
    let (x1, y1) = point(&rt, [1600.0, 1200.0]);
    begin_rect_drag(&mut rt, x0, y0, x1, y1);
    assert!(rt.find("draft").is_some());

    rt.set_viewport((900.0, 700.0), 1.0);
    settle(&mut rt);
    rt.pointer_move(x1, y1);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x1, y1);
    settle(&mut rt);
    assert!(
        rt.state.document().annotations().is_empty(),
        "resize cancels the draft"
    );
    assert!(rt.find("draft").is_none());

    let (x0, y0) = point(&rt, [600.0, 400.0]);
    let (x1, y1) = point(&rt, [1600.0, 1200.0]);
    begin_rect_drag(&mut rt, x0, y0, x1, y1);
    rt.pointer_up(x1, y1);
    settle(&mut rt);
    let original = first_annotation_handles(&rt);

    rt.key(KeyEvent {
        key: Key::Text("v".into()),
        mods: Mods::default(),
    });
    let (x, y) = point(&rt, [1100.0, 800.0]);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x + 40.0, y + 20.0);
    rt.frame(1.0 / 60.0);
    assert!(rt.find("live").is_some());

    rt.set_viewport((640.0, 480.0), 1.0);
    settle(&mut rt);
    rt.pointer_move(x + 40.0, y + 20.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x + 40.0, y + 20.0);
    settle(&mut rt);
    assert_eq!(
        first_annotation_handles(&rt),
        original,
        "resize cancels the selection gesture without changing geometry"
    );
    assert!(rt.find("live").is_none());
}

#[test]
fn live_text_stays_unwrapped_at_the_capture_edge() {
    let dir = TempDir::new("responsive-text");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);
    rt.set_viewport((480.0, 360.0), 1.0);
    settle(&mut rt);

    rt.key(KeyEvent {
        key: Key::Text("t".into()),
        mods: Mods::default(),
    });
    let canvas = live_rect(&rt, "canvas");
    let scale = canvas[2] / rt.state.image_size()[0] as f32;
    let x = canvas[0] + 3700.0 * scale;
    let y = canvas[1] + 1000.0 * scale;
    click_at(&mut rt, x, y);
    rt.paste("A long label that must scroll sideways instead of wrapping onto another line");
    settle(&mut rt);

    let field = live_rect(&rt, "text-editor");
    assert!(
        field[0] + field[2] <= canvas[0] + canvas[2] + 0.5,
        "field fits: {field:?} in {canvas:?}"
    );
    assert!(
        field[3] < 32.0,
        "long text keeps one visual line: {field:?}"
    );
}

#[test]
fn keyboard_focus_reveals_narrow_tools() {
    let dir = TempDir::new("responsive-focus");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor_at(&fixture, (480.0, 360.0), 1.0);
    settle(&mut rt);
    rt.set_tab_visits_controls(true);

    for _ in 0..7 {
        rt.key(KeyEvent {
            key: Key::Tab,
            mods: Mods::default(),
        });
        rt.frame(1.0 / 60.0);
    }
    settle(&mut rt);
    assert!(
        rt.elements()
            .into_iter()
            .any(|element| { element.focused && element.key.as_deref() == Some("tool-text") })
    );
    let strip = live_rect(&rt, "tool-strip");
    let text_tool = live_rect(&rt, "tool-text");
    assert!(
        intersects(text_tool, strip),
        "focused tool is revealed: {text_tool:?} in {strip:?}"
    );
}

#[test]
fn active_text_editing_survives_a_fit_scale_change() {
    let dir = TempDir::new("responsive-text-resize");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor_at(&fixture, (480.0, 360.0), 1.0);
    settle(&mut rt);
    rt.key(KeyEvent {
        key: Key::Text("t".into()),
        mods: Mods::default(),
    });
    let canvas = live_rect(&rt, "canvas");
    let scale = canvas[2] / rt.state.image_size()[0] as f32;
    let x = canvas[0] + 1200.0 * scale;
    let y = canvas[1] + 800.0 * scale;
    click_at(&mut rt, x, y);
    rt.paste("Before resize");
    settle(&mut rt);

    rt.set_viewport((900.0, 700.0), 1.0);
    settle(&mut rt);
    let field = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some("text-editor") && !element.exiting)
        .expect("text field stays open");
    assert_eq!(field.text.as_deref(), Some("Before resize"));

    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods::default(),
    });
    settle(&mut rt);
    let AnnotationKind::Text { text, .. } = first_annotation_kind(&rt) else {
        panic!("expected a text annotation");
    };
    assert_eq!(text, "Before resize");
}

#[test]
fn capture_aspect_ratios_fit_the_available_canvas() {
    let dir = TempDir::new("responsive-aspects");
    for (name, width, height, viewport) in [
        ("portrait", 1080, 1920, (900.0, 700.0)),
        ("square", 800, 800, (900.0, 700.0)),
        ("ultrawide", 2560, 1080, (900.0, 700.0)),
        ("portrait-minimum", 1080, 1920, (480.0, 800.0)),
        ("ultrawide-minimum", 2560, 1080, (1200.0, 360.0)),
    ] {
        let path = dir.path().join(format!("{name}.png"));
        write_png(
            &path,
            width,
            height,
            &vec![128; (width * height * 4) as usize],
        );
        let (_sink, mut rt) = headless_editor_at(&path, viewport, 1.0);
        settle(&mut rt);
        let canvas = live_rect(&rt, "canvas");
        let stage = live_rect(&rt, "stage");
        let expected = width as f32 / height as f32;
        let actual = canvas[2] / canvas[3];
        assert!((actual - expected).abs() < 0.001, "{name}: {canvas:?}");
        assert!(contained_by(canvas, stage));
    }
}

#[test]
fn narrow_layout_uses_logical_points_at_two_x() {
    let dir = TempDir::new("responsive-hidpi");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor_at(&fixture, (480.0, 360.0), 2.0);
    settle(&mut rt);

    assert!(rt.find("tool-strip").is_some());
    let select = live_rect(&rt, "tool-select");
    assert_eq!(select[3], 32.0, "controls stay 32 logical points at 2x");
    for key in ["save", "copy"] {
        assert!(contained_by(live_rect(&rt, key), [0.0, 0.0, 480.0, 360.0]));
    }
}
