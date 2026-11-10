//! Helpers shared by the headless editor tests.
#![allow(dead_code)]

pub mod fakes;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::RgbaImage;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// A unique temporary directory removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(name: &str) -> Self {
        let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "gnome-annotation-{name}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A 4K checkerboard that no annotation colour can be mistaken for.
pub fn write_4k_fixture(dir: &Path) -> PathBuf {
    const W: u32 = 3840;
    const H: u32 = 2160;
    const TILE: u32 = 96;
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let light = ((x / TILE) + (y / TILE)) % 2 == 0;
            let v = if light { 72u8 } else { 36u8 };
            pixels.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let path = dir.join("capture.png");
    write_png(&path, W, H, &pixels);
    path
}

pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().expect("png header");
    writer.write_image_data(rgba).expect("png data");
}

pub fn decode_png(bytes: &[u8]) -> RgbaImage {
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .expect("decode png")
        .to_rgba8()
}

/// Bounding box of pixels matching `predicate`, in image coordinates.
pub fn pixel_bounds(image: &RgbaImage, predicate: impl Fn([u8; 4]) -> bool) -> Option<[u32; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, px) in image.enumerate_pixels() {
        if predicate(px.0) {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    (x0 != u32::MAX).then_some([x0, y0, x1, y1])
}

/// Pixels close to the annotation red used by the editor.
pub fn is_annotation_red(px: [u8; 4]) -> bool {
    px[0] > 150 && px[1] < 90 && px[2] < 90 && px[3] > 200
}
