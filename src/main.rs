//! gnome-annotation binary. No arguments triggers the resident daemon; `--daemon` is the
//! daemon itself; `--capture` is the one-shot portal flow; `--editor` opens an explicit image.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gnome_annotation::assets::{FONTS, UI_FAMILY};
use gnome_annotation::capture::PortalScreenshot;
use gnome_annotation::cli;
use gnome_annotation::clipboard::RuneClipboard;
use gnome_annotation::editor::{
    APP_ID, APP_TITLE, DEFAULT_WINDOW, Editor, EditorOptions, MIN_WINDOW, editor_on_key,
    editor_view,
};
use gnome_annotation::resident;
use rune::AppSpec;

fn main() {
    std::process::exit(cli::run_with(
        std::env::args().skip(1),
        &PortalScreenshot,
        launch_editor,
        resident::run_daemon,
        resident::run_trigger,
    ));
}

fn launch_editor(path: PathBuf) -> Result<(), String> {
    let started = Instant::now();
    let options = EditorOptions::new(path, Arc::new(RuneClipboard));
    let editor = Editor::new(options).map_err(|error| error.to_string())?;
    eprintln!(
        "gnome-annotation: decoded and prepared the capture in {:.1} ms",
        editor.timings().decode_ms
    );

    rune::set_app_id(APP_ID);
    let mut spec = AppSpec::new(APP_TITLE, DEFAULT_WINDOW, || unreachable!(), editor_view);
    spec.min_size = Some(MIN_WINDOW);
    spec.fonts = FONTS;
    spec.families = (UI_FAMILY, UI_FAMILY);
    spec.setup = Some(|editor: &mut Editor, waker| editor.setup(waker));
    spec.on_key = Some(editor_on_key);

    rune::App::new(spec)
        .init(move || editor)
        .on_ready(move |_| {
            eprintln!(
                "gnome-annotation: first frame ready {:.1} ms after launch",
                started.elapsed().as_secs_f64() * 1000.0
            );
        })
        .try_run()
        .map_err(|error| error.to_string())
}
