//! The annotation document: an ordered list of annotations in source-image pixels.

pub type Point = [f32; 2];

/// Straight-alpha sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const ANNOTATION_RED: Self = Self {
        r: 224,
        g: 27,
        b: 36,
        a: 255,
    };

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Relative luminance in the 0..1 range, used to pick a contrasting halo.
    pub fn luminance(self) -> f32 {
        let channel = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.039_28 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }
}

/// One annotation. The kind carries the geometry; every kind shares colour and stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub id: u64,
    pub kind: AnnotationKind,
    pub color: Rgba,
    pub stroke_px: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AnnotationKind {
    Arrow {
        start: Point,
        end: Point,
    },
    Text {
        position: Point,
        text: String,
        size_px: f32,
    },
}

/// The document list is also the draw order: later annotations paint above earlier ones.
#[derive(Clone, Debug, Default)]
pub struct Document {
    annotations: Vec<Annotation>,
    next_id: u64,
}

impl Document {
    pub fn new() -> Self {
        Self {
            annotations: Vec::new(),
            next_id: 1,
        }
    }

    pub fn push(&mut self, kind: AnnotationKind, color: Rgba, stroke_px: f32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.annotations.push(Annotation {
            id,
            kind,
            color,
            stroke_px,
        });
        id
    }

    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }
}

/// The arrow's shared geometry in source-image pixels. The preview and the exporter both use it.
#[derive(Clone, Copy, Debug)]
pub struct ArrowGeometry {
    pub start: Point,
    pub end: Point,
    pub head: [Point; 3],
}

/// Geometry for an arrow, or `None` when the drag is too short to be an arrow.
pub fn arrow_geometry(start: Point, end: Point, stroke_px: f32) -> Option<ArrowGeometry> {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let len = (dx * dx + dy * dy).sqrt();
    if !len.is_finite() || len < 1.0 {
        return None;
    }
    let dir = [dx / len, dy / len];
    let normal = [-dir[1], dir[0]];
    let head_len = (stroke_px * 4.0).max(12.0).min(len * 0.6);
    let half = head_len * 0.6;
    let base = [end[0] - dir[0] * head_len, end[1] - dir[1] * head_len];
    let head = [
        end,
        [base[0] + normal[0] * half, base[1] + normal[1] * half],
        [base[0] - normal[0] * half, base[1] - normal[1] * half],
    ];
    Some(ArrowGeometry { start, end, head })
}
