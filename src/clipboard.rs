//! Clipboard sinks. The exporter hands the same PNG bytes to whichever sink is installed.

use rune::{ClipboardContent, ClipboardImage, Tx};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardError(pub String);

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ClipboardError {}

/// Where an exported PNG goes. HVN-36 adds a sink that saves the same bytes to disk.
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
