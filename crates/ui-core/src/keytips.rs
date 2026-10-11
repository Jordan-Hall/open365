//! The Alt key tips state machine, as Office does it: a bare Alt press and release shows the
//! tab tips; typing a tip's letters descends one level (a tab, then its commands); Escape goes
//! up a level and leaves at the top; any other key or a pointer press cancels. Alt used as a
//! modifier (Alt+Letter shortcuts) never opens the tips.

/// What the machine asks the UI to do after an input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TipOutcome {
    /// Nothing to do.
    None,
    /// The visible set of tips changed (or appeared / disappeared).
    Changed,
    /// A tip was completed: the id of its target.
    Activate(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TipState {
    Idle,
    /// Alt is down and nothing else happened yet.
    AltDown,
    /// Tips are shown. `typed` is the prefix typed so far across the current level.
    Showing,
}

/// One tip: its full key sequence (`"H"`, `"FP"`) and the id of its target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tip {
    pub keys: String,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct KeyTips {
    state: TipState,
    /// Tips of the current level.
    tips: Vec<Tip>,
    /// The keys typed within this level.
    typed: String,
    /// Stack of earlier levels' tips (restored by Escape).
    parents: Vec<Vec<Tip>>,
    /// Target of the tip that was just descended into (taken by the UI to select a tab).
    descended: Option<String>,
}

impl Default for KeyTips {
    fn default() -> Self {
        Self {
            state: TipState::Idle,
            tips: Vec::new(),
            typed: String::new(),
            parents: Vec::new(),
            descended: None,
        }
    }
}

impl KeyTips {
    pub fn state(&self) -> &TipState {
        &self.state
    }

    pub fn is_showing(&self) -> bool {
        self.state == TipState::Showing
    }

    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// The tips still matching what was typed.
    pub fn visible(&self) -> impl Iterator<Item = &Tip> {
        let typed = self.typed.clone();
        self.tips.iter().filter(move |t| t.keys.starts_with(&typed))
    }

    /// The target of the tip just descended into, once.
    pub fn take_descended(&mut self) -> Option<String> {
        self.descended.take()
    }

    pub fn depth(&self) -> usize {
        self.parents.len()
    }

    /// Alt went down. `repeat` is an auto-repeat of a held key.
    pub fn alt_pressed(&mut self, repeat: bool) -> TipOutcome {
        if repeat {
            return TipOutcome::None;
        }
        if self.state == TipState::Showing {
            // Pressing Alt again leaves the tips.
            return self.reset();
        }
        self.state = TipState::AltDown;
        TipOutcome::None
    }

    /// Alt came up. Shows the root tips when nothing intervened.
    pub fn alt_released(&mut self, root: Vec<Tip>) -> TipOutcome {
        if self.state != TipState::AltDown {
            return TipOutcome::None;
        }
        self.state = TipState::Showing;
        self.tips = root;
        self.typed.clear();
        self.parents.clear();
        TipOutcome::Changed
    }

    /// Any other key while Alt is held makes Alt a plain modifier.
    pub fn other_key_with_alt(&mut self) {
        if self.state == TipState::AltDown {
            self.state = TipState::Idle;
        }
    }

    /// A letter or digit while tips show. `children` supplies the tips of the next level when the
    /// typed keys complete a tip that has sub-tips (for example a tab's commands).
    pub fn letter(&mut self, c: char, children: &dyn Fn(&str) -> Vec<Tip>) -> TipOutcome {
        if self.state != TipState::Showing {
            return TipOutcome::None;
        }
        let c = c.to_ascii_uppercase();
        let mut probe = self.typed.clone();
        probe.push(c);
        if !self.tips.iter().any(|t| t.keys.starts_with(&probe)) {
            return TipOutcome::None;
        }
        self.typed = probe;
        let exact = self.tips.iter().find(|t| t.keys == self.typed).cloned();
        if let Some(tip) = exact {
            let next = children(&tip.target);
            if next.is_empty() {
                self.reset();
                return TipOutcome::Activate(tip.target);
            }
            self.descended = Some(tip.target.clone());
            self.parents.push(std::mem::replace(&mut self.tips, next));
            self.typed.clear();
        }
        TipOutcome::Changed
    }

    /// Escape: clear the typed prefix, else go up a level, else leave.
    pub fn escape(&mut self) -> TipOutcome {
        if self.state != TipState::Showing {
            return TipOutcome::None;
        }
        if !self.typed.is_empty() {
            self.typed.clear();
            return TipOutcome::Changed;
        }
        match self.parents.pop() {
            Some(p) => {
                self.tips = p;
                TipOutcome::Changed
            }
            None => self.reset(),
        }
    }

    /// Any pointer press, focus loss or unrelated key.
    pub fn cancel(&mut self) -> TipOutcome {
        self.reset()
    }

    fn reset(&mut self) -> TipOutcome {
        let was = self.state != TipState::Idle;
        *self = Self::default();
        if was {
            TipOutcome::Changed
        } else {
            TipOutcome::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip(k: &str, t: &str) -> Tip {
        Tip {
            keys: k.into(),
            target: t.into(),
        }
    }

    fn root() -> Vec<Tip> {
        vec![
            tip("H", "tab.home"),
            tip("N", "tab.insert"),
            tip("JI", "tab.draw"),
            tip("F", "file"),
        ]
    }

    fn children(target: &str) -> Vec<Tip> {
        match target {
            "tab.home" => vec![tip("V", "paste"), tip("FP", "format-painter")],
            _ => vec![],
        }
    }

    fn shown() -> KeyTips {
        let mut k = KeyTips::default();
        k.alt_pressed(false);
        assert_eq!(k.alt_released(root()), TipOutcome::Changed);
        k
    }

    #[test]
    fn bare_alt_shows_the_tab_tips() {
        let k = shown();
        assert!(k.is_showing());
        assert_eq!(k.visible().count(), 4);
    }

    #[test]
    fn alt_with_another_key_is_a_modifier() {
        let mut k = KeyTips::default();
        k.alt_pressed(false);
        k.other_key_with_alt();
        assert_eq!(k.alt_released(root()), TipOutcome::None);
        assert!(!k.is_showing());
    }

    #[test]
    fn a_letter_descends_and_filters() {
        let mut k = shown();
        assert_eq!(
            k.letter('j', &children),
            TipOutcome::Changed,
            "J is a prefix of JI"
        );
        assert_eq!(k.typed(), "J");
        assert_eq!(k.visible().count(), 1);
        assert_eq!(
            k.letter('i', &children),
            TipOutcome::Activate("tab.draw".into())
        );
        assert!(!k.is_showing());
    }

    #[test]
    fn a_tab_tip_with_children_shows_the_next_level_then_activates() {
        let mut k = shown();
        assert_eq!(k.letter('H', &children), TipOutcome::Changed);
        assert_eq!(k.depth(), 1);
        let keys: Vec<_> = k.visible().map(|t| t.keys.clone()).collect();
        assert_eq!(keys, ["V", "FP"]);
        k.letter('F', &children);
        assert_eq!(
            k.letter('p', &children),
            TipOutcome::Activate("format-painter".into())
        );
    }

    #[test]
    fn escape_goes_up_one_level_then_leaves() {
        let mut k = shown();
        k.letter('H', &children);
        k.letter('F', &children);
        assert_eq!(k.escape(), TipOutcome::Changed, "clears the typed F");
        assert_eq!(k.typed(), "");
        assert_eq!(k.depth(), 1);
        assert_eq!(k.escape(), TipOutcome::Changed, "back to the tab tips");
        assert_eq!(k.depth(), 0);
        assert_eq!(k.visible().count(), 4);
        assert_eq!(k.escape(), TipOutcome::Changed, "leaves");
        assert!(!k.is_showing());
        assert_eq!(k.escape(), TipOutcome::None);
    }

    #[test]
    fn unknown_letters_are_ignored_and_alt_again_leaves() {
        let mut k = shown();
        assert_eq!(k.letter('Z', &children), TipOutcome::None);
        assert!(k.is_showing());
        assert_eq!(k.alt_pressed(false), TipOutcome::Changed);
        assert!(!k.is_showing());
    }

    #[test]
    fn auto_repeat_of_alt_does_nothing() {
        let mut k = KeyTips::default();
        k.alt_pressed(false);
        assert_eq!(k.alt_pressed(true), TipOutcome::None);
        assert_eq!(k.alt_released(root()), TipOutcome::Changed);
    }
}
