//! The editor: state, Rune view, and the headless-operated input handlers.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use rune::color::srgb_decode;
use rune::edit::TextEdit;
use rune::image::ImageCache;
use rune::tok::{self, space, type_};
use rune::{
    Color, Cx, Drag, El, Family, Key, KeyEvent, LineCap, Phase, Shape, TextStyle, Tx, Waker,
    canvas, col, el, row, text, text_input,
};

use crate::assets::ANNOTATION_FAMILY;
use crate::clipboard::ClipboardSink;
use crate::document::{
    Annotation, AnnotationKind, DEFAULT_TEXT_SIZE_INDEX, Document, MIN_DRAG_PX, PALETTE, Point,
    Rect, Rgba, STROKE_WIDTHS_PT, TEXT_SIZES_PT, arrow_geometry, ellipse_points, line_length,
    point_in_rect,
};
use crate::export::{self, ExportError};
use crate::render::{render_annotations, render_annotations_excluding};
use crate::save::{SaveTarget, save_and_close};
use crate::source::{LoadError, SourceImage};
use crate::text::TextRenderer;

pub const APP_ID: &str = "dev.hvnxdr.GnomeAnnotation";
pub const APP_TITLE: &str = "gnome-annotation";
pub const DEFAULT_WINDOW: (f32, f32) = (1200.0, 800.0);
pub const MIN_WINDOW: (f32, f32) = (640.0, 480.0);

/// Selection handles are square, in screen points.
const HANDLE_SIZE_PT: f32 = 8.0;

pub struct EditorOptions {
    pub image_path: PathBuf,
    pub clipboard: Arc<dyn ClipboardSink>,
    pub save: SaveTarget,
    pub color: Rgba,
    pub width_pt: f32,
    pub text_size_pt: f32,
}

impl EditorOptions {
    pub fn new(image_path: impl Into<PathBuf>, clipboard: Arc<dyn ClipboardSink>) -> Self {
        Self {
            image_path: image_path.into(),
            clipboard,
            save: SaveTarget::Dialog,
            color: Rgba::ANNOTATION_RED,
            width_pt: STROKE_WIDTHS_PT[1],
            text_size_pt: TEXT_SIZES_PT[DEFAULT_TEXT_SIZE_INDEX],
        }
    }

    /// Point saving at a test sink instead of the FileChooser portal.
    pub fn with_save(mut self, save: SaveTarget) -> Self {
        self.save = save;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Line,
    Arrow,
    Rectangle,
    Ellipse,
    Redaction,
    Text,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub decode_ms: f64,
    pub copy_ms: Option<f64>,
}

struct Editing {
    /// The annotation being re-edited, or `None` for a new one.
    id: Option<u64>,
    position: Point,
    /// The size the annotation already has in source pixels. A re-edit keeps it unless the
    /// user picks another toolbar size.
    original_size_px: Option<f32>,
    size_changed: bool,
    /// When a new label's field opened. The Text tool uses this to recognise the second
    /// click of a double-click, which lands on the field instead of the canvas.
    opened_at: Option<Instant>,
    edit: TextEdit,
}

impl Editing {
    /// The size the live field draws at. A re-edit keeps the annotation's own on-screen
    /// size until the user picks a toolbar size.
    fn live_size_pt(&self, selected_pt: f32, fit_scale: f32) -> f32 {
        match self.original_size_px {
            Some(size_px) if !self.size_changed => size_px * fit_scale,
            _ => selected_pt,
        }
    }
}

struct Draft {
    tool: Tool,
    start: Point,
    end: Point,
}

/// A text raster kept for the live preview while its annotation moves.
struct LiveText {
    image: rune::Image,
    offset: [f32; 2],
    size: [f32; 2],
}

/// What the pointer is over, for the canvas cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Hover {
    #[default]
    None,
    Move,
    Handle,
}

/// What a selection gesture changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GestureMode {
    Move,
    Resize { handle: usize },
}

/// One selection adjustment in flight. The document holds the annotation at its old geometry
/// until the gesture commits; `live` is what the canvas previews.
struct Gesture {
    id: u64,
    origin: Annotation,
    live: Annotation,
    anchor: Point,
    mode: GestureMode,
    text: Option<LiveText>,
}

struct Overlay {
    pixmap: tiny_skia::Pixmap,
    image: Option<rune::Image>,
}

/// The selected annotation's outline and handles, in source-image pixels.
struct Chrome {
    bounds: Rect,
    handles: Vec<Point>,
}

pub struct Editor {
    /// The capture GNOME wrote. Save reads its stem for the suggested name and never writes it.
    image_path: PathBuf,
    source: SourceImage,
    source_pixmap: tiny_skia::Pixmap,
    image: Option<rune::Image>,
    cache: Option<ImageCache>,
    document: Document,
    /// Full-document snapshots from before each user-visible edit, newest last.
    undo_stack: Vec<Document>,
    /// Snapshots undone back to, newest last. Any new edit clears this.
    redo_stack: Vec<Document>,
    selected: Option<u64>,
    gesture: Option<Gesture>,
    chrome: Option<Chrome>,
    hover: Hover,
    tool: Tool,
    draft: Option<Draft>,
    editing: Option<Editing>,
    /// The latest pointer position in source-image pixels. `on_double_press` carries no
    /// coordinates, so it reads this to find the text under the second click.
    last_point: Point,
    overlay: Option<Overlay>,
    clipboard: Arc<dyn ClipboardSink>,
    save: SaveTarget,
    color: Rgba,
    width_pt: f32,
    text_size_pt: f32,
    text_renderer: TextRenderer,
    canvas_pt: [f32; 2],
    error: Option<String>,
    timings: Timings,
}

impl Editor {
    pub fn new(options: EditorOptions) -> Result<Self, LoadError> {
        let started = Instant::now();
        let source = SourceImage::load(&options.image_path)?;
        let source_pixmap = source.to_pixmap().map_err(|error| LoadError::Decode {
            path: options.image_path.clone(),
            error: error.to_string(),
        })?;
        let timings = Timings {
            decode_ms: started.elapsed().as_secs_f64() * 1000.0,
            copy_ms: None,
        };
        Ok(Self {
            image_path: options.image_path,
            source,
            source_pixmap,
            image: None,
            cache: None,
            document: Document::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            selected: None,
            gesture: None,
            chrome: None,
            hover: Hover::None,
            tool: Tool::Select,
            draft: None,
            editing: None,
            last_point: [0.0, 0.0],
            overlay: None,
            clipboard: options.clipboard,
            save: options.save,
            color: options.color,
            width_pt: options.width_pt,
            text_size_pt: options.text_size_pt,
            text_renderer: TextRenderer::new(),
            canvas_pt: [0.0, 0.0],
            error: None,
            timings,
        })
    }

