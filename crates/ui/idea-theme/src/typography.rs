//! Platform-aware font fallback and type scale tokens.

use crate::LogicalPixels;

/// Named font weights used by the initial type scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontWeight {
    Regular,
    Medium,
    Semibold,
}

impl FontWeight {
    /// Returns the corresponding CSS/OpenType numeric weight.
    pub const fn value(self) -> u16 {
        match self {
            Self::Regular => 400,
            Self::Medium => 500,
            Self::Semibold => 600,
        }
    }
}

/// Typography tokens and ordered platform font fallbacks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IdeaTypography {
    pub ui_font_families: &'static [&'static str],
    pub monospace_font_families: &'static [&'static str],
    pub body_size: LogicalPixels,
    pub body_line_height: LogicalPixels,
    pub compact_size: LogicalPixels,
    pub compact_line_height: LogicalPixels,
    pub title_size: LogicalPixels,
    pub body_weight: FontWeight,
    pub emphasis_weight: FontWeight,
    pub title_weight: FontWeight,
}

impl IdeaTypography {
    /// Returns the initial type scale and font fallback list for the target platform.
    pub const fn platform_default() -> Self {
        Self {
            ui_font_families: ui_font_families(),
            monospace_font_families: monospace_font_families(),
            body_size: LogicalPixels::from_static(13.0),
            body_line_height: LogicalPixels::from_static(20.0),
            compact_size: LogicalPixels::from_static(12.0),
            compact_line_height: LogicalPixels::from_static(18.0),
            title_size: LogicalPixels::from_static(16.0),
            body_weight: FontWeight::Regular,
            emphasis_weight: FontWeight::Medium,
            title_weight: FontWeight::Semibold,
        }
    }
}

#[cfg(target_os = "windows")]
const fn ui_font_families() -> &'static [&'static str] {
    &["Segoe UI", "Microsoft YaHei UI", "sans-serif"]
}

#[cfg(target_os = "macos")]
const fn ui_font_families() -> &'static [&'static str] {
    &[".AppleSystemUIFont", "PingFang SC", "sans-serif"]
}

#[cfg(target_os = "linux")]
const fn ui_font_families() -> &'static [&'static str] {
    &["Inter", "Noto Sans", "Noto Sans CJK SC", "sans-serif"]
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
const fn ui_font_families() -> &'static [&'static str] {
    &["sans-serif"]
}

#[cfg(target_os = "windows")]
const fn monospace_font_families() -> &'static [&'static str] {
    &["JetBrains Mono", "Cascadia Mono", "Consolas", "monospace"]
}

#[cfg(target_os = "macos")]
const fn monospace_font_families() -> &'static [&'static str] {
    &["JetBrains Mono", "SF Mono", "Menlo", "monospace"]
}

#[cfg(target_os = "linux")]
const fn monospace_font_families() -> &'static [&'static str] {
    &[
        "JetBrains Mono",
        "Noto Sans Mono",
        "DejaVu Sans Mono",
        "monospace",
    ]
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
const fn monospace_font_families() -> &'static [&'static str] {
    &["monospace"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_typography_has_safe_fallbacks() {
        let typography = IdeaTypography::platform_default();

        assert_eq!(typography.ui_font_families.last(), Some(&"sans-serif"));
        assert_eq!(
            typography.monospace_font_families.last(),
            Some(&"monospace")
        );
        assert!(typography.body_line_height > typography.body_size);
        assert_eq!(typography.title_weight.value(), 600);
    }
}
