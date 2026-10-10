//! Main-part content types (Word and Excel, including templates and macro-enabled forms)
//! and byte-preserving removal of a content-type `Override`.

use quick_xml::XmlVersion;
use quick_xml::events::Event;
use quick_xml::reader::Reader;

pub(crate) const CONTENT_TYPES_PART: &str = "[Content_Types].xml";

/// Content type of a VBA macro project part (`vbaProject.bin`).
pub const VBA_PROJECT_CONTENT_TYPE: &str = "application/vnd.ms-office.vbaProject";
/// Content type of VBA project data (`vbaData.xml`).
pub const VBA_DATA_CONTENT_TYPE: &str = "application/vnd.ms-office.vbaData";

pub(crate) const OFFICE_DOCUMENT_RELS: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument",
];

/// True for the two VBA content types. Macros are carried byte-identical and never run.
pub fn is_vba_content_type(content_type: &str) -> bool {
    content_type.eq_ignore_ascii_case(VBA_PROJECT_CONTENT_TYPE)
        || content_type.eq_ignore_ascii_case(VBA_DATA_CONTENT_TYPE)
}

/// Every main-part type of a Word or Excel package (ECMA-376 Part 2, plus Microsoft's
/// macro-enabled types).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MainPartKind {
    /// `.docx`
    Docx,
    /// `.docm`
    Docm,
    /// `.dotx`
    Dotx,
    /// `.dotm`
    Dotm,
    /// `.xlsx`
    Xlsx,
    /// `.xlsm`
    Xlsm,
    /// `.xltx`
    Xltx,
    /// `.xltm`
    Xltm,
}

impl MainPartKind {
    pub const ALL: [Self; 8] = [
        Self::Docx,
        Self::Docm,
        Self::Dotx,
        Self::Dotm,
        Self::Xlsx,
        Self::Xlsm,
        Self::Xltx,
        Self::Xltm,
    ];

    /// The main-part content type.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Docx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
            }
            Self::Docm => "application/vnd.ms-word.document.macroEnabled.main+xml",
            Self::Dotx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml"
            }
            Self::Dotm => "application/vnd.ms-word.template.macroEnabledTemplate.main+xml",
            Self::Xlsx => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
            }
            Self::Xlsm => "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
            Self::Xltx => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml"
            }
            Self::Xltm => "application/vnd.ms-excel.template.macroEnabled.main+xml",
        }
    }

    /// The kind a main-part content type denotes (media types compare ASCII
    /// case-insensitively), or `None` for any other type.
    pub fn from_content_type(content_type: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.content_type().eq_ignore_ascii_case(content_type))
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Docm => "docm",
            Self::Dotx => "dotx",
            Self::Dotm => "dotm",
            Self::Xlsx => "xlsx",
            Self::Xlsm => "xlsm",
            Self::Xltx => "xltx",
            Self::Xltm => "xltm",
        }
    }

    pub fn is_word(self) -> bool {
        matches!(self, Self::Docx | Self::Docm | Self::Dotx | Self::Dotm)
    }

    pub fn is_spreadsheet(self) -> bool {
        !self.is_word()
    }

    pub fn is_template(self) -> bool {
        matches!(self, Self::Dotx | Self::Dotm | Self::Xltx | Self::Xltm)
    }

    pub fn is_macro_enabled(self) -> bool {
        matches!(self, Self::Docm | Self::Dotm | Self::Xlsm | Self::Xltm)
    }
}

/// Returns `[Content_Types].xml` with every `<Override>` for `part` cut out, or `None` when
/// there is none (or the XML is unreadable or has a DOCTYPE). Only the bytes of the cut
/// elements change.
pub(crate) fn without_override(xml: &[u8], part: &str) -> Option<Vec<u8>> {
    let want = format!("/{part}");
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    // (start offset, depth) of a matching non-empty <Override> still open.
    let mut open: Option<(usize, usize)> = None;
    let mut depth = 0usize;
    loop {
        let start = usize::try_from(reader.buffer_position()).ok()?;
        let event = reader.read_event_into(&mut buf).ok()?;
        let end = usize::try_from(reader.buffer_position()).ok()?;
        match &event {
            Event::DocType(_) => return None,
            Event::Start(e) | Event::Empty(e) => {
                let is_empty = matches!(event, Event::Empty(_));
                let matches = e.local_name().as_ref() == "Override"
                    && e.attributes().flatten().any(|a| {
                        a.key.local_name().as_ref() == "PartName"
                            && a.normalized_value(XmlVersion::Explicit1_0)
                                .is_ok_and(|v| v.eq_ignore_ascii_case(&want))
                    });
                if is_empty {
                    if matches && open.is_none() {
                        cuts.push((start, end));
                    }
                } else {
                    depth += 1;
                    if matches && open.is_none() {
                        open = Some((start, depth));
                    }
                }
            }
            Event::End(_) => {
                if let Some((from, at)) = open
                    && at == depth
                {
                    cuts.push((from, end));
                    open = None;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if cuts.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(xml.len());
    let mut at = 0;
    for (from, to) in cuts {
        out.extend_from_slice(&xml[at..from]);
        at = to;
    }
    out.extend_from_slice(&xml[at..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_content_type() {
        for kind in MainPartKind::ALL {
            assert_eq!(
                MainPartKind::from_content_type(kind.content_type()),
                Some(kind)
            );
            assert_eq!(
                MainPartKind::from_content_type(&kind.content_type().to_uppercase()),
                Some(kind)
            );
        }
        assert_eq!(MainPartKind::from_content_type("application/xml"), None);
        assert!(MainPartKind::Xltm.is_template() && MainPartKind::Xltm.is_macro_enabled());
        assert!(MainPartKind::Docx.is_word() && MainPartKind::Xlsx.is_spreadsheet());
        assert!(is_vba_content_type(VBA_PROJECT_CONTENT_TYPE));
        assert!(is_vba_content_type(VBA_DATA_CONTENT_TYPE));
        assert!(!is_vba_content_type("application/xml"));
    }

    #[test]
    fn cuts_only_the_matching_override() {
        let xml = br#"<Types xmlns="x"><Default Extension="xml" ContentType="a"/>
<Override PartName="/A.xml" ContentType="a"/>
<Override PartName="/b.xml" ContentType="b"></Override>
<Override PartName="/c.xml" ContentType="c"/></Types>"#;
        let cut = without_override(xml, "a.xml").unwrap();
        let text = String::from_utf8(cut).unwrap();
        assert!(!text.contains("A.xml") && text.contains("/b.xml") && text.contains("/c.xml"));
        let cut = without_override(xml, "b.xml").unwrap();
        let text = String::from_utf8(cut).unwrap();
        assert!(!text.contains("/b.xml") && text.contains("/A.xml") && text.contains("/c.xml"));
        assert!(without_override(xml, "zzz.xml").is_none());
        assert!(without_override(b"<!DOCTYPE x><Types/>", "a.xml").is_none());
    }
}