    /// Attach the image cache and upload the capture. `AppSpec::setup` supplies a real waker;
    /// tests pass a no-op.
    pub fn setup(&mut self, waker: Waker) {
        let cache = ImageCache::new(waker);
        self.image = Some(cache.rgba(
            self.source.width,
            self.source.height,
            self.source.rgba.clone(),
        ));
        if let Some(overlay) = &mut self.overlay {
            overlay.image = Some(cache.rgba(
                overlay.pixmap.width(),
                overlay.pixmap.height(),
                export::straight_rgba(&overlay.pixmap),
            ));
        }
        self.cache = Some(cache);
    }

    pub fn image_size(&self) -> [u32; 2] {
        [self.source.width, self.source.height]
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Whether there is an edit to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether there is an undone edit to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Save the document before a user-visible change. Starting a new branch of history
    /// discards anything that was undone.
    fn record_edit(&mut self) {
        self.undo_stack.push(self.document.clone());
        self.redo_stack.clear();
    }

    /// Drop the selection and everything drawn from it. The draft and text field are left
    /// alone because each caller decides whether they survive.
    fn clear_selection_state(&mut self) {
        self.selected = None;
        self.gesture = None;
        self.chrome = None;
        self.hover = Hover::None;
    }

    /// Drop everything that points into the old document: selection, live gesture, draft,
    /// and text field. Undo and redo redraw the canvas from the restored snapshot.
    fn clear_transient(&mut self) {
        self.clear_selection_state();
        self.draft = None;
        self.editing = None;
    }

    fn undo(&mut self) -> bool {
        let Some(previous) = self.undo_stack.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.document, previous);
        self.redo_stack.push(current);
        self.clear_transient();
        self.rebuild_overlay();
        true
    }

    fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.document, next);
        self.undo_stack.push(current);
        self.clear_transient();
        self.rebuild_overlay();
        true
    }

    /// The selected annotation's id, if any.
    pub fn selected(&self) -> Option<u64> {
        self.selected
    }

    /// The committed annotation layer exactly as the preview draws it, in source pixels. The
    /// exporter composites this same pixmap, so tests compare the two directly.
    pub fn overlay_pixmap(&self) -> Option<&tiny_skia::Pixmap> {
        self.overlay.as_ref().map(|overlay| &overlay.pixmap)
    }

    /// The topmost annotation under `point`, in source-image pixels.
    fn hit_test(&mut self, point: Point) -> Option<u64> {
        let six_pt_px = pt_to_px(6.0, self.fit_scale());
        let mut found = None;
        for annotation in self.document.annotations().iter().rev() {
            let hit = match &annotation.kind {
                AnnotationKind::Text {
                    position,
                    text,
                    size_px,
                } => self
                    .text_renderer
                    .measure(text, *size_px)
                    .is_some_and(|rect| {
                        point_in_rect(
                            point,
                            Rect {
                                x: position[0] + rect.x,
                                y: position[1] + rect.y,
                                w: rect.w,
                                h: rect.h,
                            },
                        )
                    }),
                _ => {
                    let tolerance_px = (annotation.stroke_px / 2.0 + six_pt_px).max(six_pt_px);
                    annotation.hit_test(point, tolerance_px)
                }
            };
            if hit {
                found = Some(annotation.id);
                break;
            }
        }
        found
    }

    /// Points per source-image pixel at the current window size.
    pub fn fit_scale(&self) -> f32 {
        if self.canvas_pt[0] > 0.0 {
            self.canvas_pt[0] / self.source.width as f32
        } else {
            1.0
        }
    }

    pub fn color(&self) -> Rgba {
        self.color
    }

    pub fn width_pt(&self) -> f32 {
        self.width_pt
    }

    pub fn text_size_pt(&self) -> f32 {
        self.text_size_pt
    }

    pub fn set_color(&mut self, color: Rgba) {
        self.color = color;
    }

    pub fn set_width_pt(&mut self, width_pt: f32) {
        self.width_pt = width_pt;
    }

    pub fn set_text_size_pt(&mut self, size_pt: f32) {
        self.text_size_pt = size_pt;
        if let Some(editing) = &mut self.editing {
            editing.size_changed = true;
        }
    }

    /// The selected screen-point width converted to source-image pixels.
    fn stroke_px(&self) -> f32 {
        pt_to_px(self.width_pt, self.fit_scale())
    }

    /// The selected text size in screen points converted to source-image pixels. Storing
    /// pixels keeps a label's on-screen size after the window resizes.
    fn text_size_px(&self) -> f32 {
        pt_to_px(self.text_size_pt, self.fit_scale())
    }

    pub fn set_tool(&mut self, tool: Tool) {
        if self.editing.is_some() {
            self.commit_text();
        }
        self.draft = None;
        self.hover = Hover::None;
        self.tool = tool;
    }

    pub fn timings(&self) -> Timings {
        self.timings
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Record an inline error from a boundary the editor cannot own (the save dialog).
    pub(crate) fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    /// The capture GNOME wrote. Save must never write over this file.
    pub fn capture_path(&self) -> &std::path::Path {
        &self.image_path
    }

    /// The name the chooser suggests: the capture stem plus `-annotated.png`. The output is
    /// always PNG, whatever extension the capture had.
    pub fn suggested_save_name(&self) -> String {
        let stem = self
            .image_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| "capture".to_owned());
        format!("{stem}-annotated.png")
    }

    /// Whether saves go to the portal or to a test sink.
    pub(crate) fn save_target(&self) -> &SaveTarget {
        &self.save
    }

    /// Flatten the source and annotations and encode one PNG. An adjustment in flight keeps
    /// the annotation at its committed geometry, even though the preview leaves it out of the
    /// base layer.
    pub fn export_png(&mut self) -> Result<Vec<u8>, ExportError> {
        let during_gesture = if self.gesture.is_some() {
            render_annotations(
                &mut self.text_renderer,
                &self.document,
                self.source.width,
                self.source.height,
            )
        } else {
            None
        };
        let overlay = if self.gesture.is_some() {
            during_gesture.as_ref()
        } else {
            self.overlay.as_ref().map(|overlay| &overlay.pixmap)
        };
        let composed = export::compose(&self.source_pixmap, overlay)?;
        export::encode_png(&composed)
    }

    fn copy_and_close(&mut self, tx: &mut Tx) {
        let started = Instant::now();
        let png = match self.export_png() {
            Ok(png) => png,
            Err(error) => {
                self.error = Some(error.to_string());
                eprintln!("gnome-annotation: copy failed: {error}");
                return;
            }
        };
        match self.clipboard.write_png(&png, tx) {
            Ok(()) => {
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                self.timings.copy_ms = Some(ms);
                eprintln!(
                    "gnome-annotation: compose, encode, and hand to the clipboard: {ms:.1} ms"
                );
                tx.close_window();
            }
            Err(error) => {
                self.error = Some(error.to_string());
                eprintln!("gnome-annotation: copy failed: {error}");
            }
        }
    }

    fn begin_text(&mut self, position: Point) {
        let mut edit = TextEdit::new("");
        edit.focused = true;
        self.editing = Some(Editing {
            id: None,
            position,
            original_size_px: None,
            size_changed: false,
            opened_at: Some(Instant::now()),
            edit,
        });
    }

    /// Reopen a committed text annotation for editing. Select and Text both own this
    /// double-click edit; with Text, the first click of the pair may have opened an empty
    /// new label, which `on_canvas_double_press` discards before this runs.
    fn begin_text_edit(&mut self) -> bool {
        if !matches!(self.tool, Tool::Select | Tool::Text) {
            return false;
        }
        let Some(id) = self.hit_test(self.last_point) else {
            return false;
        };
        let Some(annotation) = self.document.get(id).cloned() else {
            return false;
        };
        let AnnotationKind::Text {
            position,
            text,
            size_px,
        } = annotation.kind
        else {
            return false;
        };
        let mut edit = TextEdit::new(text);
        edit.focused = true;
        self.text_size_pt = nearest_text_size_pt(size_px * self.fit_scale());
        self.editing = Some(Editing {
            id: Some(id),
            position,
            original_size_px: Some(size_px),
            size_changed: false,
            opened_at: None,
            edit,
        });
        self.clear_selection_state();
        self.draft = None;
        self.rebuild_overlay_excluding(Some(id));
        true
    }

    fn on_canvas_double_press(&mut self) {
        if self.tool == Tool::Text
            && let Some(editing) = &self.editing
        {
            if editing.id.is_some() {
                return;
            }
            if !editing.edit.text.is_empty() {
                // A new label is already being typed; keep it rather than hijacking the
                // second click into a re-edit.
                return;
            }
            // The first click opened an empty field. Drop it so the committed text under
            // the pointer can be reopened instead.
            self.editing = None;
        }
        if self.begin_text_edit() {
            return;
        }
        // A double press on empty canvas with Text still leaves a field to type into.
        if self.tool == Tool::Text && self.editing.is_none() {
            self.begin_text(self.last_point);
        }
    }

    /// A press landed on the small catcher over a fresh, empty Text field. The field covers
    /// the second click of a double-click, so the catcher forwards it as the double press.
    fn on_text_catcher_press(&mut self) {
        if self.tool != Tool::Text {
            return;
        }
        let Some(editing) = &self.editing else {
            return;
        };
        if editing.id.is_some() || !editing.edit.text.is_empty() {
            return;
        }
        let Some(opened_at) = editing.opened_at else {
            return;
        };
        // Rune's default double-click interval is 0.5 s. A later press is a caret placement.
        if opened_at.elapsed() > std::time::Duration::from_millis(500) {
            return;
        }
        self.last_point = editing.position;
        self.on_canvas_double_press();
    }

    fn commit_text(&mut self) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        let value = editing.edit.text.trim_end_matches('\n').to_owned();
        if let Some(id) = editing.id {
            if value.trim().is_empty() {
                if self.document.get(id).is_some() {
                    self.record_edit();
                    self.document.remove(id);
                }
            } else {
                let size_px = if editing.size_changed {
                    self.text_size_px()
                } else {
                    editing
                        .original_size_px
                        .unwrap_or_else(|| self.text_size_px())
                };
                let kind = AnnotationKind::Text {
                    position: editing.position,
                    text: value,
                    size_px,
                };
                let changed = self
                    .document
                    .get(id)
                    .is_some_and(|annotation| annotation.kind != kind);
                if changed {
                    self.record_edit();
                    self.document.set_kind(id, kind);
                }
            }
            self.clear_selection_state();
            self.rebuild_overlay();
            return;
        }
        if value.trim().is_empty() {
            return;
        }
        self.record_edit();
        self.document.push(
            AnnotationKind::Text {
                position: editing.position,
                text: value,
                size_px: self.text_size_px(),
            },
            self.color,
            self.stroke_px(),
        );
        self.rebuild_overlay();
    }

    fn cancel_text(&mut self) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        if editing.id.is_some() {
            self.rebuild_overlay();
        }
    }

    fn finish_draft(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        let stroke_px = self.stroke_px();
        // Store the geometry the user dragged. The annotation layer is image-sized, so the
        // renderer clips the paint at the capture bounds. Cropping here would shrink the
        // document and lose the part outside the image when the annotation is moved back in.
        let Some(kind) = draft_kind(draft.tool, draft.start, draft.end, stroke_px) else {
            return;
        };
        let (color, stroke_px) = if matches!(kind, AnnotationKind::Redaction { .. }) {
            (Rgba::BLACK, 0.0)
        } else {
            (self.color, stroke_px)
        };
        self.record_edit();
        self.document.push(kind, color, stroke_px);
        self.rebuild_overlay();
    }

    fn rebuild_overlay(&mut self) {
        self.rebuild_overlay_excluding(None);
    }

    /// Rasterise the committed annotations into the preview/export layer, optionally leaving
    /// the annotation under adjustment out so the live preview can replace it.
    fn rebuild_overlay_excluding(&mut self, skip: Option<u64>) {
        let pixmap = render_annotations_excluding(
            &mut self.text_renderer,
            &self.document,
            skip,
            self.source.width,
            self.source.height,
        );
        let Some(pixmap) = pixmap else {
            self.overlay = None;
            return;
        };
        let image = self.cache.as_ref().map(|cache| {
            cache.rgba(
                pixmap.width(),
                pixmap.height(),
                export::straight_rgba(&pixmap),
            )
        });
        self.overlay = Some(Overlay { pixmap, image });
    }

    fn drag_point(&self, drag: &Drag) -> Point {
        let scale = drag.w / self.source.width as f32;
        if scale <= 0.0 {
            return [0.0, 0.0];
        }
        point_at([drag.x, drag.y], scale)
    }

    /// Rasterise text once for the live preview; only its position changes while dragging.
    fn live_text(&mut self, text: &str, size_px: f32, color: Rgba) -> Option<LiveText> {
        let raster = self.text_renderer.rasterize(text, size_px, color)?;
        let cache = self.cache.as_ref()?;
        let image = cache.rgba(
            raster.pixmap.width(),
            raster.pixmap.height(),
            export::straight_rgba(&raster.pixmap),
        );
        Some(LiveText {
            image,
            offset: [raster.offset[0] as f32, raster.offset[1] as f32],
            size: [raster.pixmap.width() as f32, raster.pixmap.height() as f32],
        })
    }

    fn handle_at(&self, handles: &[Point], point: Point) -> Option<usize> {
        let tolerance_px = pt_to_px(6.0, self.fit_scale());
        handles
            .iter()
            .position(|handle| line_length(*handle, point) <= tolerance_px)
    }

    /// The annotation the selection refers to, at its live geometry during a gesture.
    fn live_annotation(&self) -> Option<&Annotation> {
        if let Some(gesture) = &self.gesture {
            return Some(&gesture.live);
        }
        self.selected.and_then(|id| self.document.get(id))
    }

    fn chrome_for(&mut self, annotation: &Annotation) -> Option<Chrome> {
        let padding = (annotation.stroke_px * 0.5).max(1.0);
        match &annotation.kind {
            AnnotationKind::Text {
                position,
                text,
                size_px,
            } => {
                let rect = self.text_renderer.measure(text, *size_px)?;
                Some(Chrome {
                    bounds: Rect {
                        x: position[0] + rect.x,
                        y: position[1] + rect.y,
                        w: rect.w,
                        h: rect.h,
                    },
                    handles: Vec::new(),
                })
            }
            AnnotationKind::Line { start, end } => Some(Chrome {
                bounds: inflate_rect(Rect::normalized(*start, *end), padding),
                handles: vec![*start, *end],
            }),
            AnnotationKind::Arrow { start, end } => {
                let mut points = vec![*start, *end];
                if let Some(geometry) = arrow_geometry(*start, *end, annotation.stroke_px) {
                    points.extend(geometry.head);
                }
                Some(Chrome {
                    bounds: inflate_rect(points_bounds(&points)?, padding),
                    handles: vec![*start, *end],
                })
            }
            AnnotationKind::Rectangle { start, end }
            | AnnotationKind::Ellipse { start, end }
            | AnnotationKind::Redaction { start, end } => {
                let rect = Rect::normalized(*start, *end);
                Some(Chrome {
                    bounds: inflate_rect(rect, padding),
                    handles: rect.corners().to_vec(),
                })
            }
        }
    }

    fn refresh_chrome(&mut self) {
        let Some(annotation) = self.live_annotation().cloned() else {
            self.chrome = None;
            return;
        };
        self.chrome = self.chrome_for(&annotation);
    }

    fn begin_select(&mut self, point: Point) {
        let selected = self.selected.and_then(|id| self.document.get(id)).cloned();
        if let Some(annotation) = selected
            && let Some(handle) = self.handle_at(&annotation.kind.handles(), point)
        {
            self.begin_gesture(annotation, GestureMode::Resize { handle }, point);
            return;
        }
        let Some(id) = self.hit_test(point) else {
            self.clear_selection_state();
            return;
        };
        self.selected = Some(id);
        let Some(annotation) = self.document.get(id).cloned() else {
            return;
        };
        self.begin_gesture(annotation, GestureMode::Move, point);
    }

    fn begin_gesture(&mut self, annotation: Annotation, mode: GestureMode, anchor: Point) {
        let text = if mode == GestureMode::Move {
            match &annotation.kind {
                AnnotationKind::Text { text, size_px, .. } => {
                    self.live_text(text, *size_px, annotation.color)
                }
                _ => None,
            }
        } else {
            None
        };
        let id = annotation.id;
        self.gesture = Some(Gesture {
            id,
            origin: annotation.clone(),
            live: annotation,
            anchor,
            mode,
            text,
        });
        self.rebuild_overlay_excluding(Some(id));
        self.refresh_chrome();
    }

    fn move_gesture(&mut self, point: Point) {
        // A click that pairs into a double-click may jitter by the system's click slop.
        // Keep such a press from nudging text before it reopens for editing.
        let dead_zone_px = pt_to_px(4.0, self.fit_scale());
        let Some(gesture) = &mut self.gesture else {
            return;
        };
        let next = match gesture.mode {
            GestureMode::Move => {
                // The dead zone only protects a press that has not moved yet, so a double-click
                // does not nudge text. Once the gesture is under way, returning to the anchor
                // must move the annotation back.
                let already_moved = !same_geometry(&gesture.live.kind, &gesture.origin.kind);
                if !already_moved && line_length(gesture.anchor, point) < dead_zone_px {
                    return;
                }
                let dx = point[0] - gesture.anchor[0];
                let dy = point[1] - gesture.anchor[1];
                Some(gesture.origin.kind.translated(dx, dy))
            }
            GestureMode::Resize { handle } => gesture.origin.kind.with_handle(handle, point),
        };
        if let Some(kind) = next {
            gesture.live.kind = kind;
        }
        self.refresh_chrome();
    }

    fn delete_selected(&mut self) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        if self.document.get(id).is_none() {
            self.clear_selection_state();
            return false;
        }
        self.record_edit();
        self.document.remove(id);
        self.clear_selection_state();
        self.rebuild_overlay();
        true
    }

    fn finish_gesture(&mut self, cancelled: bool) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        if !cancelled && !same_geometry(&gesture.live.kind, &gesture.origin.kind) {
            self.record_edit();
            self.document.set_kind(gesture.id, gesture.live.kind);
        }
        self.rebuild_overlay();
        self.refresh_chrome();
    }

    /// Track the pointer over the canvas so its cursor can say Move or Grab. Changing only
    /// when the class changes keeps ordinary pointer motion from rebuilding the view.
    fn on_canvas_pointer_move(&mut self, local: [f32; 2]) {
        let scale = self.fit_scale();
        if scale <= 0.0 {
            return;
        }
        let point = point_at(local, scale);
        self.last_point = point;
        if self.tool != Tool::Select || self.gesture.is_some() {
            return;
        }
        let hover = if self
            .chrome
            .as_ref()
            .and_then(|chrome| self.handle_at(&chrome.handles, point))
            .is_some()
        {
            Hover::Handle
        } else if self.selected.is_some() && self.hit_test(point) == self.selected {
            Hover::Move
        } else {
            Hover::None
        };
        if hover != self.hover {
            self.hover = hover;
        }
    }

    fn on_canvas_drag(&mut self, drag: &Drag) {
        let point = self.drag_point(drag);
        self.last_point = point;
        match self.tool {
            Tool::Select => match drag.phase {
                Phase::Down => self.begin_select(point),
                Phase::Move => self.move_gesture(point),
                Phase::Up => self.finish_gesture(drag.cancelled()),
            },
            Tool::Line | Tool::Arrow | Tool::Rectangle | Tool::Ellipse | Tool::Redaction => {
                let tool = self.tool;
                match drag.phase {
                    Phase::Down => {
                        self.draft = Some(Draft {
                            tool,
                            start: point,
                            end: point,
                        })
                    }
                    Phase::Move => {
                        if let Some(draft) = &mut self.draft {
                            draft.end = point;
                        }
                    }
                    Phase::Up => {
                        if let Some(draft) = &mut self.draft {
                            draft.end = point;
                        }
                        if drag.cancelled() {
                            self.draft = None;
                        } else {
                            self.finish_draft();
                        }
                    }
                }
            }
            Tool::Text => {
                if drag.phase == Phase::Down {
                    if self.editing.is_some() {
                        self.commit_text();
                    }
                    self.begin_text(point);
                }
            }
        }
    }
}

