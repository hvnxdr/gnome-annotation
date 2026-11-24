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
    APP_ID, APP_TITLE, Editor, EditorOptions, editor_on_key, editor_view,
};
use gnome_annotation::layout::{initial_window_size, minimum_window_size, primary_layout_bounds};
use gnome_annotation::resident;
use rune::{AppSpec, RunOptions, WindowSpec};

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

    let aspect = editor.aspect_ratio();
    rune::set_app_id(APP_ID);
    rune::try_run_windows_with_args(RunOptions::default(), [], move |displays| {
        let display_pt = primary_layout_bounds(displays);
        let size = initial_window_size(aspect, display_pt);
        let min_size = minimum_window_size(display_pt);
        let mut spec = AppSpec::new(APP_TITLE, size, || unreachable!(), editor_view);
        spec.min_size = Some(min_size);
        spec.fonts = FONTS;
        spec.families = (UI_FAMILY, UI_FAMILY);
        spec.setup = Some(|editor: &mut Editor, waker| editor.setup(waker));
        spec.on_key = Some(editor_on_key);

        let app = rune::App::new(spec)
            .init(move || editor)
            .on_ready(move |_| {
                eprintln!(
                    "gnome-annotation: first frame ready {:.1} ms after launch",
                    started.elapsed().as_secs_f64() * 1000.0
                );
            });
        vec![WindowSpec::from_app("editor", app)]
    })
    .map_err(|error| error.to_string())
}
