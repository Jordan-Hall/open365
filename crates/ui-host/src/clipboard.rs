//! The system clipboard for the widgets, through the `arboard` crate (MIT OR Apache-2.0, safe
//! public API, text only). Rich formats arrive with the platform crate (see the ADR).

use open365_ui_core::{Clipboard, MemoryClipboard};

/// Text clipboard backed by the operating system. If the system clipboard cannot be opened
/// (headless session, locked by another process) it degrades to an in-process buffer so copy and
/// paste inside the application keep working.
pub struct SystemClipboard {
    system: Option<arboard::Clipboard>,
    fallback: MemoryClipboard,
}

impl std::fmt::Debug for SystemClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemClipboard")
            .field("system", &self.system.is_some())
            .finish()
    }
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemClipboard {
    pub fn new() -> Self {
        Self {
            system: arboard::Clipboard::new().ok(),
            fallback: MemoryClipboard::default(),
        }
    }

    /// True when the operating system clipboard is in use.
    pub fn is_system(&self) -> bool {
        self.system.is_some()
    }
}

impl Clipboard for SystemClipboard {
    fn get_text(&mut self) -> Option<String> {
        if let Some(c) = self.system.as_mut() {
            match c.get_text() {
                Ok(t) => return Some(t),
                // Empty or non-text contents: nothing to paste.
                Err(arboard::Error::ContentNotAvailable) => return None,
                Err(_) => {}
            }
        }
        self.fallback.get_text()
    }

    fn set_text(&mut self, text: &str) {
        if let Some(c) = self.system.as_mut()
            && c.set_text(text.to_owned()).is_ok()
        {
            return;
        }
        self.fallback.set_text(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes and reads back through the real clipboard when the session has one. The
    /// clipboard is a shared resource: the previous text is restored afterwards.
    #[test]
    fn text_round_trips_through_the_clipboard() {
        let mut c = SystemClipboard::new();
        let saved = c.get_text();
        let probe = "open365 clipboard probe \u{00e9}\u{4e2d}";
        c.set_text(probe);
        let got = c.get_text();
        match saved {
            Some(s) => c.set_text(&s),
            None if c.is_system() => {}
            None => {}
        }
        assert_eq!(got.as_deref(), Some(probe));
    }

    #[test]
    fn without_a_system_clipboard_the_buffer_is_used() {
        let mut c = SystemClipboard {
            system: None,
            fallback: MemoryClipboard::default(),
        };
        assert!(!c.is_system());
        assert_eq!(c.get_text(), None);
        c.set_text("abc");
        assert_eq!(c.get_text().as_deref(), Some("abc"));
    }
}
