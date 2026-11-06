//! Complete theme bundles selected by appearance mode.

use crate::{IdeaColors, IdeaMetrics, IdeaTypography};

/// Supported application appearance modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeMode {
    Light,
    Dark,
}

/// A complete immutable set of UI-002 design tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IdeaTheme {
    pub mode: ThemeMode,
    pub colors: IdeaColors,
    pub typography: IdeaTypography,
    pub metrics: IdeaMetrics,
}

impl IdeaTheme {
    /// Returns the complete token bundle for an appearance mode.
    pub const fn for_mode(mode: ThemeMode) -> Self {
        let colors = match mode {
            ThemeMode::Light => IdeaColors::light(),
            ThemeMode::Dark => IdeaColors::dark(),
        };
        Self {
            mode,
            colors,
            typography: IdeaTypography::platform_default(),
            metrics: IdeaMetrics::initial(),
        }
    }

    /// Returns the initial light theme.
    pub const fn light() -> Self {
        Self::for_mode(ThemeMode::Light)
    }

    /// Returns the initial dark theme.
    pub const fn dark() -> Self {
        Self::for_mode(ThemeMode::Dark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_selects_the_matching_palette_and_shared_scale() {
        let light = IdeaTheme::light();
        let dark = IdeaTheme::dark();

        assert_eq!(light.mode, ThemeMode::Light);
        assert_eq!(dark.mode, ThemeMode::Dark);
        assert_ne!(light.colors, dark.colors);
        assert_eq!(light.metrics, dark.metrics);
        assert_eq!(light.typography, dark.typography);
    }
}
