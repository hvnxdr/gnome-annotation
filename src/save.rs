//! Save sinks. Ctrl+S and the Save button hand the exported PNG to one of these.
//!
//! The real target is the FileChooser portal through Rune's `Tx::save_file`. Tests inject a
//! memory sink that records the suggested name and the bytes, and can cancel or fail.

use std::fs::{File, Metadata, OpenOptions};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
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
                        match write_unless_original(editor.capture_path(), &path, &png) {
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

/// Write the PNG to `path` unless it is the Original capture.
///
/// Identity is the `(dev, ino)` pair of open handles, so symlinks, hard links, and a path
/// swapped after the check all resolve to the file that would really be written. The chosen
/// file is opened without truncation and compared before any byte changes. If the capture's
/// identity cannot be read, the save is refused.
fn write_unless_original(capture: &Path, path: &Path, png: &[u8]) -> Result<(), String> {
    let capture_id = File::open(capture)
        .and_then(|file| file.metadata())
        .map(|metadata| file_id(&metadata))
        .map_err(|error| format!("cannot check the original capture: {error}"))?;
    let cannot_save = |error: std::io::Error| format!("cannot save {}: {error}", path.display());
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(cannot_save)?;
    let metadata = file.metadata().map_err(cannot_save)?;
    if file_id(&metadata) == capture_id {
        return Err("cannot overwrite the original capture".to_owned());
    }
    file.set_len(0).map_err(cannot_save)?;
    file.write_all(png).map_err(cannot_save)
}

/// A file's identity on Unix: the device and inode an open handle points at.
fn file_id(metadata: &Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}
