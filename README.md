# gnome-annotation

Annotate a screenshot from GNOME's own screenshot UI and copy the result at source
resolution.

This is the v1 build after the framework gate. It has the resident daemon and Print
trigger, the capture path, the editor window, arrow and text annotations, PNG export, and
a clipboard seam. The palette, selection, undo, and save dialog come later.

## Requirements

- Rust 1.85 or newer (edition 2024).
- Linux with a Wayland or X11 session for the editor.
- An XDG desktop portal with `org.freedesktop.portal.Screenshot` for `--capture`.
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

Uninstall removes only the binary, the two entries, and the one keybinding path the
installer added.

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

# Open the editor on an existing PNG (the test seam).
gnome-annotation --editor capture.png

# One-shot: open GNOME's screenshot UI in this process, then edit what it saved.
gnome-annotation --capture

# Usage.
gnome-annotation --help
```

The daemon listens on `$XDG_RUNTIME_DIR/gnome-annotation.sock` (a per-user directory in
the temp directory when `XDG_RUNTIME_DIR` is unset). It holds a lock file so only one
daemon runs. A trigger is accepted before the portal call, and the client exits as soon as
the daemon accepts. A second trigger while the editor is open focuses the editor and keeps
its annotations.

A portal failure or an unreadable capture makes the daemon log to stderr and show a
desktop notification. Cancelling GNOME's capture UI opens nothing.

`--capture` is the separate one-shot path: its portal call runs in that process, so it does
not coordinate with the daemon. It is kept for manual testing.

In the editor:

| Key | Action |
| --- | --- |
| `V` | select tool (no move or resize yet) |
| `A` | arrow tool: drag to draw |
| `T` | text tool: click, type, `Enter` commits, `Shift+Enter` adds a line, `Esc` discards |
| `Enter` | copy the annotated PNG at source resolution and close |
| `Esc` | close without writing anything |

The toolbar has the same tools and a Copy button. The whole capture is fitted to the window.
There is no zoom or pan.

## Test

```sh
cargo test --offline
```

The editor tests drive a real Rune runtime through the explicit-image seam: they generate a
3840x2160 fixture, draw an arrow with the pointer, type and commit a label, press Enter, and
assert on the PNG a memory clipboard sink received. The daemon tests cover the state
machine, the socket, the trigger client, and the installer with a fake `gsettings` and a
temporary HOME.

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
| `src/daemon.rs` | Resident capture state machine |
| `src/ipc.rs` | Unix socket server, lock file, trigger client |
| `src/notify.rs` | Desktop notification trait and zbus adapter |
| `src/resident.rs` | Windowless Rune daemon that opens the editor |
| `src/document.rs` | Annotation document, ids, draw order, shared arrow geometry |
| `src/editor.rs` | Editor state, Rune view, input handlers, copy and close |
| `src/text.rs` | Parley shaping, swash outlines, tiny-skia text plus halo |
| `src/render.rs` | The source-resolution annotation layer |
| `src/export.rs` | Flatten and encode the PNG |
| `src/clipboard.rs` | `ClipboardSink`, Rune clipboard, memory sink |
| `src/cli.rs` | Modes: trigger, daemon, `--editor`, `--capture` |
| `scripts/install.sh` | Install and uninstall |

## Not in this ticket

No rectangle, ellipse, or redaction tools. No selection move or resize. No undo or redo. No
save dialog. The document and render seams keep stable ids, a kind enum, and draw order so
those can be added without reshaping the editor.
