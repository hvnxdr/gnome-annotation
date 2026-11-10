//! Rasterise the annotation document at source resolution. The editor previews this pixmap as
//! an image over the capture, and the exporter composites the same pixmap.

use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform,
};

use crate::document::{
    AnnotationKind, Document, Point, Rect, Rgba, arrow_geometry, ellipse_points,
};
use crate::text::TextRenderer;

pub fn render_annotations(
    text: &mut TextRenderer,
    document: &Document,
    width: u32,
    height: u32,
) -> Option<Pixmap> {
    render_annotations_excluding(text, document, None, width, height)
}

/// Rasterise the document without one annotation. Moves and resizes use this for the static
/// base layer and draw the adjusted annotation live above it.
pub fn render_annotations_excluding(
    text: &mut TextRenderer,
    document: &Document,
    skip: Option<u64>,
    width: u32,
    height: u32,
) -> Option<Pixmap> {
    if document.is_empty()
        || !document
            .annotations()
            .iter()
            .any(|annotation| Some(annotation.id) != skip)
    {
        return None;
    }
    let mut pixmap = Pixmap::new(width, height)?;
    for annotation in document.annotations() {
        if Some(annotation.id) == skip {
            continue;
        }
        match &annotation.kind {
            AnnotationKind::Line { start, end } => {
                draw_line(
                    &mut pixmap,
                    *start,
                    *end,
                    annotation.color,
                    annotation.stroke_px,
                );
            }
            AnnotationKind::Arrow { start, end } => {
                draw_arrow(
                    &mut pixmap,
                    *start,
                    *end,
                    annotation.color,
                    annotation.stroke_px,
                );
            }
            AnnotationKind::Rectangle { start, end } => {
                draw_rect(
                    &mut pixmap,
                    Rect::normalized(*start, *end),
                    annotation.color,
                    annotation.stroke_px,
                );
            }
            AnnotationKind::Ellipse { start, end } => {
                let rect = Rect::normalized(*start, *end);
                draw_polyline(
                    &mut pixmap,
                    &ellipse_points(rect),
                    true,
                    annotation.color,
                    annotation.stroke_px,
                );
            }
            AnnotationKind::Redaction { start, end } => {
                draw_redaction(&mut pixmap, Rect::normalized(*start, *end));
            }
            AnnotationKind::Text {
                position,
                text: value,
                size_px,
            } => {
                let Some(raster) = text.rasterize(value, *size_px, annotation.color) else {
                    continue;
                };
                let x = position[0].round() as i32 + raster.offset[0];
                let y = position[1].round() as i32 + raster.offset[1];
                pixmap.draw_pixmap(
                    x,
                    y,
                    raster.pixmap.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
        }
    }
    Some(pixmap)
}

fn skia_color(color: Rgba) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

fn draw_line(pixmap: &mut Pixmap, start: [f32; 2], end: [f32; 2], color: Rgba, stroke_px: f32) {
    draw_polyline(pixmap, &[start, end], false, color, stroke_px);
}

fn draw_rect(pixmap: &mut Pixmap, rect: Rect, color: Rgba, stroke_px: f32) {
    draw_polyline(pixmap, &rect.corners(), true, color, stroke_px);
}

/// Opaque black with no outline. The annotation's colour and width are ignored.
fn draw_redaction(pixmap: &mut Pixmap, rect: Rect) {
    let mut paint = Paint::default();
    paint.set_color(skia_color(Rgba::BLACK));
    paint.anti_alias = true;

    let mut path = PathBuilder::new();
    for (index, point) in rect.corners().iter().enumerate() {
        if index == 0 {
            path.move_to(point[0], point[1]);
        } else {
            path.line_to(point[0], point[1]);
        }
    }
    path.close();
    let Some(path) = path.finish() else {
        return;
    };
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn draw_polyline(pixmap: &mut Pixmap, points: &[Point], closed: bool, color: Rgba, stroke_px: f32) {
    let Some((first, rest)) = points.split_first() else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(skia_color(color));
    paint.anti_alias = true;

    let mut path = PathBuilder::new();
    path.move_to(first[0], first[1]);
    for point in rest {
        path.line_to(point[0], point[1]);
    }
    if closed {
        path.close();
    }
    let Some(path) = path.finish() else {
        return;
    };
    let stroke = Stroke {
        width: stroke_px.max(1.0),
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

fn draw_arrow(pixmap: &mut Pixmap, start: [f32; 2], end: [f32; 2], color: Rgba, stroke_px: f32) {
    let Some(geometry) = arrow_geometry(start, end, stroke_px) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(skia_color(color));
    paint.anti_alias = true;

    let mut shaft = PathBuilder::new();
    shaft.move_to(geometry.start[0], geometry.start[1]);
    shaft.line_to(geometry.end[0], geometry.end[1]);
    if let Some(shaft) = shaft.finish() {
        let stroke = Stroke {
            width: stroke_px.max(1.0),
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        pixmap.stroke_path(&shaft, &paint, &stroke, Transform::identity(), None);
    }

    let mut head = PathBuilder::new();
    head.move_to(geometry.head[0][0], geometry.head[0][1]);
    head.line_to(geometry.head[1][0], geometry.head[1][1]);
    head.line_to(geometry.head[2][0], geometry.head[2][1]);
    head.close();
    if let Some(head) = head.finish() {
        pixmap.fill_path(
            &head,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}
