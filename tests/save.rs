//! Save As: Ctrl+S through the FileChooser portal or a fake sink, cancel, failure,
//! and the guarantee that nothing but the chosen file is written.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gnome_annotation::assets::FONTS;
use gnome_annotation::editor::{Editor, EditorOptions, HostsEditor, editor_on_key, editor_view};
use gnome_annotation::save::SaveTarget;
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, FileDialogKind, Runtime, Shell, Waker};

use support::{TempDir, decode_png, is_annotation_red, pixel_bounds, write_png};

use support::fakes::{MemoryClipboardSink, MemorySaveSink};
const WIDTH: u32 = 480;
const HEIGHT: u32 = 270;

/// A capture named like GNOME's, small enough to keep the suite quick.
fn write_capture(dir: &Path) -> PathBuf {
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let light = ((x / 40) + (y / 30)) % 2 == 0;
            let v = if light { 72u8 } else { 36u8 };
            pixels.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let path = dir.join("Screenshot from 2026-11-16.png");
    write_png(&path, WIDTH, HEIGHT, &pixels);
    path
}

fn noop_waker() -> Waker {
    Arc::new(|| {})
}

fn ctrl_s() -> KeyEvent {
    KeyEvent {
        key: Key::Command('s'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    }
}

struct SaveEditor {
    clipboard: MemoryClipboardSink,
    save: MemorySaveSink,
    rt: Runtime<Editor>,
}

fn saved_editor(path: &Path) -> SaveEditor {
    let clipboard = MemoryClipboardSink::new();
    let save = MemorySaveSink::new();
    let options = EditorOptions::new(path, Arc::new(clipboard.clone()))
        .with_save(SaveTarget::Memory(Arc::new(save.clone())));
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
    SaveEditor {
        clipboard,
        save,
        rt,
    }
}

/// The real save target: Ctrl+S asks Rune for a FileChooser and the test answers it.
fn dialog_editor(path: &Path) -> Runtime<Editor> {
    let options = EditorOptions::new(path, Arc::new(MemoryClipboardSink::new()));
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

/// One arrow on the capture, drawn the same way in two runtimes.
fn annotate(rt: &mut Runtime<Editor>) {
    rt.key(key_text("a"));
    rt.frame(0.0);
    drag(rt, [60.0, 50.0], [220.0, 140.0]);
}

/// A host state shaped like the daemon's: the editor sits in a slot, not at the root.
struct HostSlot {
    editor: Option<Editor>,
}

impl HostsEditor for HostSlot {
    fn editor(&self) -> Option<&Editor> {
        self.editor.as_ref()
    }

    fn editor_mut(&mut self) -> Option<&mut Editor> {
        self.editor.as_mut()
    }
}

#[test]
fn ctrl_s_saves_one_png_named_after_the_capture_and_closes() {
    let dir = TempDir::new("save-memory");
    let fixture = write_capture(dir.path());
    let mut harness = saved_editor(&fixture);

    harness.rt.key(ctrl_s());
    harness.rt.frame(0.0);

    assert_eq!(
        harness.rt.shell_request,
        Some(Shell::Close),
        "a successful save closes the editor"
    );
    let saves = harness.save.saves();
    assert_eq!(saves.len(), 1, "exactly one PNG is written");
    assert_eq!(
        saves[0].suggested_name, "Screenshot from 2026-11-16-annotated.png",
        "the suggested name derives from the capture stem"
    );
    let saved = decode_png(&saves[0].png);
    assert_eq!(
        saved.dimensions(),
        (WIDTH, HEIGHT),
        "the save keeps the source resolution"
    );
    assert!(
        harness.clipboard.writes().is_empty(),
        "Ctrl+S does not copy to the clipboard"
    );
}

#[test]
fn cancelling_the_memory_chooser_leaves_the_editor_open_without_an_error() {
    let dir = TempDir::new("save-cancel");
    let fixture = write_capture(dir.path());
    let mut harness = saved_editor(&fixture);
    harness.save.cancel_saves();

    harness.rt.key(ctrl_s());
    harness.rt.frame(0.0);

    assert_eq!(
        harness.rt.shell_request, None,
        "a cancelled save leaves the editor open"
    );
    assert!(harness.save.saves().is_empty(), "nothing is written");
    assert_eq!(harness.rt.state.error(), None, "cancelling is not an error");
    assert!(
        harness.rt.presented("canvas").is_some(),
        "the capture is still on screen"
    );
}

#[test]
fn a_save_failure_keeps_the_editor_open_and_renders_the_status_banner() {
    let dir = TempDir::new("save-failure");
    let fixture = write_capture(dir.path());
    let mut harness = saved_editor(&fixture);
    harness.save.fail_writes(Some("disk full"));

    harness.rt.key(ctrl_s());
    harness.rt.frame(0.0);
    // Let the status banner appear.
    for _ in 0..60 {
        harness.rt.frame(1.0 / 120.0);
    }

    assert_eq!(
        harness.rt.shell_request, None,
        "a failed save does not ask the shell to close"
    );
    assert!(harness.save.saves().is_empty());
    assert_eq!(harness.rt.state.error(), Some("disk full"));
    let error = harness
        .rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some("error"))
        .expect("the status banner shows the error");
    assert_eq!(error.text.as_deref(), Some("disk full"));
    let (rect, opacity) = harness
        .rt
        .presented("error")
        .expect("the error is laid out");
    assert!(
        rect[2] > 0.0 && rect[3] > 0.0 && opacity > 0.0,
        "the error has a visible box: {rect:?} at {opacity}"
    );
    assert!(harness.rt.presented("canvas").is_some());
}

#[test]
fn ctrl_s_opens_the_portal_dialog_and_writes_the_chosen_file() {
    let dir = TempDir::new("save-dialog");
    let fixture = write_capture(dir.path());
    let mut rt = dialog_editor(&fixture);
    let output = dir.path().join("chosen.png");

    rt.key(ctrl_s());
    rt.frame(0.0);

    let (kind, options) = rt.file_dialog().expect("Ctrl+S asks for a file chooser");
    assert_eq!(kind, FileDialogKind::Save);
    assert_eq!(options.title, "Save annotated capture");
    assert_eq!(
        options.file_name.as_deref(),
        Some("Screenshot from 2026-11-16-annotated.png"),
        "the chooser suggests a name based on the capture"
    );
    assert_eq!(
        options.filters,
        vec![("PNG image".to_owned(), vec!["png".to_owned()])],
        "the chooser filters to PNG"
    );
    assert_eq!(rt.shell_request, None, "the editor waits for the answer");
    assert!(
        !output.exists(),
        "nothing is written before a path is chosen"
    );

    rt.resolve_file_dialog(Some(output.clone()))
        .expect("the dialog is open");
    rt.frame(0.0);

    assert_eq!(
        rt.shell_request,
        Some(Shell::Close),
        "a successful save closes the editor"
    );
    let saved = decode_png(&std::fs::read(&output).expect("the chosen file exists"));
    assert_eq!(saved.dimensions(), (WIDTH, HEIGHT));
}

#[test]
fn cancelling_the_portal_dialog_leaves_the_editor_open_and_writes_nothing() {
    let dir = TempDir::new("save-dialog-cancel");
    let fixture = write_capture(dir.path());
    let mut rt = dialog_editor(&fixture);

    rt.key(ctrl_s());
    rt.frame(0.0);
    assert!(rt.file_dialog().is_some());
    rt.resolve_file_dialog(None).expect("the dialog is open");
    rt.frame(0.0);

    assert_eq!(rt.shell_request, None, "cancelling does not close");
    assert_eq!(rt.state.error(), None, "cancelling is not an error");
    assert!(rt.presented("canvas").is_some());
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .expect("list the capture directory")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from("Screenshot from 2026-11-16.png")],
        "cancelling creates no files"
    );
}

