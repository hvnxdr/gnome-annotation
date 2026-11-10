//! The spike's measured numbers. The assertions are loose bounds so a slow runner reports a
//! failure instead of silently giving up; the printed values go into `docs/spike.md`.

mod support;

use std::sync::Arc;
use std::time::Instant;

use gnome_annotation::assets::FONTS;
use gnome_annotation::clipboard::MemoryClipboardSink;
use gnome_annotation::editor::{Editor, EditorOptions, editor_on_key, editor_view};
use rune::edit::{Key, KeyEvent, Mods};
use rune::text::TextSystem;
use rune::{Env, Runtime, Shell, Waker};

use support::{TempDir, write_4k_fixture};

fn headless(path: &std::path::Path) -> Runtime<Editor> {
    let options = EditorOptions::new(path, Arc::new(MemoryClipboardSink::new()));
    let mut editor = Editor::new(options).expect("load capture");
    editor.setup(Arc::new(|| {}) as Waker);
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

#[test]
fn timing_report() {
    let dir = TempDir::new("timings");
    let fixture = write_4k_fixture(dir.path());

    let cold = Instant::now();
    let mut rt = headless(&fixture);
    let decode = rt.state.timings().decode_ms;
    let first_frame = cold.elapsed().as_secs_f64() * 1000.0;
    let (canvas, _) = rt.presented("canvas").expect("canvas");
    let scale = canvas[2] / 3840.0;

    rt.key(KeyEvent {
        key: Key::Text("a".into()),
        mods: Mods::default(),
    });
    rt.frame(0.0);
    let from = (canvas[0] + 600.0 * scale, canvas[1] + 400.0 * scale);
    let to = (canvas[0] + 1800.0 * scale, canvas[1] + 900.0 * scale);
    rt.pointer_down(from.0, from.1);
    rt.frame(0.0);

    let mut frames = Vec::new();
    for step in 1..=40 {
        let t = step as f32 / 40.0;
        let frame = Instant::now();
        rt.pointer_move(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        rt.frame(0.0);
        frames.push(frame.elapsed().as_secs_f64() * 1000.0);
    }
    rt.pointer_up(to.0, to.1);
    rt.frame(0.0);

    let interactive = Instant::now();
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods::default(),
    });
    rt.frame(0.0);
    let copy = rt.state.timings().copy_ms.expect("copy timing");
    let enter = interactive.elapsed().as_secs_f64() * 1000.0;

    frames.sort_by(|a, b| a.total_cmp(b));
    let median = frames[frames.len() / 2];
    let p90 = frames[frames.len() * 9 / 10];
    eprintln!(
        "spike timings: decode={decode:.1} ms first-frame(headless total)={first_frame:.1} ms \
         drag median={median:.1} ms p90={p90:.1} ms; compose+encode+clipboard={copy:.1} ms \
         enter-to-close={enter:.1} ms; window={}x{}",
        canvas[2], canvas[3]
    );

    assert_eq!(rt.shell_request, Some(Shell::Close));
    assert!(median < 100.0, "drag frames stay under 100 ms: {median}");
    let limit = if cfg!(debug_assertions) {
        15_000.0
    } else {
        3_000.0
    };
    assert!(copy < limit, "copy stays under {limit} ms: {copy}");
}
