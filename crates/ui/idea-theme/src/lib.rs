//! IDEA-inspired visual tokens shared by all UI crates.
//!
//! UI-002 defines semantic colors, typography, logical metrics, and composable interaction
//! states. Values are engineering defaults rather than pixel measurements of IntelliJ IDEA.
//! GPUI conversion and runtime theme switching belong to later primitive and workbench tasks.

mod colors;
mod interaction;
mod metrics;
mod theme;
mod typography;

pub use colors::{ColorToken, IdeaColors, InvalidRgb};
pub use interaction::InteractionState;
pub use metrics::{IdeaMetrics, InvalidLogicalPixels, LogicalPixels};
pub use theme::{IdeaTheme, ThemeMode};
pub use typography::{FontWeight, IdeaTypography};
