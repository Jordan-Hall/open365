//! Document information: the Info dictionary and the XMP packet, which must say the same thing
//! (PDF/A checks it).

use open365_docx::Document;

use crate::Conformance;
use crate::writer::{Pdf, text_string, xml_escape};

/// Document information supplied by the caller. Nothing is filled in by default: no author, no
/// dates, so an export does not leak the machine or the user. [`PdfMetadata::from_document`]
/// takes what the document itself carries, as Word's "Document properties" option does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdfMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    /// The producing application; defaults to `Open365`.
    pub creator: Option<String>,
    /// A PDF date string such as `D:20260101120000Z`, written as is.
    pub created: Option<String>,
    /// A PDF date string for the last modification (`/ModDate`).
    pub modified: Option<String>,
    /// BCP 47 language tag of the document (`/Lang`).
    pub language: Option<String>,
}

impl PdfMetadata {
    /// The title, author, subject, keywords and dates of the document's core properties
    /// (`docProps/core.xml`); a field the document does not have stays `None`.
    pub fn from_document(doc: &Document) -> Self {
        let Some(part) = doc.package().part("docProps/core.xml") else {
            return Self::default();
        };
        let xml = String::from_utf8_lossy(part.data());
        let date = |tag: &str| element_text(&xml, tag).and_then(|d| iso_to_pdf_date(&d));
        Self {
            title: element_text(&xml, "dc:title"),
            author: element_text(&xml, "dc:creator"),
            subject: element_text(&xml, "dc:subject"),
            keywords: element_text(&xml, "cp:keywords"),
            created: date("dcterms:created"),
            modified: date("dcterms:modified"),
            ..Self::default()
        }
    }
}

