# gnome-annotation

Annotate a screenshot from GNOME's own screenshot UI and copy the result at source
resolution.

This is the v1 build after the framework gate. It has the resident daemon and Print
trigger, the capture path, the editor window, line, arrow, rectangle, ellipse, redaction, and
text annotations, six colours, three stroke widths, selection, undo and redo, Save As, PNG
export, and a clipboard seam.

Domain terms are defined in [`GLOSSARY.md`](GLOSSARY.md). Architecture decisions are
recorded in [`docs/adr/`](docs/adr/).

## Requirements

- Rust 1.88 or newer (edition 2024).
- Linux with a Wayland or X11 session for the editor.
- An XDG desktop portal with `org.freedesktop.portal.Screenshot` for Capture.
- The vendored Adwaita Sans face for annotation text. Rune's Inter faces are vendored for
  the UI.

Rune is pinned to revision `47cb857795c3b1dd5097c15d6161719ae738ddda`. Cargo needs that
revision once; after it is in the cargo cache, `--offline` builds work.

## Install

```sh
scripts/install.sh
```

The installer builds the release binary, installs it to `${XDG_BIN_HOME:-$HOME/.local/bin}`,
writes a desktop entry and a daemon autostart entry, and appends a Print custom keybinding.
Existing custom keybindings keep their order. To start the daemon without logging out,
either log in again or run the installed binary with `--daemon`.

The binding runs `<binary> --trigger`. The one-shot `--capture` mode makes its own portal
call, so it cannot focus the daemon's open editor. The trigger goes through the daemon and
keeps unsaved annotations on a second Print press.

```sh
scripts/install.sh --uninstall
```

Uninstall removes the binary, the desktop entry, and the autostart entry. It also
removes the app's fixed keybinding path and resets that path's name, command, and
binding. It does not restore a previous value at the same path.

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
# Ask the running daemon for a capture. Starts the daemon if it is not running.
gnome-annotation

# The resident daemon. The autostart entry runs this at login.
gnome-annotation --daemon

# Open the editor on an existing PNG.
gnome-annotation --editor capture.png

# One-shot: open GNOME's screenshot UI in this process, then edit what it saved.
gnome-annotation --capture

