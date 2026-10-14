//! The system clipboard with several formats at once: plain text, "HTML Format", "Rich Text
//! Format" and the private Open365 format, so a copy keeps its formatting in Office and in
//! other Open365 windows.
//!
//! * Windows: `clipboard-win` (BSL-1.0, safe API) writes every format in one open/empty/set
//!   sequence and reads them back with size limits applied before the bytes are copied.
//! * Other systems: `arboard` carries text and HTML. RTF and the private format stay in the
//!   process, and only while the system text still equals what this process copied, so a copy
//!   made in another program is never answered with stale private data.
//!
//! What is read is untrusted; `open365-clipboard` parses and bounds it. Nothing here follows a
//! URL from the clipboard.

use open365_clipboard::limits::{MAX_HTML_BYTES, MAX_PRIVATE_BYTES, MAX_RTF_BYTES, MAX_TEXT_BYTES};
use open365_clipboard::{ClipPayload, PayloadIo};

/// Multi-format clipboard. If the system clipboard cannot be opened (headless session, locked
/// by another process) it degrades to an in-process buffer so copy and paste inside the
/// application keep working.
pub struct RichClipboard {
    system: bool,
    /// The last copy could not be written to the system (clipboard locked): serve the buffer.
    unsent: bool,
    #[cfg(windows)]
    ids: Option<WinIds>,
    #[cfg(not(windows))]
    arboard: Option<arboard::Clipboard>,
    memory: ClipPayload,
}

impl std::fmt::Debug for RichClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RichClipboard")
            .field("system", &self.system)
            .finish()
    }
}

impl Default for RichClipboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct WinIds {
    html: u32,
    rtf: u32,
    private: u32,
}

impl RichClipboard {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let ids = (|| {
                Some(WinIds {
                    html: clipboard_win::raw::register_format(open365_clipboard::HTML_FORMAT_NAME)?
                        .get(),
                    rtf: clipboard_win::raw::register_format(open365_clipboard::RTF_FORMAT_NAME)?
                        .get(),
                    private: clipboard_win::raw::register_format(
                        open365_clipboard::PRIVATE_FORMAT_NAME,
                    )?
                    .get(),
                })
            })();
            Self {
                system: ids.is_some(),
                unsent: false,
                ids,
                memory: ClipPayload::default(),
            }
        }
        #[cfg(not(windows))]
        {
            let arboard = arboard::Clipboard::new().ok();
            Self {
                system: arboard.is_some(),
                unsent: false,
                arboard,
                memory: ClipPayload::default(),
            }
        }
    }

    /// A clipboard that never touches the system (tests, headless runs).
    pub fn in_memory() -> Self {
        #[cfg(windows)]
        {
            Self {
                system: false,
                unsent: false,
                ids: None,
                memory: ClipPayload::default(),
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                system: false,
                unsent: false,
                arboard: None,
                memory: ClipPayload::default(),
            }
        }
    }

    /// True when the operating system clipboard is in use.
    pub fn is_system(&self) -> bool {
        self.system
    }
}

#[cfg(windows)]
impl RichClipboard {
    fn write_system(&mut self, p: &ClipPayload) -> Result<(), String> {
        use clipboard_win::{Clipboard, options::NoClear, raw};
        let ids = self.ids.ok_or("clipboard formats are not registered")?;
        let _open = Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
        raw::empty().map_err(|e| e.to_string())?;
        if let Some(t) = &p.text {
            raw::set_string_with(t, NoClear).map_err(|e| e.to_string())?;
        }
        // A format another program cannot read must not fail the copy: text is already set.
        if let Some(h) = &p.html {
            let _ = raw::set_without_clear(ids.html, h);
        }
        if let Some(r) = &p.rtf {
            let _ = raw::set_without_clear(ids.rtf, r);
        }
        if let Some(v) = &p.private {
            let _ = raw::set_without_clear(ids.private, v);
        }
        Ok(())
    }

    fn read_system(&mut self) -> Option<ClipPayload> {
        use clipboard_win::{Clipboard, formats, raw};
        let ids = self.ids?;
        let _open = Clipboard::new_attempts(10).ok()?;
        let mut out = ClipPayload::default();
        if raw::is_format_avail(formats::CF_UNICODETEXT)
            && raw::size(formats::CF_UNICODETEXT).is_some_and(|n| n.get() <= MAX_TEXT_BYTES * 2)
        {
            let mut buf = Vec::new();
            if raw::get_string(&mut buf).is_ok() {
                out.text = String::from_utf8(buf).ok();
            }
        }
        let raw_format = |id: u32, limit: usize| -> Option<Vec<u8>> {
            if !raw::is_format_avail(id) || raw::size(id).is_none_or(|n| n.get() > limit) {
                return None;
            }
            let mut v = Vec::new();
            raw::get_vec(id, &mut v).ok()?;
            Some(v)
        };
        out.html = raw_format(ids.html, MAX_HTML_BYTES);
        out.rtf = raw_format(ids.rtf, MAX_RTF_BYTES);
        out.private = raw_format(ids.private, MAX_PRIVATE_BYTES);
        Some(out)
    }
}