#[test]
fn an_unwritable_path_shows_the_status_banner_and_keeps_the_editor_open() {
    let dir = TempDir::new("save-dialog-unwritable");
    let fixture = write_capture(dir.path());
    let mut rt = dialog_editor(&fixture);

    rt.key(ctrl_s());
    rt.frame(0.0);
    // A directory cannot be overwritten by a file write.
    rt.resolve_file_dialog(Some(dir.path().to_path_buf()))
        .expect("the dialog is open");
    rt.frame(0.0);
    for _ in 0..60 {
        rt.frame(1.0 / 120.0);
    }

    assert_eq!(
        rt.shell_request, None,
        "a failed write does not ask the shell to close"
    );
    let error = rt
        .state
        .error()
        .expect("the failure is reported in the status banner");
    assert!(
        error.contains("cannot save"),
        "the error names the failed save: {error}"
    );
    assert!(rt.presented("canvas").is_some());
    let shown = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some("error"))
        .expect("the status banner shows the error");
    assert_eq!(shown.text.as_deref(), Some(error));
}

#[test]
fn ctrl_s_while_typing_commits_the_label_then_saves_it() {
    let dir = TempDir::new("save-while-typing");
    let fixture = write_capture(dir.path());
    let mut harness = saved_editor(&fixture);

    harness.rt.key(key_text("t"));
    harness.rt.frame(0.0);
    click(&mut harness.rt, [60.0, 60.0]);
    harness.rt.paste("Label");
    harness.rt.frame(0.0);
    assert!(
        harness
            .rt
            .elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("text-editor") && e.focused),
        "the text field is focused"
    );

    harness.rt.key(ctrl_s());
    harness.rt.frame(0.0);

    assert_eq!(harness.rt.shell_request, Some(Shell::Close));
    assert!(
        !harness
            .rt
            .elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("text-editor")),
        "the field is committed before saving"
    );
    let saves = harness.save.saves();
    assert_eq!(saves.len(), 1);
    let saved = decode_png(&saves[0].png);
    let ink = pixel_bounds(&saved, is_annotation_red).expect("the committed label is in the save");
    assert!(
        ink[0] >= 55 && ink[1] >= 55,
        "the label sits where it was typed: {ink:?}"
    );
}

