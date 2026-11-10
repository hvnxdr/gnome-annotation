//! Annotation text: shape with Parley, take outlines from swash, rasterise the fill and its
//! halo with tiny-skia. The preview and the export use these exact pixels.

use std::sync::Arc;

use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, FontWeight,
    LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
};
use swash::FontRef;
use swash::scale::ScaleContext;
use swash::zeno::{Command, PathData};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform,
};

use crate::assets::{ADWAITA_SANS, ANNOTATION_FAMILY};
use crate::document::Rgba;

/// One shaped and rasterised string. `offset` is the raster's top-left relative to the text
/// annotation's anchor, in source-image pixels.
pub struct TextRaster {
    pub pixmap: Pixmap,
    pub offset: [i32; 2],
}

pub struct TextRenderer {
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
    scale_cx: ScaleContext,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    pub fn new() -> Self {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        collection.register_fonts(Blob::new(Arc::new(ADWAITA_SANS.to_vec())), None);
        let font_cx = FontContext {
            collection,
            source_cache: SourceCache::default(),
        };
        Self {
            font_cx,
            layout_cx: LayoutContext::new(),
            scale_cx: ScaleContext::new(),
        }
    }

    /// Shape `text` and return a haloed RGBA raster at source-image pixels.
    pub fn rasterize(&mut self, text: &str, size_px: f32, color: Rgba) -> Option<TextRaster> {
        if text.is_empty() || !size_px.is_finite() || size_px <= 0.0 {
            return None;
        }
        let path = self.outline(text, size_px)?;
        let bounds = path.bounds();
        if !bounds.width().is_finite() || !bounds.height().is_finite() {
            return None;
        }
        let halo = (size_px * 0.07).clamp(1.0, 8.0);
        let pad = halo + 1.0;
        let left = bounds.left().floor() - pad;
        let top = bounds.top().floor() - pad;
        let width = (bounds.right().ceil() + pad - left).max(1.0) as u32;
        let height = (bounds.bottom().ceil() + pad - top).max(1.0) as u32;
        let mut pixmap = Pixmap::new(width, height)?;
        let placed = path.transform(Transform::from_translate(-left, -top))?;

        let halo_color = if color.luminance() > 0.5 {
            Color::BLACK
        } else {
            Color::WHITE
        };
        let mut stroke_paint = Paint::default();
        stroke_paint.set_color(halo_color);
        stroke_paint.anti_alias = true;
        let stroke = Stroke {
            width: halo * 2.0,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        pixmap.stroke_path(&placed, &stroke_paint, &stroke, Transform::identity(), None);

        let mut fill_paint = Paint::default();
        fill_paint.set_color_rgba8(color.r, color.g, color.b, color.a);
        fill_paint.anti_alias = true;
        pixmap.fill_path(
            &placed,
            &fill_paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        Some(TextRaster {
            pixmap,
            offset: [left as i32, top as i32],
        })
    }

    fn outline(&mut self, text: &str, size_px: f32) -> Option<Path> {
        let family = [FontFamilyName::named(ANNOTATION_FAMILY)];
        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(FontFamily::from(family.as_slice()));
        builder.push_default(StyleProperty::FontSize(size_px));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(400.0)));
        builder.push_default(LineHeight::Absolute(size_px * 1.2));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        let mut path = PathBuilder::new();
        let mut any = false;
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let font = run.run().font();
                let data: &[u8] = font.data.as_ref();
                let font_ref = FontRef::from_index(data, font.index as usize)?;
                let mut scaler = self
                    .scale_cx
                    .builder(font_ref)
                    .size(size_px)
                    .hint(true)
                    .build();
                for glyph in run.positioned_glyphs() {
                    let Some(outline) = scaler.scale_outline(glyph.id as u16) else {
                        continue;
                    };
                    for layer in 0..outline.len() {
                        let Some(layer) = outline.get(layer) else {
                            continue;
                        };
                        // swash outlines are y-up from the baseline; tiny-skia is y-down.
                        for command in layer.path().commands() {
                            match command {
                                Command::MoveTo(p) => {
                                    path.move_to(glyph.x + p.x, glyph.y - p.y);
                                    any = true;
                                }
                                Command::LineTo(p) => path.line_to(glyph.x + p.x, glyph.y - p.y),
                                Command::QuadTo(c, p) => path.quad_to(
                                    glyph.x + c.x,
                                    glyph.y - c.y,
                                    glyph.x + p.x,
                                    glyph.y - p.y,
                                ),
                                Command::CurveTo(c1, c2, p) => path.cubic_to(
                                    glyph.x + c1.x,
                                    glyph.y - c1.y,
                                    glyph.x + c2.x,
                                    glyph.y - c2.y,
                                    glyph.x + p.x,
                                    glyph.y - p.y,
                                ),
                                Command::Close => path.close(),
                            }
                        }
                    }
                }
            }
        }
        if !any { None } else { path.finish() }
    }
}
