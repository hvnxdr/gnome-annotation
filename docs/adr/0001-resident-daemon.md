---
status: accepted
---

# Keep a resident daemon as clipboard owner and single-editor host

On Wayland, the process that owns the clipboard serves paste requests. A
one-shot process would lose its clipboard offer when the Editor closes. Keep a
windowless resident daemon running after the editor closes. It accepts Triggers
over a per-user Unix socket, owns the clipboard worker, and hosts at most one
Editor. A second Trigger while an Editor is open requests focus and preserves
unsaved annotations. The daemon retains no capture history and drops the image,
Annotation document, undo and redo state, and GPU resources when the Editor
closes.

The resident daemon is the default path for Triggers and reliable Copy. `--capture`
and `--editor` remain supported one-shot paths. They work for editing and Save
As. Their Copy depends on an external clipboard manager because the process
exits when the Editor closes.

## Considered options

- **One-shot-only process per Capture.** Rejected because its clipboard offer
  ends at Editor close and a second Trigger cannot preserve unsaved annotations.
- **Hidden one-shot process kept alive until paste.** Rejected because it has no
  bounded ownership lifetime or trigger coordination.
- **Separate clipboard broker.** Rejected for v1 because it adds another process
  and protocol. The resident daemon already stays alive.
- **Required external clipboard manager.** Rejected because it is not guaranteed
  and does not preserve the open Editor.

## Consequences

- The app needs autostart, a Unix socket, a per-user lock, and a resident process.
- Idle memory was measured at 21.3 to 22.5 MB RSS.
- Editor close releases the large image and annotation state. The daemon and
  clipboard worker remain alive.
- A second Trigger is a focus request. Wayland may refuse to raise the window.
- One-shot Copy has an accepted clipboard lifetime limitation.
- No automated test performs a cross-process Wayland paste after the Editor
  closes.