#[test]
fn escape_while_typing_cancels_the_text_and_not_the_editor() {
    let dir = TempDir::new("save-text-escape");
    let fixture = write_capture(dir.path());
    let mut harness = saved_editor(&fixture);

    harness.rt.key(key_text("t"));
    harness.rt.frame(0.0);
    click(&mut harness.rt, [60.0, 60.0]);
    harness.rt.paste("Oops");
    harness.rt.frame(0.0);
    harness.rt.key(escape());
    harness.rt.frame(0.0);

    assert_eq!(
        harness.rt.shell_request, None,
        "Esc in the field drops the text, not the editor"
    );
    assert!(harness.save.saves().is_empty());
    assert!(
        !harness
            .rt
            .elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("text-editor")),
        "the field is gone"
    );

    // The editor stayed alive, so a later save still works.
    harness.rt.key(ctrl_s());
    harness.rt.frame(0.0);
    assert_eq!(harness.rt.shell_request, Some(Shell::Close));
    assert_eq!(harness.save.saves().len(), 1);
}

#[test]
fn the_save_button_writes_the_same_png_and_fits_the_toolbar() {
    let dir = TempDir::new("save-button");
    let fixture = write_capture(dir.path());
    let mut harness = saved_editor(&fixture);
    annotate(&mut harness.rt);

    for key in ["save", "copy"] {
        let (rect, _) = harness.rt.presented(key).expect("the button is laid out");
        assert!(
            rect[0] >= 0.0 && rect[0] + rect[2] <= 1200.0,
            "{key} is inside the window: {rect:?}"
        );
    }

    press(&mut harness.rt, "save");

    assert_eq!(harness.rt.shell_request, Some(Shell::Close));
    assert_eq!(harness.save.saves().len(), 1);
    assert!(harness.clipboard.writes().is_empty());
}

