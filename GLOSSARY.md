# Screenshot annotation

This context covers capturing a GNOME screenshot, annotating it in one editing
session, and producing one PNG through Copy or Save As.

## Capture flow

**Trigger**:
A request to start a Capture or request focus for the open Editor. The Print key
sends a Trigger.
_Avoid_: shortcut, key press

**Capture**:
The action that opens GNOME's Screenshot UI and asks for an image. A Capture can
succeed or be cancelled.
_Avoid_: capture request

**Original capture**:
The PNG file GNOME writes after a successful Capture. The app reads it and never
changes or deletes it.
_Avoid_: source image, input file

**Resident daemon**:
The long-lived process that accepts Triggers, hosts at most one Editor, and keeps
a copied image available for paste after the Editor closes.
_Avoid_: background service

**Editor**:
The window used to annotate an Original capture.
_Avoid_: annotation window

**Editor session**:
One editing lifetime for one Original capture. It ends when the Editor closes.
_Avoid_: session

**Clipboard offer**:
The operating system's record that a process owns copied content. On Wayland,
another process can paste only while the owner remains alive.
_Avoid_: clipboard image

## Editing

**Annotation**:
A Line, Arrow, Rectangle, Ellipse, Redaction, or Text added to an Original
capture.
_Avoid_: object

**Shape**:
A geometric Annotation. It is a Line, Arrow, Rectangle, Ellipse, or Redaction.
_Avoid_: drawing

**Redaction**:
An opaque black Shape that hides earlier content in Copy and Save As output. It
does not change the Original capture or remove earlier annotations from the
Annotation document.
_Avoid_: erase, blur

**Text**:
An Annotation with typed characters in Adwaita Sans and a contrasting Halo.
_Avoid_: label

**Halo**:
The contrasting stroke drawn behind Text so it stays readable.
_Avoid_: shadow

**Draft annotation**:
An Annotation in progress that is not yet part of the Annotation document. It
can appear in the Editor preview without entering Copy or Save As output.
_Avoid_: uncommitted mark

**Committed annotation**:
An Annotation that belongs to the Annotation document and can enter Copy and
Save As output.
_Avoid_: saved annotation

**Annotation document**:
The ordered list of Committed annotations for one Original capture. Later
entries appear above earlier ones.
_Avoid_: project file, layer

**Editor preview**:
The current Editor display, including the Original capture, Committed
annotations, any Draft annotation, and Selection chrome.
_Avoid_: canvas

**Fitted canvas**:
The on-screen area that shows the whole Original capture at once. The app
provides no zoom or pan.
_Avoid_: canvas, source image

**Source resolution**:
The pixel dimensions of the Original capture. Copy and Save As keep these
dimensions.
_Avoid_: output size

**Selection chrome**:
The outline and handles shown around the selected Annotation. It never enters
Copy and Save As output.
_Avoid_: selection, handles

## Output

**Annotated image**:
The source-resolution PNG made from the Original capture and Annotation
document. It contains no Selection chrome.
_Avoid_: export, output

**Copy**:
The action that places one Annotated image on the clipboard and closes the
Editor. It writes no file.
_Avoid_: copy and close

**Save As**:
The action that asks for a destination, writes one Annotated image as PNG, and
closes the Editor after success. The toolbar label is "Save".
_Avoid_: Save, export
