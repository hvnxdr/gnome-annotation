# Desktop acceptance: HVN-37

Run on GNOME Shell 51.1, Wayland, eDP-1 at 1920x1080 and 1.25 scale. Single monitor.
Release binary from `cargo build --release --offline` with the pinned Rune revision.

## Automated

- `cargo test --offline` passes every suite.
- `cargo build --release --offline` and `cargo fmt --check` pass.
- The installer runs with the real `gsettings` against a temporary keyfile store. An
  existing `custom0` binding keeps its name, command, and binding. The Print entry is
  appended once and uninstall removes only that entry. The desktop entry runs the installed
  absolute path, so it does not depend on `~/.local/bin` being on the session PATH.
- A 4K fixture at device scale 2 fits a 1200x800 window. Selection handles stay 8 points,
  and the copy is 3840x2160.
- Every exported pixel equals the annotation overlay composited over the source. Selection
  chrome is absent, and the PNG carries an sRGB chunk.
- The daemon and the 4K editor open no TCP sockets. No app files appear under
  `~/.config`, `~/.local/share`, or `~/.cache`.

## Live desktop

- A trigger opens GNOME's selection UI. The portal returned a 480x270 capture; the editor
  decoded it in 2.4 ms and requested the window 3.1 ms after the response. The first frame
  was ready 219.5 ms after the response.
- The copied PNG is on the clipboard as `image/png`, 480x270 and 27,502 bytes. GNOME's
  original stayed at 480x270 and 10,477 bytes in `~/Pictures/Screenshots`.
- Enter and the Copy button use the same bytes. The live copy closed the editor.
- Ctrl+S suggested `<original stem>-annotated.png`, wrote one 480x270 PNG, and closed. The
  GNOME original was unchanged. No other file appeared for that save.
- Ctrl+S on a 4K fixture suggested `fixture-4k-annotated.png` and wrote one 3840x2160 PNG
  in sRGB. It is pixel-identical to the fixture, as expected with no annotations.
- A second trigger during an open editor was accepted without a second portal call. The
  editor stayed open with its annotations, and the daemon logged
  `gnome-annotation: focusing the open editor`. That is a focus request; whether the
  compositor raises the window is up to Wayland and is not claimed here.
- Cancelling GNOME's UI closed the portal request with no editor window, no log line, and
  no screenshot file.
- Idle memory: a windowless daemon is 21.3 to 22.5 MB. After an editor closes, RSS settles
  at about 75 MB and stays flat across captures; the 4K buffers are dropped with the editor
  state, and the residue is runtime and driver memory. With the 4K editor open, RSS was
  191.6 MB.
- The 4K editor decoded in 126.2 ms and showed its first frame 365.9 ms after launch.

## Not verified on this machine

- Multi-monitor selection. The machine has one display.
- A physical 2x display. The 2x checks are automated, and the live run used 1.25 scale.
- Compositor focus on the second trigger. The daemon logs the focus request, but Wayland may
  refuse to raise the toplevel.
- The real Print keybinding and login autostart. The installer was exercised with the real
  `gsettings` binary and a temporary keyfile store, so the live dconf store was not
  modified.
- Portal failure and notification on the live desktop. The adapter mapping and the
  notification call are covered by tests and by the daemon smoke check.
- Clipboard and save failure on the live desktop. The headless tests fail the sinks and
  assert the status banner and the open editor.

## Gaps fixed during acceptance

- The desktop entry used a bare binary name. It now uses the installed absolute path.
- The daemon now releases the GPU when the editor window closes, and logs the first frame
  time from the portal response.
- The editor README now lists undo, save, Delete, and the text sizes.
- A 2x acceptance test pins the fit, handle size, and source-resolution copy.
