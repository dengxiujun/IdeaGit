//! Logical layout metrics independent of physical display scale.

use std::{error::Error, fmt};

/// A finite, positive logical-pixel value.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct LogicalPixels(f32);

impl LogicalPixels {
    /// Creates a logical-pixel value after validating its range.
    pub fn try_new(value: f32) -> Result<Self, InvalidLogicalPixels> {
        if value.is_finite() && value > 0.0 {
            Ok(Self(value))
        } else {
            Err(InvalidLogicalPixels { value })
        }
    }

    pub(crate) const fn from_static(value: f32) -> Self {
        Self(value)
    }

    /// Returns the renderer-independent numeric value.
    pub const fn get(self) -> f32 {
        self.0
    }
}

/// Error returned for zero, negative, infinite, or NaN logical metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InvalidLogicalPixels {
    value: f32,
}

impl InvalidLogicalPixels {
    /// Returns the rejected numeric value.
    pub const fn value(self) -> f32 {
        self.value
    }
}

impl fmt::Display for InvalidLogicalPixels {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "logical pixels must be finite and greater than zero, got {}",
            self.value
        )
    }
}

impl Error for InvalidLogicalPixels {}

/// Initial IDEA-inspired component metrics in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IdeaMetrics {
    pub toolbar_height: LogicalPixels,
    pub statusbar_height: LogicalPixels,
    pub tree_row_height: LogicalPixels,
    pub compact_row_height: LogicalPixels,
    pub tab_height: LogicalPixels,
    pub panel_header_height: LogicalPixels,
    pub icon_size: LogicalPixels,
    pub panel_padding: LogicalPixels,
    pub splitter_width: LogicalPixels,
}

impl IdeaMetrics {
    /// Returns the engineering defaults from the approved UI design.
    pub const fn initial() -> Self {
        Self {
            toolbar_height: lp(36.0),
            statusbar_height: lp(26.0),
            tree_row_height: lp(24.0),
            compact_row_height: lp(22.0),
            tab_height: lp(30.0),
            panel_header_height: lp(32.0),
            icon_size: lp(16.0),
            panel_padding: lp(8.0),
            splitter_width: lp(4.0),
        }
    }
}

const fn lp(value: f32) -> LogicalPixels {
    LogicalPixels::from_static(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_metrics_match_the_design_baseline() {
        let metrics = IdeaMetrics::initial();

        assert_eq!(metrics.toolbar_height.get(), 36.0);
        assert_eq!(metrics.panel_header_height.get(), 32.0);
        assert_eq!(metrics.splitter_width.get(), 4.0);
    }

    #[test]
    fn rejects_invalid_logical_pixels() {
        assert!(LogicalPixels::try_new(0.0).is_err());
        assert!(LogicalPixels::try_new(-1.0).is_err());
        assert!(LogicalPixels::try_new(f32::NAN).is_err());
        assert!(LogicalPixels::try_new(f32::INFINITY).is_err());
    }
}
