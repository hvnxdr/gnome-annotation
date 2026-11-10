//! The editor's headless seam: a real Rune runtime, an explicit image path, and a
//! memory clipboard sink.

mod support;

use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::clipboard::MemoryClipboardSink;
use gnome_annotation::editor::{Editor, EditorOptions, editor_on_key, editor_view};
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Shell, Waker};

use support::{TempDir, decode_png, is_annotation_red, pixel_bounds, write_4k_fixture};

fn noop_waker() -> Waker {
    Arc::new(|| {})
}

fn enter() -> KeyEvent {
    KeyEvent {
        key: Key::Enter,
        mods: Mods::default(),
    }
}

fn key_text(c: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Text(c.to_string()),
        mods: Mods::default(),
    }
}

fn headless_editor_with(
    path: &std::path::Path,
    clipboard: Arc<dyn gnome_annotation::clipboard::ClipboardSink>,
) -> Runtime<Editor> {
    let options = EditorOptions::new(path, clipboard);
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
    rt
}

fn headless_editor(path: &std::path::Path) -> (MemoryClipboardSink, Runtime<Editor>) {
    let sink = MemoryClipboardSink::new();
    let rt = headless_editor_with(path, Arc::new(sink.clone()));
    (sink, rt)
}

#[test]
fn copies_an_unannotated_capture_at_source_size() {
    let dir = TempDir::new("plain-copy");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    let (canvas, _) = rt.presented("canvas").expect("canvas is laid out");
    assert!(
        canvas[2] <= 1200.0 && canvas[3] <= 800.0,
        "canvas fits the window: {canvas:?}"
    );
    assert!(
        canvas[2] < 3840.0 && canvas[3] < 2160.0,
        "canvas is fitted, not source sized: {canvas:?}"
    );

    rt.key(enter());
    rt.frame(0.0);

    assert_eq!(
        rt.shell_request,
        Some(Shell::Close),
        "Enter closes the editor"
    );
    let writes = sink.writes();
    assert_eq!(writes.len(), 1, "one PNG reached the clipboard");
    let image = decode_png(&writes[0]);
    assert_eq!(image.dimensions(), (3840, 2160));
}

#[test]
fn tool_keys_switch_the_active_tool() {
    let dir = TempDir::new("tools");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("a"));
    rt.frame(0.0);
    assert_eq!(rt.state.tool(), gnome_annotation::editor::Tool::Arrow);
    rt.key(key_text("t"));
    rt.frame(0.0);
    assert_eq!(rt.state.tool(), gnome_annotation::editor::Tool::Text);
    rt.key(key_text("v"));
    rt.frame(0.0);
    assert_eq!(rt.state.tool(), gnome_annotation::editor::Tool::Select);
}

fn canvas_rect(rt: &Runtime<Editor>) -> [f32; 4] {
    rt.presented("canvas").expect("canvas is laid out").0
}

/// Source-image pixels to window points through the fitted canvas.
fn window_point(rect: [f32; 4], source: [f32; 2], image: [u32; 2]) -> (f32, f32) {
    let scale = rect[2] / image[0] as f32;
    (rect[0] + source[0] * scale, rect[1] + source[1] * scale)
}

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
fn arrow_and_text_are_exported_at_source_positions() {
    let dir = TempDir::new("annotated");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    // One arrow, drawn by dragging with the pointer.
    rt.key(key_text("a"));
    rt.frame(0.0);
    assert!(
        drag(&mut rt, [600.0, 400.0], [1800.0, 900.0]),
        "the arrow is previewed while dragging"
    );

    // One text label: click, type, commit.
    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [800.0, 1400.0]);
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("text-editor") && e.focused),
        "the text field takes focus"
    );
    rt.paste("Label");
    rt.frame(0.0);
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("text-editor") && e.text.as_deref() == Some("Label")),
        "typing shows the live text"
    );
    rt.key(enter());
    rt.frame(0.0);
    assert!(
        !rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("text-editor")),
        "Enter commits the label and drops the field"
    );

    // Copy and close.
    rt.key(enter());
    rt.frame(0.0);
    assert_eq!(rt.shell_request, Some(Shell::Close));

    let writes = sink.writes();
    assert_eq!(writes.len(), 1);
    let image = decode_png(&writes[0]);
    assert_eq!(image.dimensions(), (3840, 2160));

    let red = pixel_bounds(&image, is_annotation_red).expect("annotations are in the copy");
    // The arrow runs from the upper left to the middle; the text sits below it.
    assert!(
        (580..=620).contains(&(red[0] as i32)),
        "arrow starts near x=600: {red:?}"
    );
    assert!(
        (380..=420).contains(&(red[1] as i32)),
        "arrow starts near y=400: {red:?}"
    );
    assert!(red[2] >= 1760, "arrow head reaches x=1800: {red:?}");
    assert!(red[3] >= 1420, "text reaches y=1400+: {red:?}");

    let arrow_mid = image.get_pixel(1200, 650).0;
    assert!(
        is_annotation_red(arrow_mid),
        "the arrow shaft is red at its midpoint: {arrow_mid:?}"
    );
    let text_hit = image.get_pixel(806, 1424).0;
    assert!(
        text_hit[0] > 120 && text_hit[1] < 120,
        "text ink lands where it was placed: {text_hit:?}"
    );

    // The preview overlay covers exactly the fitted canvas, so preview and export share
    // one image-pixel mapping. The exported arrow start maps back to the on-screen drag
    // start; a shift there would show as a mismatch.
    let (overlay, _) = rt
        .presented("annotations")
        .expect("the annotation layer is shown");
    let (canvas, _) = rt.presented("canvas").unwrap();
    assert!(
        (overlay[0] - canvas[0]).abs() < 0.01
            && (overlay[1] - canvas[1]).abs() < 0.01
            && (overlay[2] - canvas[2]).abs() < 0.01
            && (overlay[3] - canvas[3]).abs() < 0.01,
        "the annotation layer fills the canvas: overlay={overlay:?} canvas={canvas:?}"
    );
    let preview_start = window_point(canvas, [600.0, 400.0], [3840, 2160]);
    let exported_start = (
        canvas[0] + red[0] as f32 / 3840.0 * canvas[2],
        canvas[1] + red[1] as f32 / 2160.0 * canvas[3],
    );
    assert!(
        (preview_start.0 - exported_start.0).abs() < 4.0
            && (preview_start.1 - exported_start.1).abs() < 4.0,
        "exported pixels map back to the preview drag start: preview={preview_start:?} export={exported_start:?}"
    );
}