/// State that can give the shared view an editor to render. The one-shot editor is its own
/// host; the resident daemon returns `None` while idle and an empty root is drawn.
pub trait HostsEditor {
    fn editor(&self) -> Option<&Editor>;
    fn editor_mut(&mut self) -> Option<&mut Editor>;
}

impl HostsEditor for Editor {
    fn editor(&self) -> Option<&Editor> {
        Some(self)
    }

    fn editor_mut(&mut self) -> Option<&mut Editor> {
        Some(self)
    }
}

pub fn editor_on_key<S: HostsEditor + 'static>(
    state: &mut S,
    event: &KeyEvent,
    tx: &mut Tx,
) -> bool {
    if event.is_command('s') {
        let Some(editor) = state.editor_mut() else {
            return false;
        };
        // A label being typed is part of the document the user asked to save.
        editor.commit_text();
        save_and_close(state, tx);
        return true;
    }
    let Some(editor) = state.editor_mut() else {
        return false;
    };
    if editor.editing.is_some() {
        return false;
    }
    if event.is_command('z') {
        if event.mods.shift {
            editor.redo();
        } else {
            editor.undo();
        }
        return true;
    }
    if event.mods.cmd || event.mods.ctrl || event.mods.alt {
        return false;
    }
    match &event.key {
        Key::Text(value) if value.chars().count() == 1 => {
            match value.to_ascii_lowercase().as_str() {
                "v" => editor.set_tool(Tool::Select),
                "l" => editor.set_tool(Tool::Line),
                "a" => editor.set_tool(Tool::Arrow),
                "r" => editor.set_tool(Tool::Rectangle),
                "o" => editor.set_tool(Tool::Ellipse),
                "x" => editor.set_tool(Tool::Redaction),
                "t" => editor.set_tool(Tool::Text),
                _ => return false,
            }
            true
        }
        Key::Delete | Key::Backspace => editor.delete_selected(),
        Key::Enter => {
            editor.copy_and_close(tx);
            true
        }
        Key::Escape => {
            tx.close_window();
            true
        }
        _ => false,
    }
}

