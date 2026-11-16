//! Native-resolution export: one PNG for Enter and the copy button, preview pixels, sRGB
//! metadata, and the clipboard handoff.

mod support;

use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::clipboard::{ClipboardSink, RuneClipboard};
use gnome_annotation::editor::{Editor, EditorOptions, editor_on_key, editor_view};
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Shell, Tx, Waker};

use support::{TempDir, decode_png, write_4k_fixture};

use support::fakes::MemoryClipboardSink;
#[test]
fn exported_pixels_are_the_preview_overlay_over_the_source() {
    let dir = TempDir::new("export-preview");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);

    // Every shape kind and one label: strokes, a fill, and glyphs.
    rt.key(key_text("l"));
    rt.frame(0.0);
    drag(&mut rt, [400.0, 300.0], [1200.0, 320.0]);
    draw_arrow(&mut rt, [600.0, 400.0], [1800.0, 900.0]);
    rt.key(key_text("r"));
    rt.frame(0.0);
    drag(&mut rt, [2200.0, 300.0], [3000.0, 700.0]);
    rt.key(key_text("o"));
    rt.frame(0.0);
    drag(&mut rt, [2200.0, 900.0], [3000.0, 1300.0]);
    rt.key(key_text("x"));
    rt.frame(0.0);
    drag(&mut rt, [400.0, 1700.0], [1200.0, 1950.0]);
    place_text(&mut rt, [1500.0, 1500.0], "Label");

    let overlay = rt
        .state
        .overlay_pixmap()
        .expect("the committed annotations have a layer")
        .clone();
    let source = decode_png(&std::fs::read(&fixture).expect("read the fixture"));
    let exported = decode_png(&rt.state.export_png().expect("the export succeeds"));
    assert_eq!(exported.dimensions(), (3840, 2160));
    assert_eq!(
        [overlay.width(), overlay.height()],
        [3840, 2160],
        "the preview layer is at source resolution"
    );

    let mut covered = 0usize;
    let mut max_diff = 0u8;
    for y in 0..exported.height() {
        for x in 0..exported.width() {
            let preview = overlay.pixel(x, y).expect("inside the layer");
            let src = source.get_pixel(x, y).0;
            let alpha = u16::from(preview.alpha());
            if alpha > 0 {
                covered += 1;
            }
            let blended = |channel: u8, dst: u8| {
                let dst = u16::from(dst);
                (u16::from(channel) + (dst * (255 - alpha) + 127) / 255).min(255) as u8
            };
            let expected = [
                blended(preview.red(), src[0]),
                blended(preview.green(), src[1]),
                blended(preview.blue(), src[2]),
                255,
            ];
            let actual = exported.get_pixel(x, y).0;
            for channel in 0..4 {
                max_diff = max_diff.max(expected[channel].abs_diff(actual[channel]));
            }
        }
    }
    assert_eq!(
        max_diff, 0,
        "every exported pixel is the preview overlay over the source"
    );
    assert!(
        covered > 10_000,
        "the comparison covered real annotations: {covered} pixels"
    );

    // Each annotation leaves ink where it was previewed.
    let alpha_in = |x0: u32, y0: u32, x1: u32, y1: u32| {
        let mut count = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                if overlay.pixel(x, y).is_some_and(|p| p.alpha() > 0) {
                    count += 1;
                }
            }
        }
        count
    };
    for (name, bounds) in [
        ("line", (380, 280, 1220, 340)),
        ("arrow", (580, 380, 1860, 950)),
        ("rectangle", (2180, 280, 3020, 720)),
        ("ellipse", (2180, 880, 3020, 1320)),
        ("redaction", (380, 1680, 1220, 1970)),
        ("text", (1450, 1420, 1700, 1600)),
    ] {
        assert!(
            alpha_in(bounds.0, bounds.1, bounds.2, bounds.3) > 0,
            "the {name} appears at its previewed position"
        );
    }
}

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

