//! The system clipboard for the widgets, through [`RichClipboard`], the only clipboard backend
//! (`clipboard-win` on Windows, `arboard` elsewhere; both safe public APIs).

use open365_clipboard::{ClipPayload, PayloadIo};
use open365_ui_core::Clipboard;

#[path = "clipboard_rich.rs"]
pub mod rich;
pub use rich::RichClipboard;

/// Text clipboard backed by the operating system. It is the text face of [`RichClipboard`], so
/// the whole application talks to one backend per platform (`clipboard-win` on Windows, `arboard`
/// elsewhere) and never to two at once. If the system clipboard cannot be opened (headless
/// session, locked by another process) it degrades to an in-process buffer so copy and paste
/// inside the application keep working.
pub struct SystemClipboard {
    rich: RichClipboard,
}

impl std::fmt::Debug for SystemClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemClipboard")
            .field("system", &self.rich.is_system())
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
            rich: RichClipboard::new(),
        }
    }

    /// True when the operating system clipboard is in use.
    pub fn is_system(&self) -> bool {
        self.rich.is_system()
    }
}

impl Clipboard for SystemClipboard {
    fn get_text(&mut self) -> Option<String> {
        self.rich.payload().text
    }

    fn set_text(&mut self, text: &str) {
        let payload = ClipPayload {
            text: Some(text.to_owned()),
            ..ClipPayload::default()
        };
        // The buffer keeps the copy when the system refuses it, so this cannot fail.
        let _ = self.rich.set_payload(&payload);
    }
}

#[cfg(test)]
/// The OS clipboard is one global resource: tests that touch it take turns.
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes and reads back through the real clipboard when the session has one. The
    /// clipboard is a shared resource: the previous text is restored afterwards.
    #[test]
    fn text_round_trips_through_the_clipboard() {
        let _turn = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            rich: RichClipboard::in_memory(),
        };
        assert!(!c.is_system());
        assert_eq!(c.get_text(), None);
        c.set_text("abc");
        assert_eq!(c.get_text().as_deref(), Some("abc"));
    }
}
