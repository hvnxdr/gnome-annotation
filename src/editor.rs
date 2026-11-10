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
    Annotation, AnnotationKind, Document, MIN_DRAG_PX, PALETTE, Point, Rect, Rgba,
    STROKE_WIDTHS_PT, arrow_geometry, crop_segment, ellipse_points, line_length, point_in_rect,
};
use crate::export::{self, ExportError};
use crate::render::{render_annotations, render_annotations_excluding};
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
    pub color: Rgba,
    pub width_pt: f32,
    pub text_size_px: f32,
}

impl EditorOptions {
    pub fn new(image_path: impl Into<PathBuf>, clipboard: Arc<dyn ClipboardSink>) -> Self {
        Self {
            image_path: image_path.into(),
            clipboard,
            color: Rgba::ANNOTATION_RED,
            width_pt: STROKE_WIDTHS_PT[1],
            text_size_px: 48.0,
        }
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
    position: Point,
    edit: TextEdit,
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
    source: SourceImage,
    source_pixmap: tiny_skia::Pixmap,
    image: Option<rune::Image>,
    cache: Option<ImageCache>,
    document: Document,
    selected: Option<u64>,
    gesture: Option<Gesture>,
    chrome: Option<Chrome>,
    hover: Hover,
    tool: Tool,
    draft: Option<Draft>,
    editing: Option<Editing>,
    overlay: Option<Overlay>,
    clipboard: Arc<dyn ClipboardSink>,
    color: Rgba,
    width_pt: f32,
    text_size_px: f32,
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
            source,
            source_pixmap,
            image: None,
            cache: None,
            document: Document::new(),
            selected: None,
            gesture: None,
            chrome: None,
            hover: Hover::None,
            tool: Tool::Select,
            draft: None,
            editing: None,
            overlay: None,
            clipboard: options.clipboard,
            color: options.color,
            width_pt: options.width_pt,
            text_size_px: options.text_size_px,
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

    /// The selected annotation's id, if any.
    pub fn selected(&self) -> Option<u64> {
        self.selected
    }

    /// The topmost annotation under `point`, in source-image pixels.
    fn hit_test(&mut self, point: Point) -> Option<u64> {
        let six_pt_px = 6.0 / self.fit_scale().max(f32::EPSILON);
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

    pub fn set_color(&mut self, color: Rgba) {
        self.color = color;
    }

    pub fn set_width_pt(&mut self, width_pt: f32) {
        self.width_pt = width_pt;
    }

    /// The selected screen-point width converted to source-image pixels.
    fn stroke_px(&self) -> f32 {
        self.width_pt / self.fit_scale()
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
        self.editing = Some(Editing { position, edit });
    }

    fn commit_text(&mut self) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        let value = editing.edit.text.trim_end_matches('\n').to_owned();
        if value.trim().is_empty() {
            return;
        }
        self.document.push(
            AnnotationKind::Text {
                position: editing.position,
                text: value,
                size_px: self.text_size_px,
            },
            self.color,
            self.stroke_px(),
        );
        self.rebuild_overlay();
    }

    fn cancel_text(&mut self) {
        self.editing = None;
    }

    fn finish_draft(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        let stroke_px = self.stroke_px();
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            w: self.source.width as f32,
            h: self.source.height as f32,
        };
        let raw_rect = Rect::normalized(draft.start, draft.end);
        let cropped_rect = raw_rect.crop(bounds);
        let rect_kind = |rect: Rect, make: fn(Point, Point) -> AnnotationKind| {
            (!rect.is_too_small())
                .then_some(make([rect.x, rect.y], [rect.x + rect.w, rect.y + rect.h]))
        };
        let kind =
            match draft.tool {
                Tool::Line => crop_segment(draft.start, draft.end, bounds)
                    .map(|(start, end)| AnnotationKind::Line { start, end }),
                // An arrowhead belongs at the drag's end point. Cropping the segment would move it
                // inside the image, so keep the drag and let the layer clip the paint.
                Tool::Arrow => arrow_geometry(draft.start, draft.end, stroke_px).map(|_| {
                    AnnotationKind::Arrow {
                        start: draft.start,
                        end: draft.end,
                    }
                }),
                Tool::Rectangle => rect_kind(cropped_rect, |start, end| {
                    AnnotationKind::Rectangle { start, end }
                }),
                // Cropping the box of an ellipse changes the curve, so keep the drag box and let
                // the image-sized layer clip the paint instead.
                Tool::Ellipse => rect_kind(raw_rect, |start, end| AnnotationKind::Ellipse {
                    start,
                    end,
                }),
                Tool::Redaction => rect_kind(cropped_rect, |start, end| {
                    AnnotationKind::Redaction { start, end }
                }),
                Tool::Select | Tool::Text => None,
            };
        let Some(kind) = kind else {
            return;
        };
        let (color, stroke_px) = if matches!(kind, AnnotationKind::Redaction { .. }) {
            (Rgba::BLACK, 0.0)
        } else {
            (self.color, stroke_px)
        };
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
        [drag.x / scale, drag.y / scale]
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
        let tolerance_px = 6.0 / self.fit_scale().max(f32::EPSILON);
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
            self.selected = None;
            self.gesture = None;
            self.chrome = None;
            self.hover = Hover::None;
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
        let Some(gesture) = &mut self.gesture else {
            return;
        };
        let next = match gesture.mode {
            GestureMode::Move => {
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
        let removed = self.document.remove(id).is_some();
        self.selected = None;
        self.gesture = None;
        self.chrome = None;
        self.hover = Hover::None;
        if removed {
            self.rebuild_overlay();
        }
        removed
    }

    fn finish_gesture(&mut self, cancelled: bool) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        if !cancelled {
            self.document.set_kind(gesture.id, gesture.live.kind);
        }
        self.rebuild_overlay();
        self.refresh_chrome();
    }

    /// Track the pointer over the canvas so its cursor can say Move or Grab. Changing only
    /// when the class changes keeps ordinary pointer motion from rebuilding the view.
    fn on_canvas_pointer_move(&mut self, local: [f32; 2]) {
        if self.tool != Tool::Select || self.gesture.is_some() {
            return;
        }
        let scale = self.canvas_pt[0] / self.source.width as f32;
        if scale <= 0.0 {
            return;
        }
        let point = [local[0] / scale, local[1] / scale];
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
        match self.tool {
            Tool::Select => {
                let point = self.drag_point(drag);
                match drag.phase {
                    Phase::Down => self.begin_select(point),
                    Phase::Move => self.move_gesture(point),
                    Phase::Up => self.finish_gesture(drag.cancelled()),
                }
            }
            Tool::Line | Tool::Arrow | Tool::Rectangle | Tool::Ellipse | Tool::Redaction => {
                let tool = self.tool;
                let point = self.drag_point(drag);
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
                    self.begin_text(self.drag_point(drag));
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

pub fn editor_on_key(editor: &mut Editor, event: &KeyEvent, tx: &mut Tx) -> bool {
    if editor.editing.is_some() {
        return false;
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

/// The in-flight draft in screen points, built from the same geometry the exporter uses.
fn draft_shape(editor: &Editor, draft: &Draft) -> Option<Shape> {
    let scale = editor.fit_scale();
    if scale <= 0.0 {
        return None;
    }
    let color = rune_color(editor.color);
    let width = editor.width_pt.max(0.5);
    let to_pt = |point: Point| [point[0] * scale, point[1] * scale];
    match draft.tool {
        Tool::Line => (line_length(draft.start, draft.end) >= MIN_DRAG_PX).then(|| {
            Shape::new().line(
                to_pt(draft.start),
                to_pt(draft.end),
                width,
                LineCap::Round,
                color,
            )
        }),
        Tool::Arrow => {
            arrow_geometry(draft.start, draft.end, editor.width_pt / scale).map(|geometry| {
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
        Tool::Rectangle => {
            let rect = Rect::normalized(draft.start, draft.end);
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
        Tool::Ellipse => {
            let rect = Rect::normalized(draft.start, draft.end);
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
        Tool::Redaction => {
            let rect = Rect::normalized(draft.start, draft.end);
            (!rect.is_too_small()).then(|| {
                Shape::new().fill(rect.corners().map(to_pt).to_vec(), rune_color(Rgba::BLACK))
            })
        }
        Tool::Select | Tool::Text => None,
    }
}

/// The live preview of an annotation under adjustment, built from the same geometry the
/// exporter uses once the gesture commits.
fn annotation_shape(annotation: &Annotation, scale: f32) -> Option<Shape> {
    if scale <= 0.0 {
        return None;
    }
    let color = rune_color(annotation.color);
    let width = (annotation.stroke_px * scale).max(0.5);
    let to_pt = |point: Point| [point[0] * scale, point[1] * scale];
    match &annotation.kind {
        AnnotationKind::Line { start, end } => (line_length(*start, *end) >= MIN_DRAG_PX)
            .then(|| Shape::new().line(to_pt(*start), to_pt(*end), width, LineCap::Round, color)),
        AnnotationKind::Arrow { start, end } => arrow_geometry(*start, *end, annotation.stroke_px)
            .map(|geometry| {
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
            }),
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
        .on_press(move |state: &mut S| {
            if let Some(editor) = state.editor_mut() {
                editor.set_color(PALETTE[index]);
            }
        })
}

fn width_button<S: HostsEditor + 'static>(
    index: usize,
    selected: bool,
    theme: &tok::Theme,
    accent: Color,
) -> El {
    let width_pt = STROKE_WIDTHS_PT[index];
    let fill = if selected {
        accent
    } else {
        theme.control_track
    };
    let ink = if selected { Color::WHITE } else { theme.text };
    el().key(format!("width-{index}"))
        .role("button")
        .label(format!("Stroke width {width_pt} points"))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .size(30.0, 30.0)
        .radius(tok::radius::SM)
        .fill(fill)
        .hover(if selected { accent } else { theme.hover })
        .press(if selected { accent } else { theme.press })
        .center()
        .on_press(move |state: &mut S| {
            if let Some(editor) = state.editor_mut() {
                editor.set_width_pt(width_pt);
            }
        })
        .child(
            text(format!("{}", width_pt as u32))
                .font(type_::LABEL)
                .color(ink),
        )
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
                            (position[0] + text.offset[0]) * scale,
                            (position[1] + text.offset[1]) * scale,
                        )
                        .size(text.size[0] * scale, text.size[1] * scale)
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
            let to_pt = |point: Point| [point[0] * scale, point[1] * scale];
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
                            handle[0] * scale - HANDLE_SIZE_PT / 2.0,
                            handle[1] * scale - HANDLE_SIZE_PT / 2.0,
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
        let scale = if editor.canvas_pt[0] > 0.0 {
            editor.canvas_pt[0] / editor.source.width as f32
        } else {
            1.0
        };
        let font_size = (editor.text_size_px * scale).clamp(9.0, 96.0);
        let style = TextStyle::new(font_size, 400.0).family(Family::Named(ANNOTATION_FAMILY));
        canvas_el = canvas_el.child(
            text_input(&editing.edit, style)
                .key("text-editor")
                .absolute(editing.position[0] * scale, editing.position[1] * scale)
                .w(420.0)
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
        toolbar = toolbar.child(width_button::<S>(
            index,
            editor.width_pt == *width_pt,
            &theme,
            accent,
        ));
    }
    toolbar = toolbar
        .child(rune::spacer())
        .child(
            text("Enter copies · Esc cancels")
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
