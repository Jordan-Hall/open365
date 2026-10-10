//! `sharedStrings.xml`: the raw `si` items stay as they are (rich text included); the
//! plain text of each item is extracted for lookup, and new plain items can be appended.

use std::collections::HashMap;
use std::sync::Arc;

use crate::XlsxError;
use crate::xml::{self, Attr, RawXml, XmlElement};

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

fn hex4(s: &[char]) -> Option<u32> {
    if s.len() < 4 {
        return None;
    }
    let mut v = 0u32;
    for c in &s[..4] {
        v = v * 16 + c.to_digit(16)?;
    }
    Some(v)
}

/// The code unit of an `_xHHHH_` escape starting at `chars[i]`.
fn escape_at(chars: &[char], i: usize) -> Option<u32> {
    if chars.get(i) == Some(&'_')
        && matches!(chars.get(i + 1), Some('x' | 'X'))
        && chars.get(i + 6) == Some(&'_')
    {
        hex4(chars.get(i + 2..)?)
    } else {
        None
    }
}

/// Decodes the `_xHHHH_` escapes of `ST_Xstring` (a UTF-16 code unit per escape; a
/// high/low surrogate pair of escapes forms one character).
pub fn decode_xstring(s: &str) -> String {
    if !s.contains("_x") && !s.contains("_X") {
        return s.to_owned();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if let Some(unit) = escape_at(&chars, i) {
            if (0xD800..0xDC00).contains(&unit)
                && let Some(low) = escape_at(&chars, i + 7)
                && (0xDC00..0xE000).contains(&low)
                && let Some(c) = char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))
            {
                out.push(c);
                i += 14;
                continue;
            }
            if let Some(c) = char::from_u32(unit) {
                out.push(c);
                i += 7;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Encodes `s` for an `ST_Xstring` element: characters XML 1.0 cannot carry (and `\r`,
/// which parsers normalise) become `_xHHHH_`, and a literal `_xHHHH_` is protected by
/// escaping its underscore.
pub fn encode_xstring(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        let needs_escape =
            matches!(c, '\u{0}'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}');
        if needs_escape {
            out.push_str(&format!("_x{:04X}_", u32::from(c)));
        } else if c == '_' && escape_at(&chars, i).is_some() {
            out.push_str("_x005F_");
        } else {
            out.push(c);
        }
    }
    out
}

/// True when `text` needs `xml:space="preserve"` to survive a round trip.
fn needs_preserve(text: &str) -> bool {
    text.starts_with(char::is_whitespace)
        || text.ends_with(char::is_whitespace)
        || text.contains('\n')
        || text.contains('\t')
        || text.contains("  ")
}

fn is_blank_text(node: &RawXml) -> bool {
    matches!(node, RawXml::Text(t) if xml::is_blank(t))
}

/// Plain text of one `si` (or `is`) item: `t` and `r/t` runs, not phonetic runs.
pub(crate) fn item_text(si: &XmlElement) -> String {
    fn t_text(t: &XmlElement) -> String {
        decode_xstring(&t.own_text())
    }
    let mut out = String::new();
    for child in si.elements() {
        if child.head.is_main("t") {
            out.push_str(&t_text(child));
        } else if child.head.is_main("r") {
            for t in child.elements().filter(|e| e.head.is_main("t")) {
                out.push_str(&t_text(t));
            }
        }
    }
    out
}

/// `<si><t>text</t></si>` with no formatting and no phonetic data.
fn is_plain_item(si: &XmlElement) -> bool {
    let mut elements = si.elements();
    let Some(t) = elements.next() else {
        return false;
    };
    elements.next().is_none()
        && t.head.is_main("t")
        && t.head
            .attrs
            .iter()
            .all(|a| a.local_name() == "space" && a.ns.as_deref() == Some(XML_NS))
        && si.children.iter().all(|c| match c {
            RawXml::Element(_) => true,
            other => is_blank_text(other),
        })
        && t.children.iter().all(|c| matches!(c, RawXml::Text(_)))
}

/// The shared string table.
#[derive(Debug, Clone)]
pub struct SharedStrings {
    prolog: String,
    root: XmlElement,
    epilogue: String,
    texts: Vec<Arc<str>>,
    /// Text to index of plain items; built on first use.
    plain: Option<HashMap<Arc<str>, u32>>,
}

impl SharedStrings {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, XlsxError> {
        let parsed = xml::parse(bytes)?;
        if !parsed.root.head.is_main("sst") {
            return Err(XlsxError::NotSpreadsheet(
                "shared strings part is not an sst".to_owned(),
            ));
        }
        let texts = parsed
            .root
            .elements()
            .filter(|e| e.head.is_main("si"))
            .map(|si| Arc::<str>::from(item_text(si)))
            .collect();
        Ok(Self {
            prolog: parsed.prolog,
            root: parsed.root,
            epilogue: parsed.epilogue,
            texts,
            plain: None,
        })
    }

    /// An empty table with a conventional `sst` root.
    pub(crate) fn new_empty(main_ns: &str) -> Self {
        let root = XmlElement {
            head: crate::xml::ElementHead {
                name: "sst".to_owned(),
                ns: Some(Arc::from(main_ns)),
                attrs: vec![
                    Attr::plain("xmlns", main_ns),
                    Attr::plain("count", "0"),
                    Attr::plain("uniqueCount", "0"),
                ],
            },
            children: Vec::new(),
        };
        Self {
            prolog: "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n".to_owned(),
            root,
            epilogue: String::new(),
            texts: Vec::new(),
            plain: None,
        }
    }

    /// Number of items.
    pub fn len(&self) -> usize {
        self.texts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    /// Plain text of item `index`.
    pub fn text(&self, index: u32) -> Option<&Arc<str>> {
        self.texts.get(usize::try_from(index).ok()?)
    }

    pub(crate) fn texts(&self) -> &[Arc<str>] {
        &self.texts
    }

    /// Index of a plain item with exactly this text, appending one when none exists.
    /// Returns the index and the shared text.
    pub(crate) fn intern(&mut self, text: &str) -> (u32, Arc<str>) {
        if self.plain.is_none() {
            let mut map: HashMap<Arc<str>, u32> = HashMap::new();
            let items = self.root.elements().filter(|e| e.head.is_main("si"));
            for (index, si) in (0u32..).zip(items) {
                if is_plain_item(si) {
                    map.entry(Arc::clone(&self.texts[index as usize]))
                        .or_insert(index);
                }
            }
            self.plain = Some(map);
        }
        if let Some(map) = &self.plain
            && let Some(&index) = map.get(text)
        {
            return (index, Arc::clone(&self.texts[index as usize]));
        }
        let index = u32::try_from(self.texts.len()).unwrap_or(u32::MAX);
        let shared: Arc<str> = Arc::from(text);
        let mut t = self.root.sibling("t");
        if needs_preserve(text) {
            t.head.attrs.push(Attr {
                name: "xml:space".to_owned(),
                ns: Some(Arc::from(XML_NS)),
                value: "preserve".to_owned(),
            });
        }
        let encoded = encode_xstring(text);
        if !encoded.is_empty() {
            t.children.push(RawXml::Text(encoded));
        }
        let mut si = self.root.sibling("si");
        si.children.push(RawXml::Element(t));
        let at = self
            .root
            .children
            .iter()
            .rposition(|c| matches!(c, RawXml::Element(e) if e.head.is_main("si")))
            .map_or(self.root.children.len(), |p| p + 1);
        // Inserted after the last item, so an extension list stays last.
        self.root.children.insert(at, RawXml::Element(si));
        self.texts.push(Arc::clone(&shared));
        if let Some(map) = &mut self.plain {
            map.insert(Arc::clone(&shared), index);
        }
        let unique = self.texts.len().to_string();
        self.root.head.set_attr("uniqueCount", &unique);
        (index, shared)
    }

    /// Adjusts the `count` attribute (total references) when the file declares one.
    pub(crate) fn adjust_count(&mut self, delta: i64) {
        if let Some(current) = self
            .root
            .head
            .attr("count")
            .and_then(|v| v.trim().parse::<i64>().ok())
        {
            let next = (current + delta).max(0);
            self.root.head.set_attr("count", &next.to_string());
        }
    }

    pub(crate) fn serialize(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str(&self.prolog);
        self.root.write(&mut out);
        out.push_str(&self.epilogue);
        out.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SST: &[u8] = br#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="3" uniqueCount="2"><si><t>plain</t></si><si><r><rPr><b/></rPr><t xml:space="preserve">rich </t></r><r><t>text</t></r><rPh sb="0" eb="1"><t>ph</t></rPh></si></sst>"#;

    #[test]
    fn extracts_plain_and_rich_text() {
        let sst = SharedStrings::parse(SST).unwrap();
        assert_eq!(sst.len(), 2);
        assert_eq!(&**sst.text(0).unwrap(), "plain");
        assert_eq!(&**sst.text(1).unwrap(), "rich text");
    }

    #[test]
    fn intern_reuses_plain_items_and_appends_new_ones() {
        let mut sst = SharedStrings::parse(SST).unwrap();
        assert_eq!(sst.intern("plain").0, 0);
        // The rich item is not reused for its text.
        let (index, _) = sst.intern("rich text");
        assert_eq!(index, 2);
        assert_eq!(sst.intern("rich text").0, 2);
        let out = String::from_utf8(sst.serialize()).unwrap();
        assert!(out.contains("uniqueCount=\"3\""));
        assert!(out.ends_with("<si><t>rich text</t></si></sst>"));
    }

    #[test]
    fn xstring_escapes_round_trip() {
        assert_eq!(decode_xstring("a_x000D_b_x005F_x0041_"), "a\rb_x0041_");
        assert_eq!(decode_xstring("_xD83D__xDE00_"), "\u{1F600}");
        assert_eq!(encode_xstring("a\rb\u{1}"), "a_x000D_b_x0001_");
        assert_eq!(encode_xstring("_x0041_"), "_x005F_x0041_");
        for s in ["plain", "_x0041_", "tab\there", "\u{1}\u{2}", "_x", "a_b"] {
            assert_eq!(decode_xstring(&encode_xstring(s)), s);
        }
    }
}
