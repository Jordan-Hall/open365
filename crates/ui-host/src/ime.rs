//! Input method (IME) plumbing: winit's IME events become calls on [`ImeHandler`], and the
//! handler tells the host where the caret is so the candidate window follows it.

use winit::event::Ime;

/// Where the text caret is, in logical pixels relative to the window's client area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImeCursorArea {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// An IME event independent of winit's types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeEvent {
    /// The IME became active for this window.
    Enabled,
    /// Composition text. `cursor` is the selected range in bytes within `text`, if any.
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// Final text to insert; any preedit is finished.
    Commit(String),
    /// The IME stopped; clear any preedit.
    Disabled,
}

/// Converts winit's IME event.
pub fn translate(event: &Ime) -> ImeEvent {
    match event {
        Ime::Enabled => ImeEvent::Enabled,
        Ime::Preedit(text, cursor) => ImeEvent::Preedit {
            text: text.clone(),
            cursor: *cursor,
        },
        Ime::Commit(text) => ImeEvent::Commit(text.clone()),
        Ime::Disabled => ImeEvent::Disabled,
    }
}

/// What an editing surface (document canvas, grid cell editor, formula bar) implements to
/// receive composition. The host calls it on the UI thread.
pub trait ImeHandler {
    /// True while a text field has focus, so the host enables the OS input method.
    fn wants_ime(&self) -> bool;
    /// The caret rectangle, polled after every frame. `None` keeps the last position.
    fn cursor_area(&self) -> Option<ImeCursorArea>;
    fn ime_enabled(&mut self) {}
    /// Replace the current composition string. An empty `text` clears it.
    fn preedit(&mut self, text: &str, cursor: Option<(usize, usize)>);
    /// Insert final text and clear the composition.
    fn commit(&mut self, text: &str);
    fn ime_disabled(&mut self) {}
}

/// Delivers `event` to `handler`.
pub fn dispatch(handler: &mut dyn ImeHandler, event: &ImeEvent) {
    match event {
        ImeEvent::Enabled => handler.ime_enabled(),
        ImeEvent::Preedit { text, cursor } => handler.preedit(text, *cursor),
        ImeEvent::Commit(text) => handler.commit(text),
        ImeEvent::Disabled => handler.ime_disabled(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        log: Vec<String>,
    }

    impl ImeHandler for Recorder {
        fn wants_ime(&self) -> bool {
            true
        }
        fn cursor_area(&self) -> Option<ImeCursorArea> {
            None
        }
        fn ime_enabled(&mut self) {
            self.log.push("enabled".into());
        }
        fn preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) {
            self.log.push(format!("preedit {text} {cursor:?}"));
        }
        fn commit(&mut self, text: &str) {
            self.log.push(format!("commit {text}"));
        }
        fn ime_disabled(&mut self) {
            self.log.push("disabled".into());
        }
    }

    #[test]
    fn winit_events_translate_one_to_one() {
        assert_eq!(translate(&Ime::Enabled), ImeEvent::Enabled);
        assert_eq!(
            translate(&Ime::Preedit("ni".into(), Some((0, 2)))),
            ImeEvent::Preedit {
                text: "ni".into(),
                cursor: Some((0, 2))
            }
        );
        assert_eq!(
            translate(&Ime::Commit("你".into())),
            ImeEvent::Commit("你".into())
        );
        assert_eq!(translate(&Ime::Disabled), ImeEvent::Disabled);
    }

    #[test]
    fn a_composition_session_reaches_the_handler_in_order() {
        let mut h = Recorder::default();
        for e in [
            Ime::Enabled,
            Ime::Preedit("n".into(), Some((1, 1))),
            Ime::Preedit("ni".into(), Some((2, 2))),
            Ime::Commit("你".into()),
            Ime::Disabled,
        ] {
            dispatch(&mut h, &translate(&e));
        }
        assert_eq!(
            h.log,
            [
                "enabled",
                "preedit n Some((1, 1))",
                "preedit ni Some((2, 2))",
                "commit 你",
                "disabled"
            ]
        );
    }
}