/// The toolbar size closest to a text's current on-screen size, so re-editing shows the
/// button that matches the label when the window has not changed.
fn nearest_text_size_pt(size_pt: f32) -> f32 {
    TEXT_SIZES_PT
        .into_iter()
        .min_by(|a, b| (a - size_pt).abs().total_cmp(&(b - size_pt).abs()))
        .unwrap_or(TEXT_SIZES_PT[DEFAULT_TEXT_SIZE_INDEX])
}

/// Whether two kinds paint the same thing, ignoring the corner order a box was drawn in. A
/// gesture that ends where it began must not add a history entry.
fn same_geometry(a: &AnnotationKind, b: &AnnotationKind) -> bool {
    match (a, b) {
        (AnnotationKind::Text { .. }, AnnotationKind::Text { .. }) => a == b,
        _ => std::mem::discriminant(a) == std::mem::discriminant(b) && a.handles() == b.handles(),
    }
}

fn rune_color(color: Rgba) -> Color {
    let decode = |value: u8| srgb_decode(f32::from(value) / 255.0);
    Color {
        r: decode(color.r),
        g: decode(color.g),
        b: decode(color.b),
        a: f32::from(color.a) / 255.0,
    }
}

fn inflate_rect(rect: Rect, padding: f32) -> Rect {
    Rect {
        x: rect.x - padding,
        y: rect.y - padding,
        w: rect.w + padding * 2.0,
        h: rect.h + padding * 2.0,
    }
}

