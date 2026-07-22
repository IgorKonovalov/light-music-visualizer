//! The preset browse overlay's modal state — pure and window-free, so it is
//! unit-testable without winit or a GPU (Plan 0008). The shell decodes platform
//! key events into [`OverlayKey`]s, feeds them here, and acts on the returned
//! [`OverlayAction`]; each frame while open it asks [`OverlayState::visible`]
//! for the rows to draw. Type-to-filter arrives in Phase 4.

/// A key the overlay reacts to, decoded from the platform's input upstream so
/// this module stays free of winit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKey {
    /// Open the overlay when closed, close it when open.
    Toggle,
    /// Move the highlight up one row (clamped at the top).
    Up,
    /// Move the highlight down one row (clamped at the bottom).
    Down,
    /// Commit the highlighted preset and close.
    Enter,
    /// Close without selecting.
    Escape,
}

/// What the shell should do after a key is fed to the overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayAction {
    /// The key was ignored (e.g. a nav key while closed) — the shell's normal
    /// bindings (Space-cycle, …) stay in effect.
    None,
    /// Visible state changed; the shell should request a redraw.
    Redraw,
    /// Close the overlay without changing the preset.
    Close,
    /// Select the preset at this **absolute** roster index, then the overlay
    /// has closed itself.
    Select(usize),
}

/// The overlay's modal state: whether it is open and which visible row is
/// highlighted. The roster is not owned here — the caller passes the current
/// preset names into each method, so a hot-reload that swaps the roster needs no
/// coordination with this state (Phase 4 leans on that).
#[derive(Clone, Debug, Default)]
pub struct OverlayState {
    open: bool,
    /// Index into the *visible* list (== absolute index until Phase 4 filters).
    highlight: usize,
}

impl OverlayState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the overlay is currently open (the shell draws its list and
    /// suppresses Space-cycle while it is).
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The highlighted row's index into the visible list.
    pub fn highlight(&self) -> usize {
        self.highlight
    }

    /// The rows to display as `(absolute roster index, name)`. Phase 3 shows the
    /// whole roster in order; Phase 4 narrows it by the filter. The absolute
    /// index is what [`OverlayAction::Select`] carries, so selection stays
    /// correct once the visible list is a filtered subset.
    pub fn visible<'a>(&self, names: &[&'a str]) -> Vec<(usize, &'a str)> {
        names.iter().enumerate().map(|(i, &n)| (i, n)).collect()
    }

    /// Feed one key; mutate state and report what the shell should do. `names`
    /// is the current roster in order.
    pub fn handle_key(&mut self, key: OverlayKey, names: &[&str]) -> OverlayAction {
        // Toggle works regardless of open state; opening resets the highlight.
        if key == OverlayKey::Toggle {
            self.open = !self.open;
            if self.open {
                self.highlight = 0;
            }
            return OverlayAction::Redraw;
        }
        // Every other key is inert while closed, so Space-cycle et al. are
        // unaffected (the shell only cycles when this returns `None`).
        if !self.open {
            return OverlayAction::None;
        }
        match key {
            OverlayKey::Up => {
                self.step(false, names);
                OverlayAction::Redraw
            }
            OverlayKey::Down => {
                self.step(true, names);
                OverlayAction::Redraw
            }
            OverlayKey::Enter => {
                let visible = self.visible(names);
                self.open = false;
                match visible.get(self.highlight) {
                    Some(&(abs, _)) => OverlayAction::Select(abs),
                    None => OverlayAction::Close, // empty list: nothing to pick
                }
            }
            OverlayKey::Escape => {
                self.open = false;
                OverlayAction::Close
            }
            // Handled above; kept for exhaustiveness without `unreachable!`.
            OverlayKey::Toggle => OverlayAction::None,
        }
    }

    /// Move the highlight one row, clamped to the visible list (no wrap).
    fn step(&mut self, down: bool, names: &[&str]) {
        let len = self.visible(names).len();
        if len == 0 {
            self.highlight = 0;
            return;
        }
        let last = len - 1;
        self.highlight = if down {
            (self.highlight + 1).min(last)
        } else {
            self.highlight.saturating_sub(1)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{OverlayAction, OverlayKey, OverlayState};

    const NAMES: [&str; 4] = ["alpha", "bravo", "charlie", "delta"];

    fn names() -> Vec<&'static str> {
        NAMES.to_vec()
    }

    #[test]
    fn nav_keys_are_inert_while_closed() {
        let mut s = OverlayState::new();
        let n = names();
        // A closed overlay ignores nav keys so the shell's Space-cycle still runs.
        assert_eq!(s.handle_key(OverlayKey::Down, &n), OverlayAction::None);
        assert_eq!(s.handle_key(OverlayKey::Enter, &n), OverlayAction::None);
        assert_eq!(s.handle_key(OverlayKey::Escape, &n), OverlayAction::None);
        assert!(!s.is_open());
    }

    #[test]
    fn open_navigate_and_select_emits_the_absolute_index() {
        let mut s = OverlayState::new();
        let n = names();
        assert_eq!(s.handle_key(OverlayKey::Toggle, &n), OverlayAction::Redraw);
        assert!(s.is_open());
        assert_eq!(s.highlight(), 0);
        assert_eq!(s.handle_key(OverlayKey::Down, &n), OverlayAction::Redraw);
        assert_eq!(s.highlight(), 1);
        assert_eq!(s.handle_key(OverlayKey::Down, &n), OverlayAction::Redraw);
        assert_eq!(s.highlight(), 2); // the third row
        // Enter selects the third preset's absolute index and closes.
        assert_eq!(
            s.handle_key(OverlayKey::Enter, &n),
            OverlayAction::Select(2)
        );
        assert!(!s.is_open());
    }

    #[test]
    fn escape_closes_without_selecting() {
        let mut s = OverlayState::new();
        let n = names();
        s.handle_key(OverlayKey::Toggle, &n);
        assert_eq!(s.handle_key(OverlayKey::Escape, &n), OverlayAction::Close);
        assert!(!s.is_open());
    }

    #[test]
    fn down_clamps_at_the_last_row_no_wrap() {
        let mut s = OverlayState::new();
        let n = names();
        s.handle_key(OverlayKey::Toggle, &n);
        for _ in 0..10 {
            s.handle_key(OverlayKey::Down, &n);
        }
        assert_eq!(s.highlight(), 3); // last index, never wraps to 0
        assert_eq!(
            s.handle_key(OverlayKey::Enter, &n),
            OverlayAction::Select(3)
        );
    }

    #[test]
    fn up_clamps_at_the_top() {
        let mut s = OverlayState::new();
        let n = names();
        s.handle_key(OverlayKey::Toggle, &n);
        s.handle_key(OverlayKey::Up, &n); // already at the top
        assert_eq!(s.highlight(), 0);
    }
}
