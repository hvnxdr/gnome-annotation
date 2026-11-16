# Export and clipboard

Ticket: HVN-35. Parent spec: HVN-28.

## The path

Enter and the Copy button both call `Editor::copy_and_close`. They take the same steps:

1. `export::compose` copies the source pixmap and draws the committed annotation overlay onto
   it.
2. `export::encode_png` converts the result to straight-alpha RGBA and encodes an 8-bit sRGB
   PNG.
3. The installed `ClipboardSink` writes the bytes. The app uses `RuneClipboard`, which calls
   `tx.copy_rich(ClipboardContent::new("").with_image(ClipboardImage::from_png(bytes)?))`.

The annotation overlay is the same pixmap the preview draws as a Rune image. Text is shaped
once into that pixmap. Selection outlines and handles are separate view elements, so they
never enter the export. The preview and the export use source-image pixels for both, so the
fitted canvas only scales the finished images for display.

## What the tests check

`tests/export.rs` covers the export seam:

- Enter and the Copy button write byte-identical PNGs for the same document.
- Every exported pixel equals the overlay composited over the source. The test covers a line,
  arrow, rectangle, ellipse, redaction, and one label.
- The PNG is 8-bit RGBA with an sRGB chunk.
- The real Rune clipboard holds one image with the exact exported bytes.
- A PNG over 32 MiB or over 64 megapixels returns an error instead of a panic.
- A failed write keeps the editor open and shows the message in the toolbar.

`tests/selection.rs` checks that selection chrome stays out of the PNG. The export with and
without a selection is byte-identical, and no accent or handle colour appears.

`tests/editor.rs` checks that a copy with no annotations matches the source image pixel for
pixel.

## Clipboard lifetime on Linux

Rune writes the image through arboard. On Wayland, arboard uses `wl-clipboard-rs`, which serves
paste requests from a thread inside this process. A paste works while the process runs. When
the process exits, the compositor drops the offer and the image is gone.

The resident daemon keeps running after the editor window closes, so paste works in normal
use. The one-shot modes (`--editor` and `--capture`) exit when the editor closes. On those
paths the clipboard image is lost after exit, because no process is left to serve the paste.
This is a limitation of the one-shot modes, not of the copy itself. The daemon is the
supported path; the one-shot modes stay for manual testing.

Keeping a one-shot process alive until someone pastes would hold the process and the window
open with no bound. Rune's close hook already hands the bytes to the clipboard worker before
the window goes, and the worker drains its queue before shutdown, so the loss comes from the
process exit itself. Only a long-lived process keeps the offer.
