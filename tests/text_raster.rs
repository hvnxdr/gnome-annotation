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
fn a_light_fill_gets_a_dark_halo() {
    let mut renderer = TextRenderer::new();
    let raster = renderer
        .rasterize("H", 64.0, Rgba::ANNOTATION_YELLOW)
        .expect("text raster");

    let mut fill = false;
    let mut halo = false;
    for px in raster.pixmap.pixels() {
        let c = px.demultiply();
        let [r, g, b, a] = [c.red(), c.green(), c.blue(), c.alpha()];
        fill |= r > 200 && g > 170 && b < 100 && a > 200;
        halo |= r < 30 && g < 30 && b < 30 && a > 180;
    }
    assert!(fill, "the yellow fill is drawn");
    assert!(halo, "yellow text gets a dark halo");
}

#[test]
fn a_dark_fill_gets_a_light_halo() {
    let mut renderer = TextRenderer::new();
    let raster = renderer
        .rasterize("H", 64.0, Rgba::ANNOTATION_BLUE)
        .expect("text raster");

    let mut fill = false;
    let mut halo = false;
    for px in raster.pixmap.pixels() {
        let c = px.demultiply();
        let [r, g, b, a] = [c.red(), c.green(), c.blue(), c.alpha()];
        fill |= b > 150 && r < 120 && a > 200;
        halo |= r > 220 && g > 220 && b > 220 && a > 180;
    }
    assert!(fill, "the blue fill is drawn");
    assert!(halo, "blue text gets a light halo");
}

#[test]
fn a_long_string_lays_out_on_one_line() {
    let mut renderer = TextRenderer::new();
    let long = "M".repeat(120);
    let raster = renderer
        .rasterize(&long, 64.0, Rgba::ANNOTATION_RED)
        .expect("text raster");
    assert!(
        raster.pixmap.width() > 64 * 50,
        "the long line is not squeezed: {}x{}",
        raster.pixmap.width(),
        raster.pixmap.height()
    );
    assert!(
        (raster.pixmap.height() as f32) < 64.0 * 1.5,
        "120 characters stay one line tall: {}x{}",
        raster.pixmap.width(),
        raster.pixmap.height()
    );

    let two_lines = renderer
        .rasterize("M\nM", 64.0, Rgba::ANNOTATION_RED)
        .expect("text raster");
    assert!(
        (two_lines.pixmap.height() as f32) > 64.0 * 1.5,
        "an explicit newline adds a line: {}x{}",
        two_lines.pixmap.width(),
        two_lines.pixmap.height()
    );
}

#[test]
fn empty_text_has_no_raster() {
    let mut renderer = TextRenderer::new();
    assert!(renderer.rasterize("", 48.0, Rgba::ANNOTATION_RED).is_none());
}
