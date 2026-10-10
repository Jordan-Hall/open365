//! Keyboard shortcuts of the document view, separated from what they do so the mapping can be
//! tested without a window.

use egui::{Key, Modifiers};

/// Where a navigation key moves the caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// What a key press asks of the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Move the caret; `select` (Shift) extends the selection and `big` (Ctrl or Cmd) moves by
    /// word, or to the document start or end for Home and End.
    Move {
        nav: Nav,
        select: bool,
        big: bool,
    },
    SelectAll,
    Backspace {
        word: bool,
    },
    Delete {
        word: bool,
    },
    /// Enter starts a paragraph, Shift+Enter a line break.
    Enter {
        soft: bool,
    },
}

/// Maps a key press to an action, `None` for keys the view leaves to other handlers.
pub fn classify(key: Key, mods: Modifiers) -> Option<KeyAction> {
    let nav = match key {
        Key::ArrowLeft => Nav::Left,
        Key::ArrowRight => Nav::Right,
        Key::ArrowUp => Nav::Up,
        Key::ArrowDown => Nav::Down,
        Key::Home => Nav::Home,
        Key::End => Nav::End,
        Key::PageUp => Nav::PageUp,
        Key::PageDown => Nav::PageDown,
        Key::A if mods.command => return Some(KeyAction::SelectAll),
        Key::Backspace => {
            return Some(KeyAction::Backspace { word: mods.command });
        }
        Key::Delete => return Some(KeyAction::Delete { word: mods.command }),
        Key::Enter => return Some(KeyAction::Enter { soft: mods.shift }),
        _ => return None,
    };
    Some(KeyAction::Move {
        nav,
        select: mods.shift,
        big: mods.command,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_keys_carry_shift_and_command() {
        assert_eq!(
            classify(Key::PageDown, Modifiers::SHIFT),
            Some(KeyAction::Move {
                nav: Nav::PageDown,
                select: true,
                big: false
            })
        );
        assert_eq!(
            classify(Key::ArrowLeft, Modifiers::COMMAND),
            Some(KeyAction::Move {
                nav: Nav::Left,
                select: false,
                big: true
            })
        );
    }

    #[test]
    fn editing_keys_map_and_unknown_keys_do_not() {
        assert_eq!(
            classify(Key::Enter, Modifiers::SHIFT),
            Some(KeyAction::Enter { soft: true })
        );
        assert_eq!(
            classify(Key::Backspace, Modifiers::COMMAND),
            Some(KeyAction::Backspace { word: true })
        );
        assert_eq!(classify(Key::A, Modifiers::NONE), None);
        assert_eq!(
            classify(Key::A, Modifiers::COMMAND),
            Some(KeyAction::SelectAll)
        );
        assert_eq!(classify(Key::F5, Modifiers::NONE), None);
    }
}