fn points_bounds(points: &[Point]) -> Option<Rect> {
    let first = *points.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[0], first[1]);
    for point in points {
        x0 = x0.min(point[0]);
        y0 = y0.min(point[1]);
        x1 = x1.max(point[0]);
        y1 = y1.max(point[1]);
    }
    Some(Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

/// Screen points to source-image pixels at `scale` points per pixel.
fn pt_to_px(pt: f32, scale: f32) -> f32 {
    pt / scale.max(f32::EPSILON)
}

/// Source-image pixels to screen points at `scale` points per pixel.
fn px_to_pt(px: f32, scale: f32) -> f32 {
    px * scale
}

/// A canvas-local point in source-image pixels.
fn point_at(local: [f32; 2], scale: f32) -> Point {
    [pt_to_px(local[0], scale), pt_to_px(local[1], scale)]
}

/// A source-image point in canvas-local points.
fn point_to_pt(point: Point, scale: f32) -> [f32; 2] {
    [px_to_pt(point[0], scale), px_to_pt(point[1], scale)]
}

/// The kind a drawing tool would commit for a drag. The draft preview and `finish_draft`
/// share this, so a preview and its committed annotation always have the same geometry.
fn draft_kind(tool: Tool, start: Point, end: Point, stroke_px: f32) -> Option<AnnotationKind> {
    let rect = Rect::normalized(start, end);
    let rect_kind = |make: fn(Point, Point) -> AnnotationKind| {
        (!rect.is_too_small()).then_some(make([rect.x, rect.y], [rect.x + rect.w, rect.y + rect.h]))
    };
    match tool {
        Tool::Line => {
            (line_length(start, end) >= MIN_DRAG_PX).then_some(AnnotationKind::Line { start, end })
        }
        Tool::Arrow => {
            arrow_geometry(start, end, stroke_px).map(|_| AnnotationKind::Arrow { start, end })
        }
        Tool::Rectangle => rect_kind(|start, end| AnnotationKind::Rectangle { start, end }),
        Tool::Ellipse => rect_kind(|start, end| AnnotationKind::Ellipse { start, end }),
        Tool::Redaction => rect_kind(|start, end| AnnotationKind::Redaction { start, end }),
        Tool::Select | Tool::Text => None,
    }
}

/// The in-flight draft in screen points, built from the same geometry the exporter uses.
fn draft_shape(editor: &Editor, draft: &Draft) -> Option<Shape> {
    let stroke_px = editor.stroke_px();
    let kind = draft_kind(draft.tool, draft.start, draft.end, stroke_px)?;
    shape_for_kind(&kind, editor.color, stroke_px, editor.fit_scale())
}

/// The live preview of an annotation under adjustment, built from the same geometry the
/// exporter uses once the gesture commits. Text is a raster image and has no shape.
fn annotation_shape(annotation: &Annotation, scale: f32) -> Option<Shape> {
    shape_for_kind(
        &annotation.kind,
        annotation.color,
        annotation.stroke_px,
        scale,
    )
}

fn shape_for_kind(kind: &AnnotationKind, color: Rgba, stroke_px: f32, scale: f32) -> Option<Shape> {
    if scale <= 0.0 {
        return None;
    }
    let color = rune_color(color);
    let width = px_to_pt(stroke_px, scale).max(0.5);
    let to_pt = |point: Point| point_to_pt(point, scale);
    match kind {
        AnnotationKind::Line { start, end } => (line_length(*start, *end) >= MIN_DRAG_PX)
            .then(|| Shape::new().line(to_pt(*start), to_pt(*end), width, LineCap::Round, color)),
        AnnotationKind::Arrow { start, end } => {
            arrow_geometry(*start, *end, stroke_px).map(|geometry| {
                let mut shape = Shape::new().line(
                    to_pt(geometry.start),
                    to_pt(geometry.end),
                    width,
                    LineCap::Round,
                    color,
                );
                shape = shape.fill(
                    geometry.head.iter().map(|p| to_pt(*p)).collect::<Vec<_>>(),
                    color,
                );
                shape
            })
        }
        AnnotationKind::Rectangle { start, end } => {
            let rect = Rect::normalized(*start, *end);
            (!rect.is_too_small()).then(|| {
                Shape::new().stroke_with(
                    rect.corners().map(to_pt).to_vec(),
                    true,
                    width,
                    LineCap::Round,
                    color,
                )
            })
        }
        AnnotationKind::Ellipse { start, end } => {
            let rect = Rect::normalized(*start, *end);
            (!rect.is_too_small()).then(|| {
                Shape::new().stroke_with(
                    ellipse_points(rect)
                        .into_iter()
                        .map(to_pt)
                        .collect::<Vec<_>>(),
                    true,
                    width,
                    LineCap::Round,
                    color,
                )
            })
        }
        AnnotationKind::Redaction { start, end } => {
            let rect = Rect::normalized(*start, *end);
            (!rect.is_too_small()).then(|| {
                Shape::new().fill(rect.corners().map(to_pt).to_vec(), rune_color(Rgba::BLACK))
            })
        }
        AnnotationKind::Text { .. } => None,
    }
}

fn tool_button<S: HostsEditor + 'static>(
    label: &str,
    key: &'static str,
    shortcut: &str,
    active: bool,
    theme: &tok::Theme,
    accent: Color,
    on_press: impl Fn(&mut Editor) + 'static,
) -> El {
    let fill = if active { accent } else { theme.control_track };
    let ink = if active { Color::WHITE } else { theme.text };
    el().key(key)
        .role("button")
        .label(format!("{label} ({shortcut})"))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .h(32.0)
        .px(space::S2)
        .radius(tok::radius::SM)
        .fill(fill)
        .hover(if active { accent } else { theme.hover })
        .press(if active { accent } else { theme.press })
        .center_y()
        .on_press(move |state: &mut S| {
            if let Some(editor) = state.editor_mut() {
                on_press(editor);
            }
        })
        .child(text(label).font(type_::LABEL).color(ink))
}

fn color_swatch<S: HostsEditor + 'static>(index: usize, selected: bool, theme: &tok::Theme) -> El {
    let color = PALETTE[index];
    let ring = if selected {
        theme.text
    } else {
        theme.card_border
    };
    let ring_width = if selected { 3.0 } else { 1.0 };
    el().key(format!("color-{index}"))
        .role("button")
        .label(format!("Colour {}", index + 1))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .size(22.0, 22.0)
        .radius(11.0)
        .fill(rune_color(color))
        .border(ring_width, ring)
        .hover(rune_color(color))
        .press(rune_color(color))
        .preserve_focus_on_press(true)
        .keeps_text_selection()
        .on_press(move |state: &mut S| {
            if let Some(editor) = state.editor_mut() {
                editor.set_color(PALETTE[index]);
            }
        })
}

