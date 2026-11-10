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
use crate::document::{AnnotationKind, Document, Point, Rgba, arrow_geometry, line_length};
use crate::export::{self, ExportError};
use crate::render::render_annotations;
use crate::source::{LoadError, SourceImage};
use crate::text::TextRenderer;

pub const APP_ID: &str = "dev.hvnxdr.GnomeAnnotation";
pub const APP_TITLE: &str = "gnome-annotation";
pub const DEFAULT_WINDOW: (f32, f32) = (1200.0, 800.0);
pub const MIN_WINDOW: (f32, f32) = (640.0, 480.0);

pub struct EditorOptions {
    pub image_path: PathBuf,
    pub clipboard: Arc<dyn ClipboardSink>,
    pub color: Rgba,
    pub stroke_px: f32,
    pub text_size_px: f32,
}

impl EditorOptions {
    pub fn new(image_path: impl Into<PathBuf>, clipboard: Arc<dyn ClipboardSink>) -> Self {
        Self {
            image_path: image_path.into(),
            clipboard,
            color: Rgba::ANNOTATION_RED,
            stroke_px: 6.0,
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

struct Overlay {
    pixmap: tiny_skia::Pixmap,
    image: Option<rune::Image>,
}

pub struct Editor {
    source: SourceImage,
    source_pixmap: tiny_skia::Pixmap,
    image: Option<rune::Image>,
    cache: Option<ImageCache>,
    document: Document,
    tool: Tool,
    draft: Option<Draft>,
    editing: Option<Editing>,
    overlay: Option<Overlay>,
    clipboard: Arc<dyn ClipboardSink>,
    color: Rgba,
    stroke_px: f32,
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
            tool: Tool::Select,
            draft: None,
            editing: None,
            overlay: None,
            clipboard: options.clipboard,
            color: options.color,
            stroke_px: options.stroke_px,
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

    pub fn set_tool(&mut self, tool: Tool) {
        if self.editing.is_some() {
            self.commit_text();
        }
        self.tool = tool;
    }

    pub fn timings(&self) -> Timings {
        self.timings
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Flatten the source and annotations and encode one PNG.
    pub fn export_png(&self) -> Result<Vec<u8>, ExportError> {
        let composed = export::compose(
            &self.source_pixmap,
            self.overlay.as_ref().map(|o| &o.pixmap),
        )?;
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
            self.stroke_px,
        );
        self.rebuild_overlay();
    }

    fn cancel_text(&mut self) {
        self.editing = None;
    }

    fn finish_arrow(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        if arrow_geometry(draft.start, draft.end, self.stroke_px).is_none() {
            return;
        }
        self.document.push(
            AnnotationKind::Arrow {
                start: draft.start,
                end: draft.end,
            },
            self.color,
            self.stroke_px,
        );
        self.rebuild_overlay();
    }

    fn finish_line(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        if line_length(draft.start, draft.end) < 1.0 {
            return;
        }
        self.document.push(
            AnnotationKind::Line {
                start: draft.start,
                end: draft.end,
            },
            self.color,
            self.stroke_px,
        );
        self.rebuild_overlay();
    }

    fn rebuild_overlay(&mut self) {
        let pixmap = render_annotations(
            &mut self.text_renderer,
            &self.document,
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

    fn on_canvas_drag(&mut self, drag: &Drag) {
        match self.tool {
            Tool::Select => {}
            Tool::Rectangle | Tool::Ellipse | Tool::Redaction => {}
            Tool::Line | Tool::Arrow => {
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
                        } else if tool == Tool::Line {
                            self.finish_line();
                        } else {
                            self.finish_arrow();
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

fn tool_button(
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
        .px(space::S3)
        .radius(tok::radius::SM)
        .fill(fill)
        .hover(if active { accent } else { theme.hover })
        .press(if active { accent } else { theme.press })
        .center_y()
        .on_press(move |editor: &mut Editor| on_press(editor))
        .child(text(label).font(type_::LABEL).color(ink))
}

pub fn editor_view(editor: &Editor, cx: &mut Cx<Editor>) -> El {
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
                .pixels(image.pixels()),
        );
    }
    if let Some(draft) = &editor.draft {
        let scale = editor.canvas_pt[0] / editor.source.width as f32;
        if scale > 0.0 {
            let color = rune_color(editor.color);
            let width = (editor.stroke_px * scale).max(0.5);
            let to_pt = |point: Point| [point[0] * scale, point[1] * scale];
            let shape = if draft.tool == Tool::Arrow {
                arrow_geometry(draft.start, draft.end, editor.stroke_px).map(|geometry| {
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
            } else {
                let start = to_pt(draft.start);
                let end = to_pt(draft.end);
                (line_length(draft.start, draft.end) >= 1.0)
                    .then(|| Shape::new().line(start, end, width, LineCap::Round, color))
            };
            if let Some(shape) = shape {
                canvas_el = canvas_el.child(
                    canvas(shape)
                        .key("draft")
                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .clip_rounded(0.0),
                );
            }
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
                .on_edit(|editor: &mut Editor| &mut editor.editing.as_mut().unwrap().edit)
                .on_submit(|editor: &mut Editor, _tx| editor.commit_text())
                .on_input_key(|editor: &mut Editor, event, _tx| {
                    if event.key == Key::Escape {
                        editor.cancel_text();
                        true
                    } else {
                        false
                    }
                }),
        );
    }
    canvas_el = canvas_el
        .on_drawn_layout(|editor: &mut Editor, [_x, _y, w, h]| {
            editor.canvas_pt = [w, h];
        })
        .on_drag(|editor: &mut Editor, drag, _tx| editor.on_canvas_drag(drag));

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
        .child(tool_button(
            "Select",
            "tool-select",
            "V",
            editor.tool == Tool::Select,
            &theme,
            accent,
            |e| e.set_tool(Tool::Select),
        ))
        .child(tool_button(
            "Arrow",
            "tool-arrow",
            "A",
            editor.tool == Tool::Arrow,
            &theme,
            accent,
            |e| e.set_tool(Tool::Arrow),
        ))
        .child(tool_button(
            "Text",
            "tool-text",
            "T",
            editor.tool == Tool::Text,
            &theme,
            accent,
            |e| e.set_tool(Tool::Text),
        ))
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
            .on_press_tx(|editor: &mut Editor, tx: &mut Tx| {
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
