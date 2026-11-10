//! PNG export at source resolution.

use tiny_skia::{Pixmap, PixmapPaint, Transform};

pub type Result<T> = std::result::Result<T, ExportError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportError(pub String);

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExportError {}

/// Flatten the source and the annotation overlay into one premultiplied canvas.
pub fn compose(source: &Pixmap, overlay: Option<&Pixmap>) -> Result<Pixmap> {
    let mut canvas = source.clone();
    if let Some(overlay) = overlay {
        if overlay.width() != source.width() || overlay.height() != source.height() {
            return Err(ExportError(
                "annotation layer does not match the capture".into(),
            ));
        }
        canvas.draw_pixmap(
            0,
            0,
            overlay.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    Ok(canvas)
}

/// Straight-alpha sRGB RGBA from a premultiplied pixmap, for Rune's `ImageCache::rgba`.
pub fn straight_rgba(pixmap: &Pixmap) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((pixmap.width() * pixmap.height() * 4) as usize);
    for px in pixmap.pixels() {
        let c = px.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    rgba
}

/// Encode straight-alpha sRGB RGBA into a PNG.
pub fn encode_png(pixmap: &Pixmap) -> Result<Vec<u8>> {
    let rgba = straight_rgba(pixmap);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, pixmap.width(), pixmap.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        // Copying is an interactive action; a slightly larger file is worth the lower latency.
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc
            .write_header()
            .map_err(|error| ExportError(format!("cannot start the PNG: {error}")))?;
        writer
            .write_image_data(&rgba)
            .map_err(|error| ExportError(format!("cannot write the PNG: {error}")))?;
    }
    Ok(out)
}
