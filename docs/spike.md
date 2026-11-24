# Framework gate: capture to annotated copy through Rune

Ticket: HVN-29. Parent spec: HVN-28.

> Status: this document records the framework gate. Later tickets shipped the remaining
> editor features and ran the desktop acceptance. `docs/acceptance.md` is the current state;
> the "still needs" notes below are historical.

## Result

The automated gate passes. A 3840x2160 capture loads into a Rune editor fitted to the
window. An arrow is drawn by dragging the pointer with a live preview. An Adwaita Sans label
is typed, committed, and shaped to outlines with a halo. Enter composes the source pixels and
the annotation layer at 3840x2160, encodes one PNG, writes it through the clipboard sink,
and closes the editor.

One requirement is not verified in this session: the live Screenshot portal call, including
GNOME's selection, window, and screen UI. It needs a human to choose a region. The real
editor window did open on the desktop from an explicit 4K image, so the Rune window and
first-frame path is verified.

## What the automated tests prove

`cargo test --offline` runs these through the editor's headless entry point. The tests inject
an explicit image path and an in-memory clipboard sink, then drive a real Rune
`Runtime` with pointer and key events.

- A 4K PNG decodes and displays. The fitted canvas is 1152x648 inside a 1200x800 window, so
  it is smaller than the source and inside the viewport.
- One arrow drag produces a live preview while moving and one committed annotation on
  release.
- One text click creates a focused field, typing shows the live string, Enter commits, and
  Shift+Enter adds a line. Escape discards the field and leaves no pixels.
- Enter writes exactly one PNG through the clipboard sink, the PNG is 3840x2160, and the
  app requests window close.
- The copied PNG contains the arrow around its drag path and the label around its click
  point, detected by pixel colour.
- A copy with no annotations still produces a valid 3840x2160 PNG.
- A clipboard failure keeps the editor open and shows the error.
- The real Rune clipboard sink writes the same exported PNG. Rune's in-memory clipboard
  receives one image of 3840x2160.
- The preview's annotation layer has the same rect as the canvas. The exported arrow start
  maps back to the on-screen drag start within 4 points.
- Portal outcomes map correctly: response 0 plus a `file://` URI becomes a path, responses 1
  and 2 cancel, a missing URI and an unknown response are errors. The CLI is silent on
  cancel, returns 1 on a portal error, and returns 2 on bad arguments.

## Timings

The timing test is `tests/timings.rs`. It uses a headless Rune `Runtime`, so it measures CPU
work without GPU presentation.

Command:

```sh
cargo test --release --offline --test timings -- --nocapture
```

Environment: Intel Core i5-8250U, 8 threads, Arch Linux, kernel 7.2.14, rustc 1.99.1,
headless runtime with no GPU presentation.

| Measurement | Release, median of 9 runs | Debug, one run |
| --- | ---: | ---: |
| Decode and prepare the 4K PNG | 66.6 ms (range 51.0 to 86.2) | 1582.6 ms |
| Load through first headless frame | 112.2 ms (range 92.4 to 137.9) | 1744.1 ms |
| Arrow drag frame, median of 40 moves | 0.1 ms | 2.0 ms |
| Arrow drag frame, p90 | 0.1 ms | 2.3 ms |
| Compose, encode, and write the clipboard sink | 130.9 ms (range 114.8 to 152.9) | 6587.8 ms |
| Enter to close, same run | 133.7 ms | 6593.0 ms |

Debug-column numbers come from a debug build with unoptimised tiny-skia and deflate. They
are not the numbers a user sees. Run the release build for the app. The ranges are
run-to-run variation on the same machine, not different configurations.

A real window run on Wayland with the release binary:

```sh
./target/release/gnome-annotation --editor /tmp/opencode/gnome-annotation/spike-capture.png
```

Five runs measured decode and prepare between 134.0 and 182.0 ms (median 161.9 ms), and
first frame between 335.3 and 528.1 ms (median 425.0 ms). A warmer earlier run measured
318.5 ms to first frame. Treat the range as the honest number.

That run proves Rune creates the window, decodes the 4K image, and presents the first frame.
The window was closed by a timeout after the measurement; no pointer or keyboard input was
injected into the live window.

### Historical: still needed a human desktop run at the gate

The desktop run has since happened. See `docs/acceptance.md` for the measured portal
response and first frame.

