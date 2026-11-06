//! Composable visual interaction state flags.

/// A set of visual states that may coexist for one control.
///
/// For example, a selected row may also be focused and busy. Consumers should resolve semantic
/// precedence while preserving every flag rather than collapsing the state into an exclusive enum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InteractionState(u8);

impl InteractionState {
    pub const HOVERED: Self = Self(1 << 0);
    pub const SELECTED: Self = Self(1 << 1);
    pub const FOCUSED: Self = Self(1 << 2);
    pub const DISABLED: Self = Self(1 << 3);
    pub const BUSY: Self = Self(1 << 4);
    pub const ERROR: Self = Self(1 << 5);

    /// Returns an empty state set.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Returns a set containing all flags from `self` and `state`.
    #[must_use]
    pub const fn with(self, state: Self) -> Self {
        Self(self.0 | state.0)
    }

    /// Returns whether every flag in `state` is present.
    pub const fn contains(self, state: Self) -> bool {
        self.0 & state.0 == state.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_and_focused_states_can_coexist() {
        let state = InteractionState::empty()
            .with(InteractionState::SELECTED)
            .with(InteractionState::FOCUSED);

        assert!(state.contains(InteractionState::SELECTED));
        assert!(state.contains(InteractionState::FOCUSED));
        assert!(!state.contains(InteractionState::DISABLED));
    }
}
