//! Responsive layout breakpoints and initial window sizing.

use gnome_annotation::layout::{
    LayoutMode, initial_window_size, layout_mode, minimum_window_size, primary_layout_bounds,
};
use rune::Display;

fn near(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.01,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn layout_modes_use_logical_point_breakpoints() {
    assert_eq!(layout_mode(1200.0), LayoutMode::Expanded);
    assert_eq!(layout_mode(1100.0), LayoutMode::Expanded);
    assert_eq!(layout_mode(1099.0), LayoutMode::Compact);
    assert_eq!(layout_mode(700.0), LayoutMode::Compact);
    assert_eq!(layout_mode(699.0), LayoutMode::Narrow);
    assert_eq!(layout_mode(480.0), LayoutMode::Narrow);
}

#[test]
fn initial_window_respects_the_cap_and_minimum() {
    let (width, height) = initial_window_size(16.0 / 9.0, None);
    near(width, 1200.0);
    near(height, 675.0);

    let (width, height) = initial_window_size(9.0 / 16.0, None);
    near(width, 480.0);
    near(height, 800.0);
}

#[test]
fn initial_window_clamps_to_display_bounds_with_a_margin() {
    let (width, height) = initial_window_size(16.0 / 9.0, Some([1000.0, 600.0]));
    near(width, 936.0);
    near(height, 526.5);
}

#[test]
fn tiny_displays_can_force_a_window_below_the_normal_minimum() {
    let (width, height) = initial_window_size(16.0 / 9.0, Some([512.0, 300.0]));
    near(width, 448.0);
    near(height, 236.0);
    assert_eq!(minimum_window_size(Some([512.0, 300.0])), (448.0, 236.0));
}

#[test]
fn normal_displays_keep_the_usable_minimum_for_extreme_captures() {
    let (width, height) = initial_window_size(0.1, None);
    near(width, 480.0);
    near(height, 800.0);

    let (width, height) = initial_window_size(4.0, None);
    near(width, 1200.0);
    near(height, 360.0);
}

#[test]
fn display_pixels_convert_to_logical_points() {
    let display = Display {
        id: None,
        name: Some("test".into()),
        position: (0, 0),
        size: (1920, 1080),
        scale: 2.0,
        primary: true,
    };
    assert_eq!(primary_layout_bounds(&[display]), Some([960.0, 540.0]));
    assert_eq!(minimum_window_size(None), (480.0, 360.0));
}
