//! Decode the capture once. The same straight-alpha bytes feed Rune's image cache and the
//! exporter.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tiny_skia::Pixmap;

use crate::export::ExportError;

#[derive(Clone)]
pub struct SourceImage {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha sRGB RGBA, row-major.
    pub rgba: Arc<[u8]>,
}

#[derive(Debug)]
pub enum LoadError {
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    Decode {
        path: PathBuf,
        error: String,
    },
    TooLarge {
        width: u32,
        height: u32,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, error } => write!(f, "cannot read {}: {error}", path.display()),
            Self::Decode { path, error } => {
                write!(f, "cannot decode {}: {error}", path.display())
            }
            Self::TooLarge { width, height } => {
                write!(f, "capture is too large to annotate: {width}x{height}")
            }
        }
    }
}

impl std::error::Error for LoadError {}

impl SourceImage {
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let bytes = std::fs::read(path).map_err(|error| LoadError::Io {
            path: path.to_owned(),
            error,
        })?;
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|error| LoadError::Decode {
                path: path.to_owned(),
                error: error.to_string(),
            })?;
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        // The clipboard sink caps at 64 megapixels; refuse sooner with a clear message.
        if u64::from(width) * u64::from(height) > 64 * 1024 * 1024 {
            return Err(LoadError::TooLarge { width, height });
        }
        Ok(Self {
            width,
            height,
            rgba: Arc::from(rgba.into_raw().into_boxed_slice()),
        })
    }

    /// The source as a premultiplied pixmap for compositing.
    pub fn to_pixmap(&self) -> Result<Pixmap, ExportError> {
        let mut pixmap = Pixmap::new(self.width, self.height)
            .ok_or_else(|| ExportError("cannot allocate the capture canvas".into()))?;
        let out = pixmap.data_mut();
        for (i, chunk) in self.rgba.chunks_exact(4).enumerate() {
            let premul =
                tiny_skia::ColorU8::from_rgba(chunk[0], chunk[1], chunk[2], chunk[3]).premultiply();
            let at = i * 4;
            out[at] = premul.red();
            out[at + 1] = premul.green();
            out[at + 2] = premul.blue();
            out[at + 3] = premul.alpha();
        }
        Ok(pixmap)
    }
}