# Usage.
gnome-annotation --help
```

The daemon listens on `$XDG_RUNTIME_DIR/gnome-annotation.sock` (a per-user directory in
the temp directory when `XDG_RUNTIME_DIR` is unset). It holds a lock file so only one
daemon runs. A trigger is accepted before the portal call, and the client exits as soon as
the daemon accepts. A second trigger while the editor is open requests focus for the editor
and keeps its annotations.

A portal failure or an unreadable capture makes the daemon log to stderr and show a
desktop notification. Cancelling GNOME's capture UI opens nothing.

`--capture` is the separate one-shot path: its portal call runs in that process, so it does
not coordinate with the daemon. `--capture` and `--editor` remain supported for manual use
and Save As. Both exit when the editor closes, so their clipboard offer ends with the
process. Use the resident daemon for reliable Copy unless an external clipboard manager
preserves the offer. The daemon stays running after the editor closes, so a copied image
remains available for paste.

In the editor:

| Key | Action |
| --- | --- |
| `V` | select tool: click to select, drag to move, drag a handle to resize |
| `L` | line tool: drag to draw |
| `A` | arrow tool: drag to draw |
| `R` | rectangle tool: drag out the box |
| `O` | ellipse tool: drag out the box |
| `X` | redaction tool: drag out an opaque black box |
| `T` | text tool: click, type, `Enter` commits, `Shift+Enter` adds a line, `Esc` discards |
| `Delete` | remove the selected annotation |
| `Enter` | Copy the annotated PNG at source resolution and close |
| `Ctrl+S` | Save As: write one PNG through the file chooser and close |
| `Ctrl+Z` | undo the last edit |
| `Ctrl+Shift+Z` | redo the last undone edit |
| `Esc` | close without writing anything |

The toolbar has the same tools, six colours, three stroke widths, three text sizes, Save,
and Copy. New annotations use the selected colour, width, and text size. Redaction is
always opaque black. The whole capture is fitted to the window. There is no zoom or pan.
The toolbar uses one row in wide windows and two rows in narrow windows. Save and Copy
stay visible. The narrow tool row scrolls sideways and reveals the active tool. Copy
preparation and Save As failures appear in a dismissible status banner.

The editor requests a window up to 1200x800 points. Its first size follows the capture's
aspect ratio and the display bounds. The normal minimum is 480x360 points. A smaller
display can force a smaller minimum.

With the select tool, click an annotation to select it, drag it to move it, and drag a
handle to resize it. Press `Delete` to remove it. Double-click committed text to edit it
again. Selection outlines and handles never enter Copy or Save As output.

## Test

```sh
cargo test --offline
```

The editor tests drive a real Rune runtime through the explicit-image seam: they generate a
3840x2160 fixture, draw shapes with the pointer, change the colour and stroke width, type
and commit text, undo and redo edits, save through a headless file dialog, press Enter,
and assert on the PNG a memory clipboard sink received. The daemon tests cover the state
machine, the socket, the trigger client, and the installer with a fake `gsettings` and a
temporary HOME.

To print headless performance diagnostics:

```sh
cargo test --release --offline --test timings -- --nocapture
```

The test uses a headless Rune runtime. It measures CPU work without GPU presentation.

## Layout

| Path | Contents |
| --- | --- |
| `src/capture.rs` | Screenshot portal trait and zbus adapter |
| `src/daemon.rs` | Resident capture state machine |
| `src/ipc.rs` | Unix socket server, lock file, trigger client |
| `src/layout.rs` | Responsive breakpoints and initial window sizing |
| `src/notify.rs` | Desktop notification trait and zbus adapter |
| `src/resident.rs` | Windowless Rune daemon that opens the editor |
| `src/document.rs` | Annotation document, ids, draw order, palette, shared shape geometry |
| `src/editor.rs` | Editor state, Rune view, input handlers, copy and close |
| `src/text.rs` | Parley shaping, swash outlines, tiny-skia text plus halo |
| `src/render.rs` | The source-resolution annotation layer |
| `src/export.rs` | Flatten and encode the PNG |
| `src/save.rs` | Save target, file chooser handoff, and the save seam |
| `src/source.rs` | Decode the PNG into source pixels and a pixmap |
| `src/clipboard.rs` | `ClipboardSink` and the Rune clipboard |
| `src/cli.rs` | Modes: trigger, daemon, `--editor`, `--capture` |
| `scripts/install.sh` | Install and uninstall |

## Measured behavior

Measured with the release build on GNOME Shell 51.1, Wayland, one 1920x1080 display at 1.25
scale:

- A live 480x270 Capture showed its first frame 219.5 ms after the portal response.
- A 4K Editor decoded in 126.2 ms and showed its first frame 365.9 ms after launch.
- A windowless daemon used 21.3 to 22.5 MB RSS.
- After the Editor closed, RSS settled at about 75 MB.
- A 4K Editor used 191.6 MB RSS.
- An automated 2x fixture fitted a 1200x800 window, kept 8 point handles, and produced
  3840x2160 output.

## Verification gaps

- Multi-monitor selection.
- A physical 2x display. The 2x checks are automated, and the live run used 1.25 scale.
- Compositor focus after a second Trigger. The daemon requests focus, but Wayland may refuse
  to raise the window.
- The real Print keybinding and login autostart in the live dconf store. The installer was
  exercised with the real `gsettings` binary against a temporary keyfile store.
- Portal failure and desktop notification on the live desktop. The adapter mapping and
  notification call are covered by tests and by the daemon smoke check.
- Copy and Save As failure on the live desktop. Headless tests inject failures and assert
  the status banner and open Editor.
- Cross-process Wayland paste after the Editor closes.

## Known limitations

- One-shot `--capture` and `--editor` lose their clipboard offer when the process exits. Use
  the resident daemon for reliable Copy unless an external clipboard manager preserves the
  offer.
- The real clipboard sink can reject an invalid or oversized PNG before queueing. A later
  operating system write failure is not reported to the Editor.
