//! Embedded faces. Rune replaces its bundled Inter when `AppSpec::fonts` is non-empty, so both
//! the UI face and the annotation face are embedded here.

pub const INTER_REGULAR: &[u8] = include_bytes!("../assets/Inter-Regular.ttf");
pub const INTER_MEDIUM: &[u8] = include_bytes!("../assets/Inter-Medium.ttf");
pub const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/Inter-SemiBold.ttf");
pub const ADWAITA_SANS: &[u8] = include_bytes!("../assets/AdwaitaSans-Regular.ttf");

pub const FONTS: &[&[u8]] = &[INTER_REGULAR, INTER_MEDIUM, INTER_SEMIBOLD, ADWAITA_SANS];

/// The UI sans slot.
pub const UI_FAMILY: &str = "Inter";
/// The annotation family, matched by the face's internal name.
pub const ANNOTATION_FAMILY: &str = "Adwaita Sans";