The parent spec's key number, portal response to first frame, cannot be measured without
choosing a region in GNOME's UI. The app prints the pieces:

1. Run `./target/release/gnome-annotation --capture`.
2. Choose a region, a window, or a screen in GNOME's UI.
3. The log prints `screenshot portal responded after N ms`. The N includes the time spent
   choosing.
4. The next line, `first frame ready N ms after launch`, is measured from immediately after
   the portal response. That is the portal-to-first-frame number.
5. Draw an arrow and a label. Press Enter. The log prints `compose, encode, and hand to the
   clipboard: N ms`.
6. Paste into another app to confirm the image is on the system clipboard.

In the timing test the sink writes to memory. In the app the sink hands the same bytes to
Rune's clipboard worker. Neither timing includes the arboard handoff or the paste in another
app.

## What is verified, and what still needs the desktop

| Requirement | Status |
| --- | --- |
| Command opens GNOME selection, window, and screen UI | Not run live. Adapter and response mapping tested with fakes. Needs a human. |
| 4K PNG opens fitted in a Rune window | Automated fitted-rect test passes. A real Rune window opened with the 4K image and logged its first frame. |
| Arrow and Adwaita Sans label drawn with the pointer | Automated through Rune pointer and key input. Not injected into the live window. |
| Enter writes a source-resolution PNG and closes | Automated; real Rune clipboard sink receives the PNG. System clipboard paste not tested. |
| Copied PNG contains both annotations at expected positions | Automated pixel assertions. |
| Portal response to first frame, drag latency, copy time | Drag and copy measured. Decode to first frame measured headless and in a real window. Portal response to first frame still needs a human. |
| Failure reports and stops instead of swapping frameworks | No framework requirement failed. Copy and portal failures have explicit tests. |

## Decisions and deviations

- The ticket says `modal: true`; the parent spec says `modal: false`. The spike followed the
  ticket. HVN-30 restored `modal: false` as the parent spec requires; `interactive: true` and
  the response mapping are unchanged. `target` is omitted because the installed portal is
  version 2.
- All committed annotations render into one source-resolution RGBA layer with tiny-skia. The
  preview draws that layer as a Rune image and the exporter composites the same pixmap, so
  preview and export share pixels and geometry. Text is shaped once with Parley and swash
  into outlines, then rasterised once into that layer.
- The in-flight arrow preview uses a Rune `Shape` mapped through the shared
  `document::arrow_geometry`. Committed arrows use tiny-skia with the same geometry at source
  resolution. Re-rasterising the full 4K layer on every pointer move would make dragging
  slower for no pixel gain after release.
- While typing, the editor uses a Rune `text_input` scaled to the fitted canvas. On commit the
  text becomes source-resolution outlines. The field and the committed label can differ by a
  fraction of a point while the font is screen-scaled.
- `--capture` is one-shot. HVN-30 adds the resident daemon, socket, trigger client, and
  autostart beside it; the one-shot mode stays for manual testing.
- The toolbar is minimal. HVN-29 shipped Select, Arrow, Text, and Copy; HVN-31 adds Line,
  Rectangle, Ellipse, Redaction, six colours, and three stroke widths. At the gate, Select
  had no move, resize, or delete, and undo, redo, and save were later tickets. Those shipped
  after the gate; `docs/acceptance.md` records the current state.
- The editor keeps the source RGBA bytes, a premultiplied source pixmap, and a full-size
  annotation layer while annotations exist. A 4K capture peaks around 100 MB. This is
  acceptable for the gate; tiling or dirty-region rendering is a later optimisation.
- PNGs use sRGB, 8-bit RGBA, and the fast compression setting. Copy latency matters more than
  a smaller file.

## Risks for the next ticket

- The live portal has not run. The main unknowns are the returned file path, file
  permissions, and whether GNOME keeps the window alive long enough for the editor to open.
- The system clipboard path is only proven through Rune's in-memory clipboard. Pasting a 4K
  PNG into another app still needs a manual check.
- The copy path caps at Rune's 32 MiB PNG and 64 megapixel limits. A large, noisy 4K capture
  should stay well below both. If it does not, the editor stays open with a status banner.
- The live first frame will include Wayland window creation and GPU shader setup. The 318.5 ms
  real-window run is a reasonable estimate but was not measured after a portal dialog.