fn headless_editor_with(
    path: &std::path::Path,
    clipboard: Arc<dyn ClipboardSink>,
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

fn press(rt: &mut Runtime<Editor>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is present"));
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn draw_arrow(rt: &mut Runtime<Editor>, from: [f32; 2], to: [f32; 2]) {
    rt.key(key_text("a"));
    rt.frame(0.0);
    drag(rt, from, to);
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

/// One arrow and one label, drawn the same way in any runtime.
fn annotate(rt: &mut Runtime<Editor>) {
    draw_arrow(rt, [600.0, 400.0], [1800.0, 900.0]);
    place_text(rt, [800.0, 1400.0], "Label");
}

#[test]
fn the_exported_png_is_eight_bit_srgb_rgba() {
    let dir = TempDir::new("export-srgb");
    let fixture = write_4k_fixture(dir.path());
    let (_sink, mut rt) = headless_editor(&fixture);
    draw_arrow(&mut rt, [600.0, 400.0], [1800.0, 900.0]);

    let png_bytes = rt.state.export_png().expect("the export succeeds");
    let reader = png::Decoder::new(std::io::Cursor::new(&png_bytes))
        .read_info()
        .expect("the export is a valid PNG");
    let info = reader.info();
    assert_eq!((info.width, info.height), (3840, 2160));
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(
        info.srgb,
        Some(png::SrgbRenderingIntent::Perceptual),
        "the PNG carries an sRGB chunk"
    );
    assert!(
        png_bytes.windows(4).any(|chunk| chunk == b"sRGB"),
        "the sRGB chunk is in the file"
    );
}

/// A PNG header with the given dimensions and no image data. Enough for the clipboard's
/// header-only checks, and cheap to make oversized.
fn header_only_png(width: u32, height: u32) -> Vec<u8> {
    let mut png = Vec::with_capacity(33);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.extend_from_slice(&[0, 0, 0, 0]);
    png
}

#[test]
fn the_system_sink_refuses_images_over_the_clipboard_caps() {
    let sink = RuneClipboard;
    let mut tx = Tx::default();

    // 8192x8192 is exactly 64 megapixels: the largest image the clipboard accepts.
    let largest = header_only_png(8192, 8192);
    assert!(
        sink.write_png(&largest, &mut tx).is_ok(),
        "exactly 64 megapixels is accepted"
    );

    let too_many_pixels = header_only_png(8193, 8192);
    let error = sink
        .write_png(&too_many_pixels, &mut tx)
        .expect_err("more than 64 megapixels is refused");
    assert!(
        error.to_string().contains("64 megapixels"),
        "the error names the pixel cap: {error}"
    );

    let mut too_many_bytes = header_only_png(2, 2);
    too_many_bytes.resize(rune::clipboard::MAX_IMAGE_BYTES + 1, 0);
    let error = sink
        .write_png(&too_many_bytes, &mut tx)
        .expect_err("more than 32 MiB is refused");
    assert!(
        error.to_string().contains("32 MiB"),
        "the error names the byte cap: {error}"
    );
}

#[test]
fn the_system_sink_puts_the_exact_export_on_the_clipboard() {
    let dir = TempDir::new("export-system-clipboard");
    let fixture = write_4k_fixture(dir.path());
    let board = rune::clipboard::memory();
    let mut rt = headless_editor_with(&fixture, Arc::new(RuneClipboard));
    rt.use_clipboard(&board);
    annotate(&mut rt);

    let expected = rt.state.export_png().expect("the export succeeds");
    rt.key(enter());
    rt.frame(0.0);
    rt.frame(0.0);

    assert_eq!(rt.shell_request, Some(Shell::Close));
    let content = board.content().expect("the clipboard received a copy");
    let expected_image =
        rune::ClipboardImage::from_png(expected.clone()).expect("the export fits the clipboard");
    assert_eq!(
        content,
        rune::ClipboardContent::new("").with_image(expected_image),
        "the clipboard holds one image and no other form"
    );
    let image = content.image().expect("the copy carries one image");
    assert_eq!(image.size(), (3840, 2160));
    assert_eq!(
        image.png(),
        expected.as_slice(),
        "the clipboard holds the exact exported bytes"
    );
}

#[test]
fn a_failed_copy_keeps_the_editor_open_and_renders_the_error() {
    let dir = TempDir::new("export-failure");
    let fixture = write_4k_fixture(dir.path());
    let (sink, mut rt) = headless_editor(&fixture);
    annotate(&mut rt);
    let message = "PNG is larger than the clipboard limit (32 MiB or 64 megapixels)";
    sink.fail_writes(Some(message));

    rt.key(enter());
    rt.frame(0.0);
    // Let the inline error fade in.
    for _ in 0..60 {
        rt.frame(1.0 / 120.0);
    }

    assert_eq!(
        rt.shell_request, None,
        "a failed copy does not ask the shell to close"
    );
    assert!(
        rt.presented("canvas").is_some(),
        "the editor still shows the capture"
    );
    let error = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some("error"))
        .expect("the toolbar shows an inline error");
    assert_eq!(error.text.as_deref(), Some(message));
    let (rect, opacity) = rt.presented("error").expect("the error is laid out");
    assert!(
        rect[2] > 0.0 && rect[3] > 0.0 && opacity > 0.0,
        "the error has a visible box: {rect:?} at {opacity}"
    );
    assert_eq!(rt.state.error(), Some(message));
    assert!(sink.writes().is_empty());

    // The editor stayed alive, so a later copy still works.
    let png = rt.state.export_png().expect("the editor is still usable");
    assert_eq!(decode_png(&png).dimensions(), (3840, 2160));
}

#[test]
fn enter_and_the_copy_button_write_the_same_png() {
    let dir = TempDir::new("export-same-bytes");
    let fixture = write_4k_fixture(dir.path());
    let (enter_sink, mut enter_rt) = headless_editor(&fixture);
    let (button_sink, mut button_rt) = headless_editor(&fixture);

    for rt in [&mut enter_rt, &mut button_rt] {
        annotate(rt);
    }

    enter_rt.key(enter());
    enter_rt.frame(0.0);
    press(&mut button_rt, "copy");

    assert_eq!(enter_rt.shell_request, Some(Shell::Close));
    assert_eq!(button_rt.shell_request, Some(Shell::Close));
    let enter_writes = enter_sink.writes();
    let button_writes = button_sink.writes();
    assert_eq!(enter_writes.len(), 1);
    assert_eq!(button_writes.len(), 1);
    assert_eq!(
        enter_writes[0], button_writes[0],
        "Enter and the copy button encode the same PNG"
    );
}
