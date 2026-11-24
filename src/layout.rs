//! Responsive layout rules shared by the editor and window hosts.

/// The largest initial window requested for a capture, in logical points.
pub const MAX_WINDOW: (f32, f32) = (1200.0, 800.0);

/// The normal minimum editor window size, in logical points.
pub const MIN_WINDOW: (f32, f32) = (480.0, 360.0);

/// Space left between the requested window and monitor bounds on X11.
pub const DISPLAY_MARGIN_PT: f32 = 32.0;

const EXPANDED_MIN_WIDTH_PT: f32 = 1100.0;
const NARROW_MAX_WIDTH_PT: f32 = 700.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    Expanded,
    Compact,
    Narrow,
}

pub fn layout_mode(width_pt: f32) -> LayoutMode {
    if width_pt >= EXPANDED_MIN_WIDTH_PT {
        LayoutMode::Expanded
    } else if width_pt >= NARROW_MAX_WIDTH_PT {
        LayoutMode::Compact
    } else {
        LayoutMode::Narrow
    }
}

/// Fit a capture aspect ratio inside the initial window cap and optional display bounds.
/// Displays smaller than the normal minimum may force a smaller window. The minimum can
/// override one window dimension for extreme captures; the editor canvas still keeps the
/// capture's aspect ratio.
pub fn initial_window_size(aspect: f32, display_pt: Option<[f32; 2]>) -> (f32, f32) {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    let available = available_size(display_pt);
    let max_width = MAX_WINDOW.0.min(available[0]);
    let max_height = MAX_WINDOW.1.min(available[1]);
    let (mut width, mut height) = if aspect >= max_width / max_height {
        (max_width, max_width / aspect)
    } else {
        (max_height * aspect, max_height)
    };

    let minimum = minimum_window_size(display_pt);
    width = width.max(minimum.0);
    height = height.max(minimum.1);
    (width, height)
}

/// The effective minimum window size for the display, in logical points.
pub fn minimum_window_size(display_pt: Option<[f32; 2]>) -> (f32, f32) {
    let available = available_size(display_pt);
    (
        MIN_WINDOW.0.min(available[0]),
        MIN_WINDOW.1.min(available[1]),
    )
}

fn available_size(display_pt: Option<[f32; 2]>) -> [f32; 2] {
    display_pt
        .map(|[width, height]| {
            [
                (width - DISPLAY_MARGIN_PT * 2.0).max(1.0),
                (height - DISPLAY_MARGIN_PT * 2.0).max(1.0),
            ]
        })
        .unwrap_or([MAX_WINDOW.0, MAX_WINDOW.1])
}

/// The primary display's logical size for app-side clamping, or the first display when none
/// is marked primary. Wayland returns `None` so the compositor owns work-area constraints.
pub fn primary_layout_bounds(displays: &[rune::Display]) -> Option<[f32; 2]> {
    let display = displays
        .iter()
        .find(|display| display.primary)
        .or_else(|| displays.first())?;
    let wayland = matches!(display.native_id(), Some(rune::NativeDisplayId::Wayland(_)));
    logical_layout_bounds(display.size, display.scale, wayland)
}

fn logical_layout_bounds(size: (u32, u32), scale: f64, wayland: bool) -> Option<[f32; 2]> {
    if wayland {
        return None;
    }
    let scale = if scale.is_finite() && scale > 0.0 {
        scale as f32
    } else {
        1.0
    };
    Some([size.0 as f32 / scale, size.1 as f32 / scale])
}

#[cfg(test)]
mod tests {
    use super::logical_layout_bounds;

    #[test]
    fn wayland_leaves_initial_bounds_to_the_compositor() {
        assert_eq!(logical_layout_bounds((1920, 1080), 2.0, true), None);
        assert_eq!(
            logical_layout_bounds((1920, 1080), 2.0, false),
            Some([960.0, 540.0])
        );
    }
}
