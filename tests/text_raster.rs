//! The shaped text raster is shared by the preview and the export, so its orientation and
//! halo are checked directly as well.

use gnome_annotation::document::Rgba;
use gnome_annotation::text::TextRenderer;

fn opaque_red(raster: &tiny_skia::Pixmap) -> Vec<(u32, u32)> {
    let mut points = Vec::new();
    for (i, px) in raster.pixels().iter().enumerate() {
        let c = px.demultiply();
        if c.red() > 180 && c.green() < 90 && c.blue() < 90 && c.alpha() > 200 {
            points.push(((i as u32) % raster.width(), (i as u32) / raster.width()));
        }
    }
    points
}

#[test]
fn glyphs_are_upright_and_the_halo_surrounds_the_fill() {
    let mut renderer = TextRenderer::new();
    let raster = renderer
        .rasterize("A", 100.0, Rgba::ANNOTATION_RED)
        .expect("text raster");
    let red = opaque_red(&raster.pixmap);
    assert!(!red.is_empty(), "the fill produced pixels");

    // "A" is narrow at the top and wide at the bottom. If the y flip were missing this
    // comparison would invert.
    let width_in = |min: u32, max: u32| {
        let xs: Vec<u32> = red
            .iter()
            .filter(|(_, y)| *y >= min && *y < max)
            .map(|(x, _)| *x)
            .collect();
        xs.iter()
            .max()
            .copied()
            .unwrap_or(0)
            .saturating_sub(xs.iter().min().copied().unwrap_or(0))
    };
    let h = raster.pixmap.height();
    let top = width_in(0, h / 4);
    let bottom = width_in(h * 3 / 4, h);
    assert!(
        bottom > top,
        "A is upright: top={top} bottom={bottom} ({}x{})",
        raster.pixmap.width(),
        raster.pixmap.height()
    );

    // Red text has a white halo by luminance. Find a white, mostly opaque pixel.
    let halo = raster.pixmap.pixels().iter().any(|px| {
        let c = px.demultiply();
        c.red() > 220 && c.green() > 220 && c.blue() > 220 && c.alpha() > 180
    });
    assert!(halo, "a contrasting halo surrounds the fill");
}

#[test]
fn empty_text_has_no_raster() {
    let mut renderer = TextRenderer::new();
    assert!(renderer.rasterize("", 48.0, Rgba::ANNOTATION_RED).is_none());
}
