//! The clipboard seam. The host supplies a platform implementation; tests use [`MemoryClipboard`].

/// Text clipboard access. Rich formats arrive with the platform crate (see the ADR).
pub trait Clipboard {
    fn get_text(&mut self) -> Option<String>;
    fn set_text(&mut self, text: &str);
}

/// An in-process clipboard.
#[derive(Debug, Default)]
pub struct MemoryClipboard {
    text: Option<String>,
}

impl Clipboard for MemoryClipboard {
    fn get_text(&mut self) -> Option<String> {
        self.text.clone()
    }

    fn set_text(&mut self, text: &str) {
        self.text = Some(text.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_clipboard_round_trips() {
        let mut c = MemoryClipboard::default();
        assert_eq!(c.get_text(), None);
        c.set_text("abc");
        assert_eq!(c.get_text().as_deref(), Some("abc"));
    }
}