/// The text of the first `<tag ...>text</tag>`, entities decoded; `None` when empty or absent.
fn element_text(xml: &str, tag: &str) -> Option<String> {
    let open = xml.find(&format!("<{tag}"))?;
    let rest = &xml[open..];
    let gt = rest.find('>')?;
    if rest[..gt].ends_with('/') {
        return None;
    }
    let body = &rest[gt + 1..];
    let end = body.find(&format!("</{tag}>"))?;
    let text = body[..end]
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// `2026-01-01T12:00:00Z` as `D:20260101120000Z`.
fn iso_to_pdf_date(iso: &str) -> Option<String> {
    let digits: String = iso.chars().filter(char::is_ascii_digit).take(14).collect();
    (digits.len() == 14 && iso.ends_with('Z')).then(|| format!("D:{digits}Z"))
}

/// `D:20260101120000Z` as `2026-01-01T12:00:00Z` (`+01'00'` becomes `+01:00`); `None` for a
/// string that is not a PDF date.
pub fn pdf_date_to_iso(d: &str) -> Option<String> {
    let s = d.strip_prefix("D:")?;
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<&str> {
        let part = s.get(from..from + len)?;
        part.bytes().all(|c| c.is_ascii_digit()).then_some(part)
    };
    let year = num(0, 4)?;
    let month = num(4, 2).unwrap_or("01");
    let day = num(6, 2).unwrap_or("01");
    let hour = num(8, 2).unwrap_or("00");
    let min = num(10, 2).unwrap_or("00");
    let sec = num(12, 2).unwrap_or("00");
    let tz = match b.get(14) {
        None | Some(b'Z') => "Z".to_owned(),
        Some(sign @ (b'+' | b'-')) => {
            let h = num(15, 2)?;
            let m = s
                .get(18..20)
                .filter(|m| m.bytes().all(|c| c.is_ascii_digit()))
                .unwrap_or("00");
            format!("{}{h}:{m}", *sign as char)
        }
        _ => return None,
    };
    Some(format!("{year}-{month}-{day}T{hour}:{min}:{sec}{tz}"))
}

pub fn write_info(pdf: &mut Pdf, meta: &PdfMetadata) -> u32 {
    let mut d = String::from("<< ");
    for (key, value) in [
        ("Title", &meta.title),
        ("Author", &meta.author),
        ("Subject", &meta.subject),
        ("Keywords", &meta.keywords),
        ("CreationDate", &meta.created),
        ("ModDate", &meta.modified),
    ] {
        if let Some(v) = value.as_deref().filter(|v| !v.is_empty()) {
            d.push_str(&format!("/{key} {} ", text_string(v)));
        }
    }
    let creator = meta.creator.as_deref().unwrap_or("Open365");
    d.push_str(&format!(
        "/Creator {} /Producer (Open365) >>",
        text_string(creator)
    ));
    pdf.add(d)
}

pub fn write_xmp(pdf: &mut Pdf, meta: &PdfMetadata, conformance: Conformance, ua: bool) -> u32 {
    let mut x = String::from(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">",
    );
    x.push_str("<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"><dc:format>application/pdf</dc:format>");
    if let Some(t) = meta.title.as_deref().filter(|t| !t.is_empty()) {
        x.push_str(&format!(
            "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>",
            xml_escape(t)
        ));
    }
    if let Some(a) = meta.author.as_deref().filter(|a| !a.is_empty()) {
        x.push_str(&format!(
            "<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>",
            xml_escape(a)
        ));
    }
    if let Some(s) = meta.subject.as_deref().filter(|s| !s.is_empty()) {
        x.push_str(&format!(
            "<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>",
            xml_escape(s)
        ));
    }
    if let Some(k) = meta.keywords.as_deref().filter(|k| !k.is_empty()) {
        x.push_str(&format!("<pdf:Keywords>{}</pdf:Keywords>", xml_escape(k)));
    }
    x.push_str("<pdf:Producer>Open365</pdf:Producer>");
    x.push_str(&format!(
        "<xmp:CreatorTool>{}</xmp:CreatorTool>",
        xml_escape(meta.creator.as_deref().unwrap_or("Open365"))
    ));
    for (tag, value) in [
        ("CreateDate", &meta.created),
        ("ModifyDate", &meta.modified),
    ] {
        if let Some(iso) = value.as_deref().and_then(pdf_date_to_iso) {
            x.push_str(&format!("<xmp:{tag}>{iso}</xmp:{tag}>"));
        }
    }
    x.push_str("</rdf:Description>");
    if let Some((part, level)) = conformance.pdfa() {
        x.push_str(&format!(
            "<rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\"><pdfaid:part>{part}</pdfaid:part><pdfaid:conformance>{level}</pdfaid:conformance></rdf:Description>"
        ));
    }
    if ua {
        x.push_str("<rdf:Description rdf:about=\"\" xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\"><pdfuaid:part>1</pdfuaid:part></rdf:Description>");
    }
    x.push_str("</rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>");
    pdf.add_stream("/Type /Metadata /Subtype /XML", x.as_bytes(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_convert_both_ways() {
        assert_eq!(
            pdf_date_to_iso("D:20260101120000Z").as_deref(),
            Some("2026-01-01T12:00:00Z")
        );
        assert_eq!(
            pdf_date_to_iso("D:20260101120000+01'00'").as_deref(),
            Some("2026-01-01T12:00:00+01:00")
        );
        assert_eq!(pdf_date_to_iso("yesterday"), None);
        assert_eq!(
            iso_to_pdf_date("2026-01-01T12:00:00Z").as_deref(),
            Some("D:20260101120000Z")
        );
    }

    #[test]
    fn core_properties_are_read() {
        let xml = "<cp:coreProperties><dc:title>A &amp; B</dc:title><dc:creator/><cp:keywords> x, y </cp:keywords></cp:coreProperties>";
        assert_eq!(element_text(xml, "dc:title").as_deref(), Some("A & B"));
        assert_eq!(element_text(xml, "dc:creator"), None);
        assert_eq!(element_text(xml, "cp:keywords").as_deref(), Some("x, y"));
    }
}