#[test]
fn escape_discards_the_text_being_typed() {
    let dir = TempDir::new("text-escape");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);

    rt.key(key_text("t"));
    rt.frame(0.0);
    click(&mut rt, [800.0, 1400.0]);
    rt.paste("Oops");
    rt.frame(0.0);
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(0.0);
    assert!(
        !rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("text-editor")),
        "Esc drops the field"
    );

    rt.key(enter());
    rt.frame(0.0);
    assert_eq!(rt.shell_request, Some(Shell::Close));
    let image = decode_png(&sink.writes()[0]);
    assert!(
        pixel_bounds(&image, is_annotation_red).is_none(),
        "the discarded text is not in the copy"
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
    let shown = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("text-editor"))
        .and_then(|e| e.text)
        .unwrap_or_default();
    assert_eq!(shown, "First\nSecond", "Shift+Enter adds a line");
}

#[test]
fn a_copy_failure_keeps_the_editor_open() {
    let dir = TempDir::new("copy-failure");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);
    sink.fail_writes(Some("clipboard denied"));

    rt.key(enter());
    rt.frame(0.0);

    assert_eq!(rt.shell_request, None, "a failed copy does not close");
    assert_eq!(rt.state.error(), Some("clipboard denied"));
    assert!(sink.writes().is_empty());
}

#[test]
fn the_real_clipboard_sink_writes_the_exported_png() {
    let dir = TempDir::new("rune-clipboard");
    let fixture = write_4k_fixture(dir.path());
    let board = rune::clipboard::memory();
    let mut rt = headless_editor_with(
        &fixture,
        Arc::new(gnome_annotation::clipboard::RuneClipboard),
    );
    rt.use_clipboard(&board);

    rt.key(enter());
    rt.frame(0.0);
    rt.frame(0.0);

    let content = board.content().expect("the clipboard received a copy");
    let image = content.image().expect("the copy carries an image");
    assert_eq!(image.size(), (3840, 2160));
    let decoded = decode_png(image.png());
    assert_eq!(decoded.dimensions(), (3840, 2160));
    assert_eq!(rt.shell_request, Some(Shell::Close));
}

#[test]
fn a_fake_portal_capture_opens_the_editor_on_the_returned_file() {
    use gnome_annotation::capture::{CaptureError, CaptureOutcome, ScreenshotPortal};
    use gnome_annotation::cli::run_with;

    struct Fake(Result<CaptureOutcome, CaptureError>);
    impl ScreenshotPortal for Fake {
        fn capture(&self) -> Result<CaptureOutcome, CaptureError> {
            self.0.clone()
        }
    }

    let dir = TempDir::new("capture-flow");
    let fixture = write_4k_fixture(dir.path());
    let fake = Fake(Ok(CaptureOutcome::Captured(fixture.clone())));
    let mut opened = None;
    let code = run_with(["--capture".into()], &fake, |path| {
        opened = Some(path);
        Ok(())
    });
    assert_eq!(code, 0);
    assert_eq!(opened.as_deref(), Some(fixture.as_path()));

    let (_sink, rt) = headless_editor(&fixture);
    let (canvas, _) = rt
        .presented("canvas")
        .expect("the returned 4K image is fitted");
    assert!(canvas[2] < 3840.0 && canvas[3] < 2160.0);
}
