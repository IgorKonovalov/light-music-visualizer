//! The preset browse overlay's modal state — pure and window-free, so it is
//! unit-testable without winit or a GPU (Plan 0008). The shell decodes platform
//! key events into [`OverlayKey`]s, feeds them here, and acts on the returned
//! [`OverlayAction`]; each frame while open it asks [`OverlayState::visible`]
//! for the (filtered) rows to draw. Typed characters narrow the list by a
//! case-insensitive substring filter, and [`OverlayAction::Select`] always
//! carries the **absolute** roster index so a filtered pick stays correct.

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
    /// Append a printable character to the type-to-filter query.
    Char(char),
    /// Delete the last character of the filter query.
    Backspace,
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
    /// Index into the *visible* (filtered) list, not the absolute roster.
    highlight: usize,
    /// Case-insensitive substring filter; empty means "show the whole roster".
    filter: String,
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

    /// The current filter query (for the shell to echo, e.g. in the list header).
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// The rows to display as `(absolute roster index, name)`, narrowed by the
    /// case-insensitive substring filter (empty filter → the whole roster in
    /// order). The **absolute** index is what [`OverlayAction::Select`] carries,
    /// so selection stays correct even when the visible list is a filtered
    /// subset — an off-by-one here would silently pick the wrong preset.
    pub fn visible<'a>(&self, names: &[&'a str]) -> Vec<(usize, &'a str)> {
        if self.filter.is_empty() {
            return names.iter().enumerate().map(|(i, &n)| (i, n)).collect();
        }
        let needle = self.filter.to_lowercase();
        names
            .iter()
            .enumerate()
            .filter(|(_, n)| n.to_lowercase().contains(&needle))
            .map(|(i, &n)| (i, n))
            .collect()
    }

    /// Re-clamp the highlight after the roster changed under the overlay (a
    /// hot-reload swapped presets). Keeps the open state and the filter — only
    /// ensures the highlight still points at a visible row, so a shrunk roster
    /// or a filter that now matches fewer rows never leaves a stale highlight.
    pub fn on_roster_changed(&mut self, names: &[&str]) {
        let len = self.visible(names).len();
        if len == 0 {
            self.highlight = 0;
        } else if self.highlight >= len {
            self.highlight = len - 1;
        }
    }

    /// Feed one key; mutate state and report what the shell should do. `names`
    /// is the current roster in order.
    pub fn handle_key(&mut self, key: OverlayKey, names: &[&str]) -> OverlayAction {
        // Toggle works regardless of open state; opening starts a fresh query.
        if key == OverlayKey::Toggle {
            self.open = !self.open;
            if self.open {
                self.highlight = 0;
                self.filter.clear();
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
            // Type-to-filter: narrowing resets the highlight to the first match.
            OverlayKey::Char(c) => {
                self.filter.push(c);
                self.highlight = 0;
                OverlayAction::Redraw
            }
            OverlayKey::Backspace => {
                self.filter.pop();
                self.highlight = 0;
                OverlayAction::Redraw
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

    fn type_str(s: &mut OverlayState, text: &str, names: &[&str]) {
        for c in text.chars() {
            s.handle_key(OverlayKey::Char(c), names);
        }
    }

    #[test]
    fn typing_filters_case_insensitively_and_selects_absolute_index() {
        let mut s = OverlayState::new();
        // "warp" sits at absolute index 2 and is capitalized.
        let n = vec!["Aurora", "Ember", "Warp", "Glacier"];
        s.handle_key(OverlayKey::Toggle, &n);
        // Lowercase "war" matches "Warp" case-insensitively, nothing else.
        type_str(&mut s, "war", &n);
        let visible = s.visible(&n);
        assert_eq!(visible, [(2, "Warp")]);
        // Enter must carry Warp's ABSOLUTE index (2), not its filtered row (0).
        assert_eq!(
            s.handle_key(OverlayKey::Enter, &n),
            OverlayAction::Select(2)
        );
    }

    #[test]
    fn backspace_widens_the_filtered_list() {
        let mut s = OverlayState::new();
        let n = vec!["alpha", "altair", "beta"];
        s.handle_key(OverlayKey::Toggle, &n);
        type_str(&mut s, "alt", &n);
        assert_eq!(s.visible(&n).len(), 1); // only "altair"
        s.handle_key(OverlayKey::Backspace, &n); // -> "al"
        assert_eq!(s.visible(&n).len(), 2); // "alpha" + "altair" restored
    }

    #[test]
    fn no_match_filter_yields_an_empty_list_not_a_stale_one() {
        let mut s = OverlayState::new();
        let n = names();
        s.handle_key(OverlayKey::Toggle, &n);
        type_str(&mut s, "zzz", &n);
        assert!(s.visible(&n).is_empty());
        // Enter on an empty list closes without selecting.
        assert_eq!(s.handle_key(OverlayKey::Enter, &n), OverlayAction::Close);
    }

    #[test]
    fn reopening_clears_the_prior_filter() {
        let mut s = OverlayState::new();
        let n = names();
        s.handle_key(OverlayKey::Toggle, &n); // open
        type_str(&mut s, "zzz", &n); // filters to nothing
        assert!(s.visible(&n).is_empty());
        s.handle_key(OverlayKey::Toggle, &n); // close
        s.handle_key(OverlayKey::Toggle, &n); // reopen -> fresh query
        assert_eq!(s.filter(), "");
        assert_eq!(s.visible(&n).len(), 4);
    }

    #[test]
    fn roster_change_reclamps_the_highlight_and_keeps_open() {
        let mut s = OverlayState::new();
        let big = vec!["a", "b", "c", "d"];
        s.handle_key(OverlayKey::Toggle, &big);
        for _ in 0..3 {
            s.handle_key(OverlayKey::Down, &big);
        }
        assert_eq!(s.highlight(), 3);
        // A hot-reload shrinks the roster under the open overlay.
        let small = vec!["a", "b"];
        s.on_roster_changed(&small);
        assert_eq!(s.highlight(), 1); // clamped to the new last row
        assert!(s.is_open()); // still open
    }
}
