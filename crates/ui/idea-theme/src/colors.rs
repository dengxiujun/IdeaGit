//! Semantic color tokens for light and dark themes.

use std::{error::Error, fmt};

/// An opaque RGB color encoded as `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColorToken(u32);

impl ColorToken {
    const MAX_RGB: u32 = 0x00ff_ffff;

    /// Creates a color after validating that no bits outside `0xRRGGBB` are set.
    pub fn try_from_rgb(rgb: u32) -> Result<Self, InvalidRgb> {
        if rgb <= Self::MAX_RGB {
            Ok(Self(rgb))
        } else {
            Err(InvalidRgb { value: rgb })
        }
    }

    pub(crate) const fn from_static_rgb(rgb: u32) -> Self {
        Self(rgb)
    }

    /// Returns the color as `0xRRGGBB` for renderer adapters.
    pub const fn as_rgb(self) -> u32 {
        self.0
    }
}

/// Error returned when an RGB token contains unsupported high bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidRgb {
    value: u32,
}

impl InvalidRgb {
    /// Returns the rejected raw value.
    pub const fn value(self) -> u32 {
        self.value
    }
}

impl fmt::Display for InvalidRgb {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "RGB color 0x{:08X} exceeds the 0xRRGGBB range",
            self.value
        )
    }
}

impl Error for InvalidRgb {}

/// Semantic colors consumed by UI primitives instead of feature-specific literals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdeaColors {
    pub window_background: ColorToken,
    pub panel_background: ColorToken,
    pub editor_background: ColorToken,
    pub border: ColorToken,
    pub separator: ColorToken,
    pub text_primary: ColorToken,
    pub text_secondary: ColorToken,
    pub text_disabled: ColorToken,
    pub selection_background: ColorToken,
    pub hover_background: ColorToken,
    pub focus_border: ColorToken,
    pub git_added: ColorToken,
    pub git_modified: ColorToken,
    pub git_deleted: ColorToken,
    pub git_conflicted: ColorToken,
    pub busy: ColorToken,
    pub error: ColorToken,
}

impl IdeaColors {
    /// Returns the initial dark palette.
    pub const fn dark() -> Self {
        Self {
            window_background: color(0x2b2d30),
            panel_background: color(0x2b2d30),
            editor_background: color(0x1e1f22),
            border: color(0x393b40),
            separator: color(0x43454a),
            text_primary: color(0xdfe1e5),
            text_secondary: color(0x9da0a8),
            text_disabled: color(0x6f737a),
            selection_background: color(0x2e436e),
            hover_background: color(0x393b40),
            focus_border: color(0x3574f0),
            git_added: color(0x57965c),
            git_modified: color(0xddb75b),
            git_deleted: color(0xc75450),
            git_conflicted: color(0xdb5c5c),
            busy: color(0x3574f0),
            error: color(0xdb5c5c),
        }
    }

    /// Returns the initial light palette.
    pub const fn light() -> Self {
        Self {
            window_background: color(0xf7f8fa),
            panel_background: color(0xffffff),
            editor_background: color(0xffffff),
            border: color(0xd3d5d9),
            separator: color(0xdfe1e5),
            text_primary: color(0x1f2329),
            text_secondary: color(0x5f6670),
            text_disabled: color(0x9aa0a8),
            selection_background: color(0xdde8ff),
            hover_background: color(0xedf2fa),
            focus_border: color(0x315fbd),
            git_added: color(0x2e7d32),
            git_modified: color(0xa66f00),
            git_deleted: color(0xc0362c),
            git_conflicted: color(0xc43e3e),
            busy: color(0x315fbd),
            error: color(0xc43e3e),
        }
    }
}

const fn color(rgb: u32) -> ColorToken {
    ColorToken::from_static_rgb(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_and_dark_palettes_use_distinct_primary_surfaces() {
        let light = IdeaColors::light();
        let dark = IdeaColors::dark();

        assert_ne!(light.window_background, dark.window_background);
        assert_ne!(light.text_primary, dark.text_primary);
        assert_eq!(dark.focus_border.as_rgb(), 0x3574f0);
    }

    #[test]
    fn rejects_values_outside_rgb_range() {
        let error = ColorToken::try_from_rgb(0x0100_0000).expect_err("high bits must be rejected");

        assert_eq!(error.value(), 0x0100_0000);
    }
}