#[test]
fn esc_closes_without_saving() {
    let dir = TempDir::new("save-escape");
    let fixture = write_capture(dir.path());
    let mut harness = saved_editor(&fixture);
    annotate(&mut harness.rt);

    harness.rt.key(escape());
    harness.rt.frame(0.0);

    assert_eq!(harness.rt.shell_request, Some(Shell::Close));
    assert!(
        harness.save.saves().is_empty(),
        "Esc discards without writing"
    );
    assert!(harness.clipboard.writes().is_empty());
    assert!(
        harness.rt.file_dialog().is_none(),
        "Esc does not open the chooser"
    );
}

#[test]
fn a_save_writes_only_the_chosen_file_and_never_touches_the_capture() {
    let dir = TempDir::new("save-hygiene");
    let fixture = write_capture(dir.path());
    let original = std::fs::read(&fixture).expect("read the capture");
    let output = dir.path().join("out.png");
    let mut rt = dialog_editor(&fixture);

    let before: Vec<_> = std::fs::read_dir(dir.path())
        .expect("list the capture directory")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(
        before,
        vec![std::ffi::OsString::from("Screenshot from 2026-11-16.png")]
    );

    rt.key(ctrl_s());
    rt.frame(0.0);
    rt.resolve_file_dialog(Some(output.clone()))
        .expect("the dialog is open");
    rt.frame(0.0);

    assert_eq!(rt.shell_request, Some(Shell::Close));
    assert!(fixture.exists(), "the GNOME original still exists");
    assert_eq!(
        std::fs::read(&fixture).expect("read the capture again"),
        original,
        "the GNOME original is byte for byte unchanged"
    );

    let mut after: Vec<_> = std::fs::read_dir(dir.path())
        .expect("list the capture directory")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    after.sort();
    assert_eq!(
        after,
        vec![
            std::ffi::OsString::from("Screenshot from 2026-11-16.png"),
            std::ffi::OsString::from("out.png"),
        ],
        "only the chosen output appeared; no temporary or history files"
    );
    let saved = decode_png(&std::fs::read(&output).expect("read the output"));
    assert_eq!(saved.dimensions(), (WIDTH, HEIGHT));
}

#[test]
fn choosing_the_original_capture_is_refused_and_leaves_it_unchanged() {
    let dir = TempDir::new("save-original");
    let fixture = write_capture(dir.path());
    let original = std::fs::read(&fixture).expect("read the capture");
    let mut rt = dialog_editor(&fixture);

    rt.key(ctrl_s());
    rt.frame(0.0);
    rt.resolve_file_dialog(Some(fixture.clone()))
        .expect("the dialog is open");
    rt.frame(0.0);
    for _ in 0..60 {
        rt.frame(1.0 / 120.0);
    }

    assert_eq!(
        rt.shell_request, None,
        "a refused save leaves the editor open"
    );
    assert_eq!(
        rt.state.error(),
        Some("cannot overwrite the original capture")
    );
    let shown = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some("error"))
        .expect("the toolbar shows the refusal");
    assert_eq!(
        shown.text.as_deref(),
        Some("cannot overwrite the original capture")
    );
    assert_eq!(
        std::fs::read(&fixture).expect("read the capture again"),
        original,
        "the GNOME original is byte for byte unchanged"
    );
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .expect("list the capture directory")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from("Screenshot from 2026-11-16.png")],
        "the refused save writes nothing"
    );
    assert!(rt.presented("canvas").is_some());
}