#[cfg(not(windows))]
impl RichClipboard {
    fn write_system(&mut self, p: &ClipPayload) -> Result<(), String> {
        let cb = self.arboard.as_mut().ok_or("no system clipboard")?;
        let text = p.text.clone().unwrap_or_default();
        match &p.html {
            Some(h) => {
                let html = open365_clipboard::cfhtml::unwrap(h).map_err(|e| e.to_string())?;
                cb.set().html(html, Some(text)).map_err(|e| e.to_string())
            }
            None => cb.set_text(text).map_err(|e| e.to_string()),
        }
    }

    fn read_system(&mut self) -> Option<ClipPayload> {
        let cb = self.arboard.as_mut()?;
        let mut out = ClipPayload {
            text: cb.get_text().ok().filter(|t| t.len() <= MAX_TEXT_BYTES),
            ..ClipPayload::default()
        };
        if let Ok(h) = cb.get().html()
            && h.len() <= MAX_HTML_BYTES
        {
            out.html = Some(open365_clipboard::cfhtml::wrap(
                &h,
                open365_clipboard::cfhtml::Flavor::Text,
            ));
        }
        // The formats the system cannot carry come from this process, while its text is current.
        if self.memory.text.is_some() && self.memory.text == out.text {
            out.rtf = out.rtf.or_else(|| self.memory.rtf.clone());
            out.private = self.memory.private.clone();
        }
        Some(out)
    }
}

impl PayloadIo for RichClipboard {
    fn set_payload(&mut self, payload: &ClipPayload) -> Result<(), String> {
        self.memory = payload.clone();
        // A locked clipboard keeps the copy in the buffer: copy and paste still work inside the
        // application, and the next copy tries the system again.
        if self.system {
            self.unsent = self.write_system(payload).is_err();
        }
        Ok(())
    }

    fn payload(&mut self) -> ClipPayload {
        if self.system
            && !self.unsent
            && let Some(p) = self.read_system()
        {
            return p;
        }
        self.memory.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use open365_clipboard::{
        CellKind, ClipCell, SheetClip, Source, read_sheet, read_writer, sheet_payload,
        writer_payload,
    };
    use open365_clipboard::{CharFmt, Para, RichText, Run};

    fn rich() -> RichText {
        RichText {
            paras: vec![Para {
                runs: vec![Run {
                    text: "Hello \u{e9}\u{4e2d}".into(),
                    fmt: CharFmt {
                        bold: true,
                        font: Some("Arial".into()),
                        ..CharFmt::default()
                    },
                    link: None,
                }],
                ..Para::default()
            }],
        }
    }

    #[test]
    fn memory_clipboard_keeps_every_format() {
        let mut c = RichClipboard::in_memory();
        assert!(!c.is_system());
        assert!(c.payload().is_empty());
        let p = writer_payload(&rich());
        c.set_payload(&p).unwrap();
        assert_eq!(c.payload(), p);
        assert_eq!(read_writer(&c.payload()).unwrap().0, Source::Private);
    }

    /// Writes and reads back through the real clipboard when the session has one. The clipboard
    /// is a shared resource: the previous text is restored afterwards.
    #[cfg(windows)]
    #[test]
    fn formats_round_trip_through_the_system_clipboard() {
        let _turn = crate::clipboard::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut c = RichClipboard::new();
        if !c.is_system() {
            return;
        }
        let saved = c.payload().text;

        // Writer: all four formats arrive, and the best one is the private format.
        let p = writer_payload(&rich());
        c.set_payload(&p).unwrap();
        let got = c.payload();
        assert_eq!(got.text, p.text);
        assert_eq!(got.html, p.html);
        assert_eq!(got.rtf, p.rtf);
        assert_eq!(got.private, p.private);

        // Without the private format the HTML keeps the formatting.
        let mut no_private = got.clone();
        no_private.private = None;
        let (src, rt) = read_writer(&no_private).unwrap();
        assert_eq!(src, Source::Html);
        assert!(rt.paras[0].runs[0].fmt.bold);
        assert_eq!(rt.paras[0].runs[0].fmt.font.as_deref(), Some("Arial"));

        // Sheets: TSV, HTML and private.
        let mut cell = ClipCell::plain("");
        cell.text = "1.50".into();
        cell.entry = "1.5".into();
        cell.kind = CellKind::Number(1.5);
        cell.number_format = Some("0.00".into());
        let clip = SheetClip {
            origin: Some((1, 1)),
            rows: vec![vec![cell, ClipCell::plain("x")]],
        };
        c.set_payload(&sheet_payload(&clip)).unwrap();
        let got = c.payload();
        assert_eq!(got.text.as_deref(), Some("1.50\tx\r\n"));
        assert_eq!(read_sheet(&got).unwrap(), (Source::Private, clip));

        if let Some(s) = saved {
            c.set_payload(&ClipPayload {
                text: Some(s),
                ..ClipPayload::default()
            })
            .unwrap();
        }
    }
}
