//! The set of formats one copy puts on the clipboard, and the rules for choosing among them
//! when pasting. The operating-system glue (`open365-ui-host`) moves a [`ClipPayload`] to and
//! from the real clipboard; everything else is decided here, in pure code.

use std::cell::RefCell;
use std::rc::Rc;

use crate::cfhtml::{self, Flavor};
use crate::limits::{MAX_HTML_BYTES, MAX_RTF_BYTES, MAX_TEXT_BYTES};
use crate::model::RichText;
use crate::private::PrivateClip;
use crate::sheet::SheetClip;
use crate::{rtf, writer_html};

/// Registered name of the HTML container format.
pub const HTML_FORMAT_NAME: &str = "HTML Format";
/// Registered name of the RTF format.
pub const RTF_FORMAT_NAME: &str = "Rich Text Format";

/// Every format of one copy. `html` is the "HTML Format" container, `rtf` the RTF document and
/// `private` the [`PrivateClip`] bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipPayload {
    pub text: Option<String>,
    pub html: Option<Vec<u8>>,
    pub rtf: Option<Vec<u8>>,
    pub private: Option<Vec<u8>>,
}

impl ClipPayload {
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.html.is_none() && self.rtf.is_none() && self.private.is_none()
    }
}

/// Where payloads go. The operating system clipboard implements it in `open365-ui-host`;
/// [`MemoryPayload`] is the in-process one.
pub trait PayloadIo {
    fn set_payload(&mut self, payload: &ClipPayload) -> Result<(), String>;
    fn payload(&mut self) -> ClipPayload;
}

/// An in-process clipboard; clones share the contents.
#[derive(Debug, Clone, Default)]
pub struct MemoryPayload(pub Rc<RefCell<ClipPayload>>);

impl PayloadIo for MemoryPayload {
    fn set_payload(&mut self, payload: &ClipPayload) -> Result<(), String> {
        *self.0.borrow_mut() = payload.clone();
        Ok(())
    }

    fn payload(&mut self) -> ClipPayload {
        self.0.borrow().clone()
    }
}

/// Which format a paste was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Private,
    Html,
    Rtf,
    Text,
}

fn crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// All formats for a Writer selection.
pub fn writer_payload(rt: &RichText) -> ClipPayload {
    let html = cfhtml::wrap(&writer_html::to_html(rt), Flavor::Text);
    let rtf = rtf::to_rtf(rt).into_bytes();
    ClipPayload {
        text: Some(crlf(&rt.plain())),
        html: (html.len() <= MAX_HTML_BYTES).then_some(html),
        rtf: (rtf.len() <= MAX_RTF_BYTES).then_some(rtf),
        private: PrivateClip::Text(rt.clone()).encode(),
    }
}

/// All formats for a Sheets range: TSV, HTML table and the private format.
pub fn sheet_payload(clip: &SheetClip) -> ClipPayload {
    let html = cfhtml::wrap(&clip.to_html(), Flavor::Sheet);
    ClipPayload {
        text: Some(clip.to_tsv()),
        html: (html.len() <= MAX_HTML_BYTES).then_some(html),
        rtf: None,
        private: PrivateClip::Sheet(clip.clone()).encode(),
    }
}

fn text_of(p: &ClipPayload) -> Option<&str> {
    p.text.as_deref().filter(|t| t.len() <= MAX_TEXT_BYTES)
}

/// Chooses the best format for the Writer: private, HTML, RTF, then plain text. A format that
/// fails to parse is passed over for the next one.
pub fn read_writer(p: &ClipPayload) -> Option<(Source, RichText)> {
    if let Some(Ok(PrivateClip::Text(t))) = p.private.as_deref().map(PrivateClip::decode) {
        return Some((Source::Private, t));
    }
    if let Some(h) = p.html.as_deref()
        && let Ok(s) = cfhtml::unwrap(h)
        && let Ok(t) = writer_html::parse(&s)
        && !t.is_empty()
    {
        return Some((Source::Html, t));
    }
    if let Some(r) = p.rtf.as_deref()
        && let Ok(t) = rtf::parse(r)
        && !t.is_empty()
    {
        return Some((Source::Rtf, t));
    }
    let text = text_of(p)?;
    let t = RichText::from_plain(text);
    (!t.paras.is_empty()).then_some((Source::Text, t))
}