#[test]
fn choosing_an_alias_of_the_original_capture_is_refused() {
    let dir = TempDir::new("save-original-alias");
    let fixture = write_capture(dir.path());
    let original = std::fs::read(&fixture).expect("read the capture");
    let alias = dir.path().join("alias.png");
    std::os::unix::fs::symlink(&fixture, &alias).expect("create the alias");
    let mut rt = dialog_editor(&fixture);

    rt.key(ctrl_s());
    rt.frame(0.0);
    rt.resolve_file_dialog(Some(alias))
        .expect("the dialog is open");
    rt.frame(0.0);

    assert_eq!(rt.shell_request, None, "the editor stays open");
    assert_eq!(
        rt.state.error(),
        Some("cannot overwrite the original capture")
    );
    assert_eq!(
        std::fs::read(&fixture).expect("read the capture again"),
        original,
        "writing through the alias would have destroyed the original"
    );
}

#[test]
fn copy_and_save_produce_identical_bytes_for_the_same_document() {
    let dir = TempDir::new("save-same-bytes");
    let fixture = write_capture(dir.path());
    let mut copy_harness = saved_editor(&fixture);
    let mut save_harness = saved_editor(&fixture);
    annotate(&mut copy_harness.rt);
    annotate(&mut save_harness.rt);

    copy_harness.rt.key(enter());
    copy_harness.rt.frame(0.0);
    save_harness.rt.key(ctrl_s());
    save_harness.rt.frame(0.0);

    assert_eq!(copy_harness.rt.shell_request, Some(Shell::Close));
    assert_eq!(save_harness.rt.shell_request, Some(Shell::Close));
    let copied = copy_harness.clipboard.writes();
    let saved = save_harness.save.saves();
    assert_eq!(copied.len(), 1);
    assert_eq!(saved.len(), 1);
    assert_eq!(
        copied[0], saved[0].png,
        "Enter and Ctrl+S encode the same PNG"
    );
    assert!(
        copy_harness.save.saves().is_empty(),
        "Enter does not save a file"
    );
    assert!(
        save_harness.clipboard.writes().is_empty(),
        "Ctrl+S does not copy"
    );
}

#[test]
fn the_suggested_name_is_always_png_even_when_the_capture_is_not() {
    let dir = TempDir::new("save-extension");
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for _ in 0..WIDTH * HEIGHT {
        pixels.extend_from_slice(&[40, 40, 40, 255]);
    }
    // The PNG content is what the loader reads; the original extension is arbitrary.
    let fixture = dir.path().join("capture.jpg");
    write_png(&fixture, WIDTH, HEIGHT, &pixels);
    let mut harness = saved_editor(&fixture);

    harness.rt.key(ctrl_s());
    harness.rt.frame(0.0);

    assert_eq!(
        harness.save.saves()[0].suggested_name,
        "capture-annotated.png",
        "the output is PNG whatever the capture was called"
    );
}

#[test]
fn the_portal_callback_returns_to_a_host_state() {
    let dir = TempDir::new("save-host");
    let fixture = write_capture(dir.path());
    let output = dir.path().join("host.png");
    let options = EditorOptions::new(&fixture, Arc::new(MemoryClipboardSink::new()));
    let mut editor = Editor::new(options).expect("load capture");
    editor.setup(noop_waker());
    let mut rt = Runtime::with_text(
        HostSlot {
            editor: Some(editor),
        },
        editor_view,
        (1200.0, 800.0),
        Env::new(true, 1.0),
        TextSystem::new(FONTS, ("Inter", "Inter")),
    );
    rt.on_key = Some(editor_on_key);
    rt.frame(0.0);

    rt.key(ctrl_s());
    rt.frame(0.0);
    assert!(
        rt.file_dialog().is_some(),
        "the daemon-shaped state opens the chooser"
    );
    rt.resolve_file_dialog(Some(output.clone()))
        .expect("the dialog is open");
    rt.frame(0.0);

    assert_eq!(
        rt.shell_request,
        Some(Shell::Close),
        "the answer reaches the host state's editor"
    );
    assert!(output.exists(), "the chosen file was written");
}
