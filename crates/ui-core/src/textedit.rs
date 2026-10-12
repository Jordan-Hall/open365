//! Single-line text editing state: text, caret, selection and IME composition. Offsets are byte
//! offsets on character boundaries.

use std::time::{Duration, Instant};

use crate::input::Key;

/// The IME composition shown inline at the caret.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Composition {
    pub text: String,
    /// Cursor (start, end) inside `text`, when the IME reports one.
    pub cursor: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextEdit {
    pub text: String,
    /// Caret position.
    pub caret: usize,
    /// The other end of the selection (equal to `caret` when nothing is selected).
    pub anchor: usize,
    pub composition: Option<Composition>,
    /// When the caret last moved or text changed; blinking restarts from "on".
    pub last_activity: Option<Instant>,
    /// A password field: it paints a bullet per character; its text is never given to
    /// accessibility, the clipboard or the `TextChanged` and `TextCommitted` events.
    pub masked: bool,
}

/// What a masked field paints for each character.
pub const MASK_CHAR: char = '\u{2022}';

impl TextEdit {
    /// A masked (password) field holding `text`.
    pub fn masked(text: &str) -> Self {
        Self {
            masked: true,
            ..Self::new(text)
        }
    }

    /// What the field shows: itself, or for a masked field a copy holding a bullet for each
    /// character, with the caret and selection moved to match.
    pub fn display(&self) -> std::borrow::Cow<'_, Self> {
        if !self.masked {
            return std::borrow::Cow::Borrowed(self);
        }
        let to_shown =
            |i: usize| self.text[..i.min(self.text.len())].chars().count() * MASK_CHAR.len_utf8();
        let shown: String = self.text.chars().map(|_| MASK_CHAR).collect();
        std::borrow::Cow::Owned(Self {
            caret: to_shown(self.caret),
            anchor: to_shown(self.anchor),
            text: shown,
            composition: self.composition.as_ref().map(|c| Composition {
                text: c.text.chars().map(|_| MASK_CHAR).collect(),
                cursor: None,
            }),
            ..self.clone()
        })
    }

    /// The byte offset in the text of a masked field that corresponds to byte `shown` of its
    /// bullets; `shown` itself for a plain field.
    pub fn real_byte(&self, shown: usize) -> usize {
        if !self.masked {
            return shown;
        }
        self.text
            .char_indices()
            .nth(shown / MASK_CHAR.len_utf8())
            .map_or(self.text.len(), |(i, _)| i)
    }

    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            caret: text.len(),
            anchor: text.len(),
            ..Self::default()
        }
    }

    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn selected_text(&self) -> &str {
        let (a, b) = self.selection();
        &self.text[a..b]
    }

    fn prev(&self, i: usize) -> usize {
        self.text[..i]
            .char_indices()
            .next_back()
            .map_or(0, |(p, _)| p)
    }

    fn next(&self, i: usize) -> usize {
        self.text[i..]
            .chars()
            .next()
            .map_or(i, |c| i + c.len_utf8())
    }

    fn word_prev(&self, i: usize) -> usize {
        let t = &self.text[..i];
        let t = t.trim_end();
        t.rfind(char::is_whitespace).map_or(0, |p| p + 1)
    }

    fn word_next(&self, i: usize) -> usize {
        let rest = &self.text[i..];
        let after_word = rest.find(char::is_whitespace).map_or(rest.len(), |p| p);
        let skip = rest[after_word..]
            .find(|c: char| !c.is_whitespace())
            .map_or(rest.len() - after_word, |p| p);
        i + after_word + skip
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    pub fn set_caret(&mut self, i: usize, extend: bool) {
        self.caret = i.min(self.text.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// Replaces the selection by `s` (an empty `s` deletes it).
    pub fn insert(&mut self, s: &str) {
        let (a, b) = self.selection();
        self.text.replace_range(a..b, s);
        self.caret = a + s.len();
        self.anchor = self.caret;
    }

    pub fn delete_selection(&mut self) -> bool {
        if self.caret == self.anchor {
            return false;
        }
        self.insert("");
        true
    }

    /// Applies a navigation or deletion key. Returns true when the text changed.
    pub fn key(&mut self, key: Key, shift: bool, ctrl: bool) -> bool {
        match key {
            Key::Left => {
                let to = if !shift && self.caret != self.anchor {
                    self.selection().0
                } else if ctrl {
                    self.word_prev(self.caret)
                } else {
                    self.prev(self.caret)
                };
                self.set_caret(to, shift);
                false
            }
            Key::Right => {
                let to = if !shift && self.caret != self.anchor {
                    self.selection().1
                } else if ctrl {
                    self.word_next(self.caret)
                } else {
                    self.next(self.caret)
                };
                self.set_caret(to, shift);
                false
            }
            Key::Home => {
                self.set_caret(0, shift);
                false
            }
            Key::End => {
                self.set_caret(self.text.len(), shift);
                false
            }
            Key::Backspace => {
                if self.delete_selection() {
                    return true;
                }
                if self.caret == 0 {
                    return false;
                }
                self.anchor = if ctrl {
                    self.word_prev(self.caret)
                } else {
                    self.prev(self.caret)
                };
                self.delete_selection()
            }
            Key::Delete => {
                if self.delete_selection() {
                    return true;
                }
                if self.caret >= self.text.len() {
                    return false;
                }
                self.anchor = if ctrl {
                    self.word_next(self.caret)
                } else {
                    self.next(self.caret)
                };
                self.delete_selection()
            }
            Key::Char('A') if ctrl => {
                self.select_all();
                false
            }
            _ => false,
        }
    }

    /// Whether the caret is drawn at `now`: solid for one blink interval after activity, then
    /// alternating; always on when the OS asks for a steady caret.
    pub fn caret_visible(&self, now: Instant, blink: Option<Duration>) -> bool {
        match self.last_activity {
            Some(t0) => blink_on(t0, now, blink),
            None => true,
        }
    }
}

/// Whether a caret whose blink cycle restarted at `epoch` is shown at `now`: on for one `blink`
/// interval, then off for one. A steady caret (`None` or zero) is always shown.
#[must_use]
pub fn blink_on(epoch: Instant, now: Instant, blink: Option<Duration>) -> bool {
    match blink.filter(|b| !b.is_zero()) {
        Some(b) => {
            (now.saturating_duration_since(epoch).as_nanos() / b.as_nanos()).is_multiple_of(2)
        }
        None => true,
    }
}

/// The first blink edge (the caret turns on or off) after `now`, for a cycle restarted at `epoch`.
#[must_use]
pub fn next_blink_edge(epoch: Instant, now: Instant, blink: Duration) -> Instant {
    let b = blink.as_nanos().max(1);
    let n = now.saturating_duration_since(epoch).as_nanos() / b + 1;
    epoch + Duration::from_nanos(u64::try_from(n * b).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_replaces_the_selection() {
        let mut e = TextEdit::new("hello");
        e.set_caret(1, false);
        e.set_caret(4, true);
        assert_eq!(e.selected_text(), "ell");
        e.insert("a");
        assert_eq!(e.text, "hao");
        assert_eq!(e.caret, 2);
    }

    #[test]
    fn arrows_collapse_a_selection_to_its_edge() {
        let mut e = TextEdit::new("hello");
        e.select_all();
        e.key(Key::Left, false, false);
        assert_eq!((e.caret, e.anchor), (0, 0));
        e.select_all();
        e.key(Key::Right, false, false);
        assert_eq!((e.caret, e.anchor), (5, 5));
    }

    #[test]
    fn backspace_and_delete_respect_utf8_boundaries() {
        let mut e = TextEdit::new("a\u{e9}\u{4e2d}b");
        e.set_caret(e.text.len() - 1, false);
        assert!(e.key(Key::Backspace, false, false));
        assert_eq!(e.text, "a\u{e9}b");
        e.set_caret(1, false);
        assert!(e.key(Key::Delete, false, false));
        assert_eq!(e.text, "ab");
        assert!(!e.key(Key::Delete, false, false) || e.text == "a");
    }

    #[test]
    fn ctrl_arrows_and_backspace_move_by_words() {
        let mut e = TextEdit::new("one two  three");
        e.key(Key::Left, false, true);
        assert_eq!(e.caret, 9);
        e.key(Key::Left, false, true);
        assert_eq!(e.caret, 4);
        e.key(Key::Backspace, false, true);
        assert_eq!(e.text, "two  three");
    }

    #[test]
    fn shift_extends_and_ctrl_a_selects_all() {
        let mut e = TextEdit::new("abc");
        e.set_caret(0, false);
        e.key(Key::Right, true, false);
        e.key(Key::Right, true, false);
        assert_eq!(e.selected_text(), "ab");
        e.key(Key::Char('A'), false, true);
        assert_eq!(e.selected_text(), "abc");
    }

    #[test]
    fn caret_blinks_from_on_after_activity() {
        let t0 = Instant::now();
        let mut e = TextEdit::new("x");
        e.last_activity = Some(t0);
        let b = Some(Duration::from_millis(500));
        assert!(e.caret_visible(t0, b));
        assert!(e.caret_visible(t0 + Duration::from_millis(499), b));
        assert!(!e.caret_visible(t0 + Duration::from_millis(500), b));
        assert!(e.caret_visible(t0 + Duration::from_millis(1000), b));
        assert!(
            e.caret_visible(t0 + Duration::from_millis(500), None),
            "steady caret"
        );
    }
}
