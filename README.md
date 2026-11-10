# gnome-annotation

Annotate a screenshot from GNOME's own screenshot UI and copy the result at source
resolution.

This repository has the capture path, the editor window, line, arrow, rectangle, ellipse,
redaction, and text annotations, six colours, three stroke widths, PNG export, and a
clipboard seam. Selection, undo, and the save dialog come later.

## Requirements

- Rust 1.85 or newer (edition 2024).
- Linux with a Wayland or X11 session for the editor.
- An XDG desktop portal with `org.freedesktop.portal.Screenshot` for `--capture`.
- The vendored Adwaita Sans face for annotation text. Rune's Inter faces are vendored for
  the UI.

Rune is pinned to revision `47cb857795c3b1dd5097c15d6161719ae738ddda`. Cargo needs that
revision once; after it is in the cargo cache, `--offline` builds work.

## Build

```sh
cargo build --release
```

For the fastest editor startup, run the release binary:

```sh
./target/release/gnome-annotation --editor /path/to/capture.png
```

## Run

```sh
# Open the editor on an existing PNG (the test seam).
gnome-annotation --editor capture.png

# One-shot: open GNOME's screenshot UI, then edit what it saved.
gnome-annotation --capture

# Usage.
gnome-annotation --help
```

`--capture` opens GNOME's selection, window, and screen modes through the Screenshot
portal. A cancelled capture exits silently. A portal or image error prints to stderr and
exits non-zero.

In the editor:

| Key | Action |
| --- | --- |
| `V` | select tool (no move or resize yet) |
| `L` | line tool: drag to draw |
| `A` | arrow tool: drag to draw |
| `R` | rectangle tool: drag out the box |
| `O` | ellipse tool: drag out the box |
| `X` | redaction tool: drag out an opaque black box |
| `T` | text tool: click, type, `Enter` commits, `Shift+Enter` adds a line, `Esc` discards |
| `Enter` | copy the annotated PNG at source resolution and close |
| `Esc` | close without writing anything |

The toolbar has the same tools, six colours, three stroke widths, and a Copy button. New
annotations use the selected colour and width; redaction is always opaque black. The whole
capture is fitted to the window. There is no zoom or pan.

## Test

```sh
cargo test --offline
```

The editor tests drive a real Rune runtime through the explicit-image seam: they generate a
3840x2160 fixture, draw shapes with the pointer, change the colour and stroke width, type and
commit a label, press Enter, and assert on the PNG a memory clipboard sink received.

To print the spike timings:

```sh
cargo test --release --offline --test timings -- --nocapture
```

See [`docs/spike.md`](docs/spike.md) for the measured numbers and what still needs a desktop
run.

## Layout

| Path | Contents |
| --- | --- |
| `src/capture.rs` | Screenshot portal trait and zbus adapter |
| `src/document.rs` | Annotation document, ids, draw order, palette, shared shape geometry |
| `src/editor.rs` | Editor state, Rune view, input handlers, copy and close |
| `src/text.rs` | Parley shaping, swash outlines, tiny-skia text plus halo |
| `src/render.rs` | The source-resolution annotation layer |
| `src/export.rs` | Flatten and encode the PNG |
| `src/clipboard.rs` | `ClipboardSink`, Rune clipboard, memory sink |
| `src/cli.rs` | `--editor`, `--capture`, `--help` |

## Not yet

No selection move, resize, or delete. No undo or redo. No save dialog. No daemon, socket,
installer, or desktop acceptance run. The document and render seams keep stable ids, a kind
enum, and draw order so those can be added without reshaping the editor.
