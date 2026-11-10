//! Clipboard sinks. The exporter hands the same PNG bytes to whichever sink is installed.

use std::sync::{Arc, Mutex};

use rune::{ClipboardContent, ClipboardImage, Tx};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardError(pub String);

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ClipboardError {}

/// Where an exported PNG goes. HVN-35 extends this seam with a save sink.
pub trait ClipboardSink: Send + Sync {
    fn write_png(&self, png: &[u8], tx: &mut Tx) -> Result<(), ClipboardError>;
}

/// The real clipboard. Linux writes the image form through Rune's arboard worker.
pub struct RuneClipboard;

impl ClipboardSink for RuneClipboard {
    fn write_png(&self, png: &[u8], tx: &mut Tx) -> Result<(), ClipboardError> {
        let image = ClipboardImage::from_png(png.to_vec()).ok_or_else(|| {
            ClipboardError(
                "PNG is larger than the clipboard limit (32 MiB or 64 megapixels)".into(),
            )
        })?;
        tx.copy_rich(ClipboardContent::new("").with_image(image));
        Ok(())
    }
}

/// A sink that keeps every PNG it receives. Used by the headless editor tests.
#[derive(Clone, Default)]
pub struct MemoryClipboardSink {
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
    fail: Arc<Mutex<Option<String>>>,
}

impl MemoryClipboardSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn writes(&self) -> Vec<Vec<u8>> {
        self.writes.lock().unwrap().clone()
    }

    /// Make later writes fail, to exercise the editor's error path.
    pub fn fail_writes(&self, error: Option<&str>) {
        *self.fail.lock().unwrap() = error.map(str::to_owned);
    }
}

impl ClipboardSink for MemoryClipboardSink {
    fn write_png(&self, png: &[u8], _tx: &mut Tx) -> Result<(), ClipboardError> {
        if let Some(error) = self.fail.lock().unwrap().clone() {
            return Err(ClipboardError(error));
        }
        self.writes.lock().unwrap().push(png.to_vec());
        Ok(())
    }
}
