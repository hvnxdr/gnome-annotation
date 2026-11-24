//! Save sinks. Ctrl+S and the Save button hand the exported PNG to one of these.
//!
//! The real target is the FileChooser portal through Rune's `Tx::save_file`. Tests inject a
//! memory sink that records the suggested name and the bytes, and can cancel or fail.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rune::{FileDialogOptions, Tx};

use crate::editor::HostsEditor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveError(pub String);

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SaveError {}

/// What a memory save did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    Cancelled,
}

/// A record-only save target. Tests implement this; the app uses the file dialog.
pub trait SaveSink: Send + Sync {
    fn save(&self, suggested_name: &str, png: &[u8]) -> Result<SaveOutcome, SaveError>;
}

/// Where an exported PNG goes. The dialog is the only real target; memory is the test seam.
#[derive(Clone)]
pub enum SaveTarget {
    Dialog,
    Memory(Arc<dyn SaveSink>),
}

impl Default for SaveTarget {
    fn default() -> Self {
        Self::Dialog
    }
}

/// The one file output: export the same bytes Enter copies, then ask where they go.
///
/// The dialog answer runs on a later frame. A chosen path is written once; cancelling does
/// nothing; an export or write failure leaves the editor open with a status banner.
pub fn save_and_close<S: HostsEditor + 'static>(state: &mut S, tx: &mut Tx) {
    let Some(editor) = state.editor_mut() else {
        return;
    };
    let png = match editor.export_png() {
        Ok(png) => png,
        Err(error) => {
            editor.set_error(error.to_string());
            eprintln!("gnome-annotation: save failed: {error}");
            return;
        }
    };
    let suggested_name = editor.suggested_save_name();
    match editor.save_target().clone() {
        SaveTarget::Memory(sink) => match sink.save(&suggested_name, &png) {
            Ok(SaveOutcome::Saved) => tx.close_window(),
            Ok(SaveOutcome::Cancelled) => {}
            Err(error) => {
                editor.set_error(error.to_string());
                eprintln!("gnome-annotation: save failed: {error}");
            }
        },
        SaveTarget::Dialog => {
            let options = FileDialogOptions {
                title: "Save annotated capture".into(),
                file_name: Some(suggested_name),
                ..Default::default()
            }
            .filter("PNG image", &["png"]);
            tx.save_file(options, move |state: &mut S, result, tx| {
                let Some(editor) = state.editor_mut() else {
                    return;
                };
                match result {
                    Ok(Some(path)) => {
                        if same_file(editor.capture_path(), &path) {
                            let error = "cannot overwrite the original capture".to_owned();
                            eprintln!("gnome-annotation: save failed: {error}");
                            editor.set_error(error);
                            return;
                        }
                        match write_png(&path, &png) {
                            Ok(()) => tx.close_window(),
                            Err(error) => {
                                eprintln!("gnome-annotation: save failed: {error}");
                                editor.set_error(error);
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(error) => editor.set_error(error.to_string()),
                }
            });
        }
    }
}

/// Whether two paths name the same file. An existing file resolves through `canonicalize`;
/// a path that does not exist yet resolves its parent, so a chosen name in a linked directory
/// still compares equal to the capture.
fn same_file(a: &Path, b: &Path) -> bool {
    fn resolved(path: &Path) -> Option<PathBuf> {
        if let Ok(canonical) = path.canonicalize() {
            return Some(canonical);
        }
        let name = path.file_name()?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        parent.canonicalize().ok().map(|parent| parent.join(name))
    }
    match (resolved(a), resolved(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn write_png(path: &Path, png: &[u8]) -> Result<(), String> {
    std::fs::write(path, png).map_err(|error| format!("cannot save {}: {error}", path.display()))
}