/// A square choice button for a width or text size. Both pickers share the same chrome;
/// only the key, label, value, and action differ.
fn choice_button<S: HostsEditor + 'static>(
    key: String,
    label: String,
    value: u32,
    selected: bool,
    theme: &tok::Theme,
    accent: Color,
    on_press: impl Fn(&mut Editor) + 'static,
) -> El {
    let fill = if selected {
        accent
    } else {
        theme.control_track
    };
    let ink = if selected { Color::WHITE } else { theme.text };
    el().key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .size(30.0, 30.0)
        .radius(tok::radius::SM)
        .fill(fill)
        .hover(if selected { accent } else { theme.hover })
        .press(if selected { accent } else { theme.press })
        .preserve_focus_on_press(true)
        .keeps_text_selection()
        .center()
        .on_press(move |state: &mut S| {
            if let Some(editor) = state.editor_mut() {
                on_press(editor);
            }
        })
        .child(text(format!("{value}")).font(type_::LABEL).color(ink))
}

pub fn editor_view<S: HostsEditor + 'static>(state: &S, cx: &mut Cx<S>) -> El {
    let Some(editor) = state.editor() else {
        // The daemon is idle: no window is open, so the root is empty.
        return el().key("empty").w_full().h_full();
    };
    let dark = cx.system_dark();
    let theme = tok::theme(dark);
    let accent = tok::accent(0, dark);
    let ratio = editor.source.width as f32 / editor.source.height as f32;

    let mut canvas_el = el().key("canvas").aspect_fit(ratio);
    if let Some(image) = &editor.image {
        canvas_el = canvas_el.child(
            el().key("canvas-image")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .pixels(image.pixels()),
        );
    }
    if let Some(overlay) = &editor.overlay
        && let Some(image) = &overlay.image
    {
        canvas_el = canvas_el.child(
            el().key("annotations")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .clip_rounded(0.0)
                .pixels(image.pixels()),
        );
    }
    if let Some(draft) = &editor.draft
        && let Some(shape) = draft_shape(editor, draft)
    {
        canvas_el = canvas_el.child(
            canvas(shape)
                .key("draft")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .clip_rounded(0.0),
        );
    }
    if let Some(gesture) = &editor.gesture {
        let scale = editor.fit_scale();
        if scale > 0.0 {
            let mut layer = el()
                .key("live")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .clip_rounded(0.0);
            let mut painted = false;
            if let Some(shape) = annotation_shape(&gesture.live, scale) {
                layer = layer.child(
                    canvas(shape)
                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .clip_rounded(0.0),
                );
                painted = true;
            }
            if let (Some(text), AnnotationKind::Text { position, .. }) =
                (&gesture.text, &gesture.live.kind)
            {
                layer = layer.child(
                    el().key("live-text")
                        .absolute(
                            px_to_pt(position[0] + text.offset[0], scale),
                            px_to_pt(position[1] + text.offset[1], scale),
                        )
                        .size(px_to_pt(text.size[0], scale), px_to_pt(text.size[1], scale))
                        .pixels(text.image.pixels()),
                );
                painted = true;
            }
            if painted {
                canvas_el = canvas_el.child(layer);
            }
        }
    }
    if let Some(chrome) = &editor.chrome {
        let scale = editor.fit_scale();
        if scale > 0.0 {
            let to_pt = |point: Point| point_to_pt(point, scale);
            let mut layer = el()
                .key("selection")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .clip_rounded(0.0)
                .inert();
            if chrome.bounds.w > 0.0 && chrome.bounds.h > 0.0 {
                let outline = Shape::new().stroke_with(
                    chrome.bounds.corners().map(to_pt).to_vec(),
                    true,
                    1.0,
                    LineCap::Round,
                    accent,
                );
                layer = layer.child(
                    canvas(outline)
                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .clip_rounded(0.0),
                );
            }
            for (index, handle) in chrome.handles.iter().enumerate() {
                layer = layer.child(
                    el().key(format!("handle-{index}"))
                        .absolute(
                            px_to_pt(handle[0], scale) - HANDLE_SIZE_PT / 2.0,
                            px_to_pt(handle[1], scale) - HANDLE_SIZE_PT / 2.0,
                        )
                        .size(HANDLE_SIZE_PT, HANDLE_SIZE_PT)
                        .radius(1.0)
                        .fill(Color::WHITE)
                        .border(1.0, accent),
                );
            }
            canvas_el = canvas_el.child(layer);
        }
    }
    if let Some(editing) = &editor.editing {
        let scale = editor.fit_scale();
        let font_size = editing
            .live_size_pt(editor.text_size_pt, editor.fit_scale())
            .clamp(9.0, 96.0);
        let x_pt = px_to_pt(editing.position[0], scale);
        let y_pt = px_to_pt(editing.position[1], scale);
        // Give the field the rest of the canvas (capped) so a normal label is typed on one
        // line. The committed text never wraps; this only keeps the live field from doing so.
        let field_width = (editor.canvas_pt[0] - x_pt - 8.0).clamp(200.0, 960.0);
        let style = TextStyle::new(font_size, 400.0).family(Family::Named(ANNOTATION_FAMILY));
        canvas_el = canvas_el.child(
            text_input(&editing.edit, style)
                .key("text-editor")
                .absolute(x_pt, y_pt)
                .w(field_width)
                .min_h(font_size * 1.6)
                .padding(2.0)
                .color(rune_color(editor.color))
                .caret(theme.text, theme.text_secondary)
                .on_edit(|state: &mut S| {
                    let editor = state.editor_mut().expect("the editor is open");
                    &mut editor
                        .editing
                        .as_mut()
                        .expect("the text field is being edited")
                        .edit
                })
                .on_submit(|state: &mut S, _tx| {
                    if let Some(editor) = state.editor_mut() {
                        editor.commit_text();
                    }
                })
                .on_input_key(|state: &mut S, event, _tx| {
                    let Some(editor) = state.editor_mut() else {
                        return false;
                    };
                    if event.key == Key::Escape {
                        editor.cancel_text();
                        true
                    } else {
                        false
                    }
                }),
        );
        // With the Text tool, the first click of a double-click opens an empty field at the
        // pointer, which covers the second click. Keep a small transparent catcher above the
        // field's anchor so that second press still reaches the editor.
        if editing.id.is_none() && editing.edit.text.is_empty() {
            const SLOP_PT: f32 = 4.0;
            canvas_el = canvas_el.child(
                el().key("text-double-press")
                    .absolute(x_pt - SLOP_PT, y_pt - SLOP_PT)
                    .size(SLOP_PT * 2.0 + 1.0, SLOP_PT * 2.0 + 1.0)
                    .on_drag(|state: &mut S, drag, _tx| {
                        if drag.phase == Phase::Down
                            && let Some(editor) = state.editor_mut()
                        {
                            editor.on_text_catcher_press();
                        }
                    }),
            );
        }
    }
    let cursor = match editor.hover {
        Hover::Move => rune::Cursor::Move,
        Hover::Handle => rune::Cursor::Grab,
        Hover::None => rune::Cursor::Default,
    };
    canvas_el = canvas_el
        .cursor(cursor)
        .on_pointer_move_local(|state: &mut S, event, _tx| {
            if let Some(editor) = state.editor_mut() {
                editor.on_canvas_pointer_move([event.x, event.y]);
            }
        })
        .on_drawn_layout(|state: &mut S, [_x, _y, w, h]| {
            if let Some(editor) = state.editor_mut() {
                editor.canvas_pt = [w, h];
            }
        })
        .on_double_press(|state: &mut S| {
            if let Some(editor) = state.editor_mut() {
                editor.on_canvas_double_press();
            }
        })
        .on_drag(|state: &mut S, drag, _tx| {
            if let Some(editor) = state.editor_mut() {
                editor.on_canvas_drag(drag);
            }
        });

    let stage = col()
        .key("stage")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .items_center()
        .justify_center()
        .padding(space::S6)
        .fill(theme.bg)
        .child(canvas_el);

    let mut toolbar = row()
        .key("toolbar")
        .w_full()
        .h(56.0)
        .px(space::S3)
        .gap(space::S2)
        .center_y()
        .fill(theme.card)
        .border(1.0, theme.card_border)
        .child(tool_button::<S>(
            "Select",
            "tool-select",
            "V",
            editor.tool == Tool::Select,
            &theme,
            accent,
            |e| e.set_tool(Tool::Select),
        ))
        .child(tool_button::<S>(
            "Line",
            "tool-line",
            "L",
            editor.tool == Tool::Line,
            &theme,
            accent,
            |e| e.set_tool(Tool::Line),
        ))
        .child(tool_button::<S>(
            "Arrow",
            "tool-arrow",
            "A",
            editor.tool == Tool::Arrow,
            &theme,
            accent,
            |e| e.set_tool(Tool::Arrow),
        ))
        .child(tool_button::<S>(
            "Rect",
            "tool-rect",
            "R",
            editor.tool == Tool::Rectangle,
            &theme,
            accent,
            |e| e.set_tool(Tool::Rectangle),
        ))
        .child(tool_button::<S>(
            "Ellipse",
            "tool-ellipse",
            "O",
            editor.tool == Tool::Ellipse,
            &theme,
            accent,
            |e| e.set_tool(Tool::Ellipse),
        ))
        .child(tool_button::<S>(
            "Redact",
            "tool-redaction",
            "X",
            editor.tool == Tool::Redaction,
            &theme,
            accent,
            |e| e.set_tool(Tool::Redaction),
        ))
        .child(tool_button::<S>(
            "Text",
            "tool-text",
            "T",
            editor.tool == Tool::Text,
            &theme,
            accent,
            |e| e.set_tool(Tool::Text),
        ));
    for (index, color) in PALETTE.iter().enumerate() {
        toolbar = toolbar.child(color_swatch::<S>(index, editor.color == *color, &theme));
    }
    for (index, width_pt) in STROKE_WIDTHS_PT.iter().enumerate() {
        let width_pt = *width_pt;
        toolbar = toolbar.child(choice_button::<S>(
            format!("width-{index}"),
            format!("Stroke width {width_pt} points"),
            width_pt as u32,
            editor.width_pt == width_pt,
            &theme,
            accent,
            move |editor| editor.set_width_pt(width_pt),
        ));
    }
    for (index, size_pt) in TEXT_SIZES_PT.iter().enumerate() {
        let size_pt = *size_pt;
        toolbar = toolbar.child(choice_button::<S>(
            format!("text-size-{index}"),
            format!("Text size {size_pt} points"),
            size_pt as u32,
            editor.text_size_pt == size_pt,
            &theme,
            accent,
            move |editor| editor.set_text_size_pt(size_pt),
        ));
    }
    toolbar = toolbar
        .child(rune::spacer())
        .child(
            text("Enter copies · Esc discards")
                .font(type_::LABEL)
                .color(theme.text_secondary),
        )
        .child(rune::spacer());
    if let Some(error) = &editor.error {
        toolbar = toolbar.child(
            text(error)
                .key("error")
                .font(type_::LABEL)
                .color(tok::accent(2, dark)),
        );
    }
    toolbar = toolbar.child(
        el().key("save")
            .role("button")
            .label("Save (Ctrl+S)")
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .h(32.0)
            .px(space::S3)
            .radius(tok::radius::SM)
            .fill(theme.control_track)
            .hover(theme.hover)
            .press(theme.press)
            .center_y()
            .on_press_tx(|state: &mut S, tx: &mut Tx| {
                if let Some(editor) = state.editor_mut() {
                    editor.commit_text();
                }
                save_and_close(state, tx);
            })
            .child(text("Save").font(type_::LABEL).color(theme.text)),
    );
    toolbar = toolbar.child(
        el().key("copy")
            .role("button")
            .label("Copy (Enter)")
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .h(32.0)
            .px(space::S3)
            .radius(tok::radius::SM)
            .fill(accent)
            .hover(accent)
            .press(accent)
            .center_y()
            .on_press_tx(|state: &mut S, tx: &mut Tx| {
                let Some(editor) = state.editor_mut() else {
                    return;
                };
                editor.commit_text();
                editor.copy_and_close(tx);
            })
            .child(text("Copy").font(type_::LABEL).color(Color::WHITE)),
    );

    col()
        .w_full()
        .h_full()
        .fill(theme.bg)
        .child(stage)
        .child(toolbar)
}
