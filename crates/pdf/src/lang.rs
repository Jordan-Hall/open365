//! Languages of the document: the default (`w:docDefaults`) and the effective language of a run
//! (direct `w:lang`, then its character style, then the paragraph style chain, then the default).
//! Word writes the primary subtag (`de` for `de-DE`) as `/Lang`, on a `Span` for text whose
//! language differs from the document's (evidence: `fixtures/reference/pdf/probes/probe-lang*`).

use std::collections::HashMap;

use open365_docx::{Document, Paragraph, RawXml, Run, RunPropItem, XmlElement, parse_xml};

#[derive(Debug, Default, Clone)]
struct StyleLang {
    based_on: Option<String>,
    lang: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct Languages {
    default: Option<String>,
    default_paragraph_style: Option<String>,
    styles: HashMap<String, StyleLang>,
}

fn element<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    e.children.iter().find_map(|c| match c {
        RawXml::Element(c) if c.head.is_w(local) => Some(c),
        _ => None,
    })
}

fn lang_of_rpr(rpr: &XmlElement) -> Option<String> {
    element(rpr, "lang")?
        .head
        .attr_w("val")
        .map(str::to_owned)
        .filter(|v| !v.is_empty())
}

/// The primary language subtag, lower case (`de-DE` gives `de`); `None` for an empty or
/// unspecified tag.
pub fn primary(tag: &str) -> Option<String> {
    let first = tag.split(['-', '_']).next()?.trim().to_ascii_lowercase();
    (!first.is_empty() && first != "none" && first != "zxx").then_some(first)
}

impl Languages {
    pub fn read(doc: &Document) -> Self {
        let mut out = Self::default();
        let Some(part) = doc.package().part("word/styles.xml") else {
            return out;
        };
        let Ok(parsed) = parse_xml(part.data()) else {
            return out;
        };
        let root = parsed.root;
        out.default = element(&root, "docDefaults")
            .and_then(|d| element(d, "rPrDefault"))
            .and_then(|d| element(d, "rPr"))
            .and_then(lang_of_rpr);
        for child in &root.children {
            let RawXml::Element(style) = child else {
                continue;
            };
            if !style.head.is_w("style") {
                continue;
            }
            let Some(id) = style.head.attr_w("styleId") else {
                continue;
            };
            if style.head.attr_w("type") == Some("paragraph")
                && style
                    .head
                    .attr_w("default")
                    .is_some_and(|v| matches!(v, "1" | "true" | "on"))
            {
                out.default_paragraph_style = Some(id.to_owned());
            }
            out.styles.insert(
                id.to_owned(),
                StyleLang {
                    based_on: element(style, "basedOn")
                        .and_then(|b| b.head.attr_w("val"))
                        .map(str::to_owned),
                    lang: element(style, "rPr").and_then(lang_of_rpr),
                },
            );
        }
        out
    }

    /// The document's default language tag as written in the file (`en-GB`).
    pub fn default_tag(&self) -> Option<&str> {
        self.default.as_deref()
    }

    fn style_lang(&self, id: &str) -> Option<&str> {
        let mut at = Some(id);
        for _ in 0..32 {
            let style = self.styles.get(at?)?;
            if let Some(l) = &style.lang {
                return Some(l);
            }
            at = style.based_on.as_deref();
        }
        None
    }

    /// The language of the text of `run` in `para`.
    pub fn of_run(&self, para: &Paragraph, run: &Run) -> Option<String> {
        let mut style_id: Option<&str> = None;
        for item in run.props.items() {
            let (RunPropItem::Known(_, e) | RunPropItem::Raw(RawXml::Element(e))) = item else {
                continue;
            };
            if e.head.is_w("lang")
                && let Some(v) = e.head.attr_w("val").filter(|v| !v.is_empty())
            {
                return Some(v.to_owned());
            }
            if e.head.is_w("rStyle") {
                style_id = e.head.attr_w("val");
            }
        }
        style_id
            .and_then(|s| self.style_lang(s))
            .or_else(|| self.of_paragraph(para))
            .map(str::to_owned)
    }

    /// The language the paragraph style gives.
    pub fn of_paragraph(&self, para: &Paragraph) -> Option<&str> {
        let id = para
            .props
            .as_ref()
            .and_then(|p| element(p, "pStyle"))
            .and_then(|s| s.head.attr_w("val"))
            .or(self.default_paragraph_style.as_deref());
        id.and_then(|id| self.style_lang(id))
            .or(self.default.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_subtag_is_lower_case() {
        assert_eq!(primary("de-DE").as_deref(), Some("de"));
        assert_eq!(primary("EN_gb").as_deref(), Some("en"));
        assert_eq!(primary("zxx"), None);
        assert_eq!(primary(""), None);
    }
}