/// Chooses the best format for Sheets: private, an HTML table, then TSV text.
pub fn read_sheet(p: &ClipPayload) -> Option<(Source, SheetClip)> {
    if let Some(Ok(PrivateClip::Sheet(s))) = p.private.as_deref().map(PrivateClip::decode) {
        return Some((Source::Private, s));
    }
    if let Some(h) = p.html.as_deref()
        && let Ok(s) = cfhtml::unwrap(h)
        && let Ok(c) = SheetClip::from_html(&s)
        && !c.rows.is_empty()
    {
        return Some((Source::Html, c));
    }
    let c = SheetClip::from_tsv(text_of(p)?).ok()?;
    (!c.rows.is_empty()).then_some((Source::Text, c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Para, Run};
    use crate::sheet::{CellKind, ClipCell};

    fn rich() -> RichText {
        RichText {
            paras: vec![Para {
                runs: vec![Run {
                    text: "hi".into(),
                    fmt: crate::model::CharFmt {
                        bold: true,
                        ..Default::default()
                    },
                    link: None,
                }],
                ..Para::default()
            }],
        }
    }

    #[test]
    fn writer_picks_the_richest_format_available() {
        let p = writer_payload(&rich());
        assert_eq!(read_writer(&p).unwrap().0, Source::Private);
        let mut q = p.clone();
        q.private = None;
        let (s, t) = read_writer(&q).unwrap();
        assert_eq!((s, t), (Source::Html, rich()));
        q.html = None;
        let (s, t) = read_writer(&q).unwrap();
        assert_eq!(s, Source::Rtf);
        assert!(t.paras[0].runs[0].fmt.bold);
        q.rtf = None;
        assert_eq!(read_writer(&q).unwrap().0, Source::Text);
        assert_eq!(p.text.as_deref(), Some("hi"));
    }

    #[test]
    fn a_broken_format_falls_through() {
        let mut p = writer_payload(&rich());
        p.private = Some(b"garbage".to_vec());
        p.html = Some(b"<p>from html</p>".to_vec());
        assert_eq!(read_writer(&p).unwrap().1.plain(), "from html");
        p.html = Some(vec![b'a'; MAX_HTML_BYTES + 1]);
        p.rtf = Some(b"not rtf".to_vec());
        assert_eq!(read_writer(&p).unwrap().0, Source::Text);
    }

    #[test]
    fn sheet_formats() {
        let mut c = ClipCell::plain("");
        c.text = "1.50".into();
        c.entry = "1.5".into();
        c.kind = CellKind::Number(1.5);
        c.number_format = Some("0.00".into());
        let clip = SheetClip {
            origin: Some((2, 3)),
            rows: vec![vec![c, ClipCell::plain("x")]],
        };
        let p = sheet_payload(&clip);
        assert_eq!(p.text.as_deref(), Some("1.50\tx\r\n"));
        assert_eq!(read_sheet(&p).unwrap(), (Source::Private, clip.clone()));
        let mut q = p.clone();
        q.private = None;
        let (s, got) = read_sheet(&q).unwrap();
        assert_eq!(s, Source::Html);
        assert_eq!(got.rows[0][0].number_format.as_deref(), Some("0.00"));
        assert_eq!(got.rows[0][0].kind, CellKind::Number(1.5));
        q.html = None;
        let (s, got) = read_sheet(&q).unwrap();
        assert_eq!(s, Source::Text);
        assert_eq!(got.rows[0][1].entry, "x");
        assert!(read_sheet(&ClipPayload::default()).is_none());
    }

    #[test]
    fn html_without_a_table_falls_back_to_text_in_sheets() {
        let p = ClipPayload {
            text: Some("a\tb\r\n".into()),
            html: Some(b"<p>a b</p>".to_vec()),
            ..ClipPayload::default()
        };
        assert_eq!(read_sheet(&p).unwrap().0, Source::Text);
    }
}
