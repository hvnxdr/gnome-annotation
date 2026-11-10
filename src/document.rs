//! The annotation document: an ordered list of annotations in source-image pixels.

pub type Point = [f32; 2];

/// The smallest drag, in source-image pixels, that produces an annotation.
pub const MIN_DRAG_PX: f32 = 1.0;

/// An axis-aligned rectangle in source-image pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// The rectangle spanned by two drag corners, in any direction.
    pub fn normalized(start: Point, end: Point) -> Self {
        Self {
            x: start[0].min(end[0]),
            y: start[1].min(end[1]),
            w: (end[0] - start[0]).abs(),
            h: (end[1] - start[1]).abs(),
        }
    }

    /// Corners clockwise from the top left.
    pub fn corners(self) -> [Point; 4] {
        [
            [self.x, self.y],
            [self.x + self.w, self.y],
            [self.x + self.w, self.y + self.h],
            [self.x, self.y + self.h],
        ]
    }

    pub fn is_too_small(self) -> bool {
        self.w < MIN_DRAG_PX || self.h < MIN_DRAG_PX
    }

    /// The part of this rectangle inside `bounds`, with the same origin when they overlap.
    pub fn crop(self, bounds: Rect) -> Rect {
        let x0 = self.x.max(bounds.x);
        let y0 = self.y.max(bounds.y);
        let x1 = (self.x + self.w).min(bounds.x + bounds.w);
        let y1 = (self.y + self.h).min(bounds.y + bounds.h);
        if x1 <= x0 || y1 <= y0 {
            Rect {
                x: x0,
                y: y0,
                w: 0.0,
                h: 0.0,
            }
        } else {
            Rect {
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            }
        }
    }
}

/// Crop a segment to `bounds`, keeping its direction. `None` when no part is inside or the
/// visible part is too short to draw.
pub fn crop_segment(start: Point, end: Point, bounds: Rect) -> Option<(Point, Point)> {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let edges = [
        (-dx, start[0] - bounds.x),
        (dx, bounds.x + bounds.w - start[0]),
        (-dy, start[1] - bounds.y),
        (dy, bounds.y + bounds.h - start[1]),
    ];
    for (p, q) in edges {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            if r > t1 {
                return None;
            }
            if r > t0 {
                t0 = r;
            }
        } else {
            if r < t0 {
                return None;
            }
            if r < t1 {
                t1 = r;
            }
        }
    }
    let clipped_start = [start[0] + t0 * dx, start[1] + t0 * dy];
    let clipped_end = [start[0] + t1 * dx, start[1] + t1 * dy];
    (line_length(clipped_start, clipped_end) >= MIN_DRAG_PX).then_some((clipped_start, clipped_end))
}

/// The ellipse inscribed in `rect`, as a closed ring of points sampled from the true curve.
/// Both the preview and the exporter stroke the same points.
pub fn ellipse_points(rect: Rect) -> Vec<Point> {
    const SEGMENTS: usize = 96;
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    let rx = rect.w / 2.0;
    let ry = rect.h / 2.0;
    (0..SEGMENTS)
        .map(|i| {
            let angle = std::f32::consts::TAU * i as f32 / SEGMENTS as f32;
            [cx + rx * angle.cos(), cy + ry * angle.sin()]
        })
        .collect()
}

/// Straight-alpha sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };

    pub const ANNOTATION_RED: Self = Self {
        r: 224,
        g: 27,
        b: 36,
        a: 255,
    };

    pub const ANNOTATION_ORANGE: Self = Self {
        r: 255,
        g: 120,
        b: 0,
        a: 255,
    };

    pub const ANNOTATION_YELLOW: Self = Self {
        r: 246,
        g: 211,
        b: 45,
        a: 255,
    };

    pub const ANNOTATION_GREEN: Self = Self {
        r: 46,
        g: 194,
        b: 126,
        a: 255,
    };

    pub const ANNOTATION_BLUE: Self = Self {
        r: 53,
        g: 132,
        b: 228,
        a: 255,
    };

    pub const ANNOTATION_PURPLE: Self = Self {
        r: 145,
        g: 65,
        b: 172,
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

/// The six annotation colours offered by the toolbar, in palette order.
pub const PALETTE: [Rgba; 6] = [
    Rgba::ANNOTATION_RED,
    Rgba::ANNOTATION_ORANGE,
    Rgba::ANNOTATION_YELLOW,
    Rgba::ANNOTATION_GREEN,
    Rgba::ANNOTATION_BLUE,
    Rgba::ANNOTATION_PURPLE,
];

/// The three stroke widths offered by the toolbar, in screen points at the current fit scale.
pub const STROKE_WIDTHS_PT: [f32; 3] = [2.0, 4.0, 8.0];

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
    Line {
        start: Point,
        end: Point,
    },
    Arrow {
        start: Point,
        end: Point,
    },
    Rectangle {
        start: Point,
        end: Point,
    },
    Ellipse {
        start: Point,
        end: Point,
    },
    Redaction {
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

impl Annotation {
    /// Whether `point` lands on this annotation, using `tolerance_px` for thin strokes.
    pub fn hit_test(&self, point: Point, tolerance_px: f32) -> bool {
        self.kind.hit_test(point, tolerance_px)
    }
}

impl AnnotationKind {
    /// Whether `point` lands on this kind. Lines and arrows are hit by distance to the
    /// segment so a thin mark is easy to grab; boxes are hit by their interior.
    pub fn hit_test(&self, point: Point, _tolerance_px: f32) -> bool {
        match self {
            Self::Line { .. } | Self::Arrow { .. } => false,
            Self::Rectangle { start, end } | Self::Redaction { start, end } => {
                point_in_rect(point, Rect::normalized(*start, *end))
            }
            Self::Ellipse { start, end } => {
                let rect = Rect::normalized(*start, *end);
                if rect.w <= 0.0 || rect.h <= 0.0 {
                    return false;
                }
                let rx = rect.w / 2.0;
                let ry = rect.h / 2.0;
                let dx = (point[0] - (rect.x + rx)) / rx;
                let dy = (point[1] - (rect.y + ry)) / ry;
                dx * dx + dy * dy <= 1.0
            }
            Self::Text { .. } => false,
        }
    }
}

/// Whether `point` is inside `rect`, including its edges.
pub fn point_in_rect(point: Point, rect: Rect) -> bool {
    point[0] >= rect.x
        && point[0] <= rect.x + rect.w
        && point[1] >= rect.y
        && point[1] <= rect.y + rect.h
}

/// The arrow's shared geometry in source-image pixels. The preview and the exporter both use it.
#[derive(Clone, Copy, Debug)]
pub struct ArrowGeometry {
    pub start: Point,
    pub end: Point,
    pub head: [Point; 3],
}

/// Length of the segment from `start` to `end`, in source-image pixels.
pub fn line_length(start: Point, end: Point) -> f32 {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    (dx * dx + dy * dy).sqrt()
}

/// Geometry for an arrow, or `None` when the drag is too short to be an arrow.
pub fn arrow_geometry(start: Point, end: Point, stroke_px: f32) -> Option<ArrowGeometry> {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let len = line_length(start, end);
    if !len.is_finite() || len < MIN_DRAG_PX {
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
