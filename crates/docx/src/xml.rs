//! A small order-preserving XML tree used for everything the typed model does not
//! interpret. Qualified names, attribute order and namespace declarations are kept
//! exactly as written; namespaces are resolved only so that elements can be recognised.

use std::sync::Arc;

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

use crate::DocxError;

/// Deepest element nesting accepted. Real documents stay far below this; the limit keeps
/// recursive code (conversion, serialisation, drop) clear of stack exhaustion.
pub const MAX_XML_DEPTH: usize = 128;

/// Main WordprocessingML namespaces (transitional and strict).
pub const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub const W_NS_STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const BOM: &str = "\u{feff}";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// True for either WordprocessingML main namespace.
pub fn is_w_ns(ns: Option<&str>) -> bool {
    matches!(ns, Some(W_NS | W_NS_STRICT))
}

/// One attribute, in source order. `value` is unescaped; `ns` is the resolved namespace
/// (`None` for unprefixed attributes and namespace declarations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub ns: Option<Arc<str>>,
    pub value: String,
}

impl Attr {
    /// Name without prefix.
    pub fn local_name(&self) -> &str {
        local_part(&self.name)
    }

    fn is_w(&self, local: &str) -> bool {
        is_w_ns(self.ns.as_deref()) && self.local_name() == local
    }

    /// True for `xml:space`.
    pub fn is_xml_space(&self) -> bool {
        self.ns.as_deref() == Some(XML_NS) && self.local_name() == "space"
    }
}

/// Name part after the prefix: `w:p` gives `p`.
pub fn local_part(qname: &str) -> &str {
    qname.rsplit_once(':').map_or(qname, |(_, local)| local)
}

/// `w:p` + `r` gives `w:r`; an unprefixed name stays unprefixed.
pub fn sibling_name(qname: &str, local: &str) -> String {
    match qname.rsplit_once(':') {
        Some((prefix, _)) => format!("{prefix}:{local}"),
        None => local.to_owned(),
    }
}

/// Name, namespace and attributes of an element, without its children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementHead {
    pub name: String,
    pub ns: Option<Arc<str>>,
    pub attrs: Vec<Attr>,
}

impl ElementHead {
    /// A head in the same namespace and prefix as `self`, with another local name.
    pub fn sibling(&self, local: &str) -> Self {
        Self {
            name: sibling_name(&self.name, local),
            ns: self.ns.clone(),
            attrs: Vec::new(),
        }
    }

    pub fn local_name(&self) -> &str {
        local_part(&self.name)
    }

    /// True when this is the WordprocessingML element `local`.
    pub fn is_w(&self, local: &str) -> bool {
        is_w_ns(self.ns.as_deref()) && self.local_name() == local
    }

    /// Value of the WordprocessingML-namespaced attribute `local` (for example `w:val`).
    pub fn attr_w(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.is_w(local))
            .map(|a| a.value.as_str())
    }

    /// Sets (or adds) the WordprocessingML-namespaced attribute `local`.
    pub fn set_attr_w(&mut self, local: &str, value: &str) {
        if let Some(attr) = self.attrs.iter_mut().find(|a| a.is_w(local)) {
            value.clone_into(&mut attr.value);
            return;
        }
        self.attrs.push(Attr {
            name: sibling_name(&self.name, local),
            ns: self.ns.clone(),
            value: value.to_owned(),
        });
    }

    /// Removes the WordprocessingML-namespaced attribute `local`.
    pub fn remove_attr_w(&mut self, local: &str) {
        self.attrs.retain(|a| !a.is_w(local));
    }
}

/// An element with its children in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlElement {
    pub head: ElementHead,
    pub children: Vec<RawXml>,
}

/// Any XML node the typed model keeps verbatim. Adjacent text and CDATA are merged
/// into one `Text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawXml {
    Element(XmlElement),
    Text(String),
    Comment(String),
    ProcessingInstruction(String),
}

impl RawXml {
    /// The element, when this node is one.
    pub fn as_element(&self) -> Option<&XmlElement> {
        match self {
            Self::Element(e) => Some(e),
            _ => None,
        }
    }
}

/// A parsed XML document: raw bytes before and after the root element are kept verbatim
/// (declaration, comments, processing instructions, whitespace).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedXml {
    pub prolog: String,
    pub root: XmlElement,
    pub epilogue: String,
}

struct Scope {
    /// (prefix, uri) pairs; the empty prefix is the default namespace.
    bindings: Vec<(String, Option<Arc<str>>)>,
    marks: Vec<usize>,
}

impl Scope {
    fn lookup(&self, prefix: &str) -> Option<Arc<str>> {
        if prefix == "xml" {
            return Some(Arc::from(XML_NS));
        }
        self.bindings
            .iter()
            .rev()
            .find(|(p, _)| p == prefix)
            .and_then(|(_, uri)| uri.clone())
    }
}

fn xml_err(e: impl std::fmt::Display) -> DocxError {
    DocxError::Xml(e.to_string())
}

/// Builds the head of a start tag, declaring its namespaces in `scope` first.
fn start_head(e: &BytesStart<'_>, scope: &mut Scope) -> Result<ElementHead, DocxError> {
    scope.marks.push(scope.bindings.len());
    let mut raw: Vec<(String, String)> = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(xml_err)?;
        let key = attr.key.as_ref().to_owned();
        let value = attr
            .normalized_value(XmlVersion::Explicit1_0)
            .map_err(xml_err)?
            .into_owned();
        if key == "xmlns" {
            let uri = (!value.is_empty()).then(|| Arc::<str>::from(value.as_str()));
            scope.bindings.push((String::new(), uri));
        } else if let Some(prefix) = key.strip_prefix("xmlns:") {
            let uri = (!value.is_empty()).then(|| Arc::<str>::from(value.as_str()));
            scope.bindings.push((prefix.to_owned(), uri));
        }
        raw.push((key, value));
    }
    let name = e.name().as_ref().to_owned();
    let ns = match name.split_once(':') {
        Some((prefix, _)) => scope.lookup(prefix),
        None => scope.lookup(""),
    };
    let attrs = raw
        .into_iter()
        .map(|(name, value)| {
            let ns = if name == "xmlns" || name.starts_with("xmlns:") {
                None
            } else {
                name.split_once(':')
                    .and_then(|(prefix, _)| scope.lookup(prefix))
            };
            Attr { name, ns, value }
        })
        .collect();
    Ok(ElementHead { name, ns, attrs })
}

fn close_scope(scope: &mut Scope) {
    if let Some(mark) = scope.marks.pop() {
        scope.bindings.truncate(mark);
    }
}

fn predefined_entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        _ => None,
    }
}

fn is_xml_blank(text: &str) -> bool {
    text.chars()
        .all(|c| matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{feff}'))
}

/// Parses `bytes` as UTF-8 XML. Rejects DOCTYPE, undefined entities, several roots and
/// nesting beyond [`MAX_XML_DEPTH`].
pub fn parse(bytes: &[u8]) -> Result<ParsedXml, DocxError> {
    let full = std::str::from_utf8(bytes).map_err(|_| DocxError::NotUtf8)?;
    // The reader's offsets exclude a byte order mark, so split it off and put it back.
    let (bom, text) = match full.strip_prefix(BOM) {
        Some(rest) => (BOM, rest),
        None => ("", full),
    };
    let mut reader = Reader::from_str(text);
    let mut scope = Scope {
        bindings: Vec::new(),
        marks: Vec::new(),
    };
    let mut stack: Vec<XmlElement> = Vec::new();
    let mut pending = String::new();
    let mut root: Option<XmlElement> = None;
    let mut prolog_end: Option<usize> = None;
    let mut epilogue_start: Option<usize> = None;

    fn flush(pending: &mut String, stack: &mut [XmlElement]) {
        if pending.is_empty() {
            return;
        }
        if let Some(top) = stack.last_mut() {
            top.children.push(RawXml::Text(std::mem::take(pending)));
        } else {
            pending.clear();
        }
    }

    loop {
        let before = usize::try_from(reader.buffer_position()).map_err(xml_err)?;
        let event = reader.read_event().map_err(xml_err)?;
        match event {
            Event::DocType(_) => return Err(DocxError::DoctypeForbidden),
            Event::Eof => break,
            Event::Decl(_) => {}
            Event::Start(e) => {
                flush(&mut pending, &mut stack);
                if root.is_some() {
                    return Err(xml_err("more than one root element"));
                }
                if stack.len() >= MAX_XML_DEPTH {
                    return Err(DocxError::DepthExceeded(MAX_XML_DEPTH));
                }
                if stack.is_empty() {
                    prolog_end = Some(before);
                }
                let head = start_head(&e, &mut scope)?;
                stack.push(XmlElement {
                    head,
                    children: Vec::new(),
                });
            }
            Event::Empty(e) => {
                flush(&mut pending, &mut stack);
                if root.is_some() {
                    return Err(xml_err("more than one root element"));
                }
                if stack.len() >= MAX_XML_DEPTH {
                    return Err(DocxError::DepthExceeded(MAX_XML_DEPTH));
                }
                let head = start_head(&e, &mut scope)?;
                close_scope(&mut scope);
                let el = XmlElement {
                    head,
                    children: Vec::new(),
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(RawXml::Element(el)),
                    None => {
                        prolog_end = Some(before);
                        epilogue_start =
                            Some(usize::try_from(reader.buffer_position()).map_err(xml_err)?);
                        root = Some(el);
                    }
                }
            }
            Event::End(_) => {
                flush(&mut pending, &mut stack);
                close_scope(&mut scope);
                let el = stack.pop().ok_or_else(|| xml_err("unbalanced end tag"))?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(RawXml::Element(el)),
                    None => {
                        epilogue_start =
                            Some(usize::try_from(reader.buffer_position()).map_err(xml_err)?);
                        root = Some(el);
                    }
                }
            }
            Event::Text(t) => {
                let content = t.xml10_content();
                if stack.is_empty() {
                    if !is_xml_blank(&content) {
                        return Err(xml_err("text outside the root element"));
                    }
                } else {
                    pending.push_str(&content);
                }
            }
            Event::CData(c) => {
                if stack.is_empty() {
                    return Err(xml_err("CDATA outside the root element"));
                }
                pending.push_str(&c.into_inner());
            }
            Event::GeneralRef(r) => {
                if stack.is_empty() {
                    return Err(xml_err("entity reference outside the root element"));
                }
                if let Some(ch) = r.resolve_char_ref().map_err(xml_err)? {
                    pending.push(ch);
                } else if let Some(ch) = predefined_entity(&r) {
                    pending.push(ch);
                } else {
                    return Err(xml_err(format!("undefined entity &{};", &*r)));
                }
            }
            Event::Comment(c) => {
                if let Some(top) = stack.last_mut() {
                    flush(&mut pending, std::slice::from_mut(top));
                    top.children.push(RawXml::Comment(c.to_string()));
                }
            }
            Event::PI(p) => {
                if let Some(top) = stack.last_mut() {
                    flush(&mut pending, std::slice::from_mut(top));
                    top.children
                        .push(RawXml::ProcessingInstruction(p.to_string()));
                }
            }
        }
    }
    if !stack.is_empty() {
        return Err(xml_err("unexpected end of document"));
    }
    let root = root.ok_or_else(|| xml_err("no root element"))?;
    let prolog_end = prolog_end.unwrap_or(0);
    let epilogue_start = epilogue_start.unwrap_or(text.len());
    Ok(ParsedXml {
        prolog: format!("{bom}{}", text.get(..prolog_end).unwrap_or_default()),
        root,
        epilogue: text.get(epilogue_start..).unwrap_or_default().to_owned(),
    })
}

/// Appends `text` with `&`, `<`, `>` and carriage returns escaped.
pub fn escape_text(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}

fn escape_attr(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}

/// Writes a start tag (or an empty-element tag when `empty`).
pub fn write_head(head: &ElementHead, empty: bool, out: &mut String) {
    out.push('<');
    out.push_str(&head.name);
    for attr in &head.attrs {
        out.push(' ');
        out.push_str(&attr.name);
        out.push_str("=\"");
        escape_attr(&attr.value, out);
        out.push('"');
    }
    out.push_str(if empty { "/>" } else { ">" });
}

pub fn write_end(head: &ElementHead, out: &mut String) {
    out.push_str("</");
    out.push_str(&head.name);
    out.push('>');
}

impl RawXml {
    pub fn write(&self, out: &mut String) {
        match self {
            Self::Element(e) => e.write(out),
            Self::Text(t) => escape_text(t, out),
            Self::Comment(c) => {
                out.push_str("<!--");
                out.push_str(c);
                out.push_str("-->");
            }
            Self::ProcessingInstruction(p) => {
                out.push_str("<?");
                out.push_str(p);
                out.push_str("?>");
            }
        }
    }
}

impl XmlElement {
    pub fn write(&self, out: &mut String) {
        if self.children.is_empty() {
            write_head(&self.head, true, out);
            return;
        }
        write_head(&self.head, false, out);
        for child in &self.children {
            child.write(out);
        }
        write_end(&self.head, out);
    }

    /// An empty element in the same namespace and prefix as `self`.
    pub fn sibling(&self, local: &str) -> Self {
        Self {
            head: self.head.sibling(local),
            children: Vec::new(),
        }
    }
}

impl ParsedXml {
    /// Serialises with the original prolog and epilogue.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::with_capacity(self.prolog.len() + self.epilogue.len() + 1024);
        out.push_str(&self.prolog);
        self.root.write(&mut out);
        out.push_str(&self.epilogue);
        out.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_namespaces_and_attribute_order_survive() {
        let src = "<?xml version=\"1.0\"?>\r\n<a:x xmlns:a=\"urn:a\" b=\"1\" a:c=\"2\"><a:y/>t&amp;u</a:x>\n";
        let doc = parse(src.as_bytes()).unwrap();
        assert_eq!(doc.prolog, "<?xml version=\"1.0\"?>\r\n");
        assert_eq!(doc.epilogue, "\n");
        assert_eq!(doc.root.head.ns.as_deref(), Some("urn:a"));
        let out = String::from_utf8(doc.to_bytes()).unwrap();
        assert_eq!(
            out,
            "<?xml version=\"1.0\"?>\r\n<a:x xmlns:a=\"urn:a\" b=\"1\" a:c=\"2\"><a:y/>t&amp;u</a:x>\n"
        );
    }

    #[test]
    fn rejects_doctype_entities_and_multiple_roots() {
        assert!(matches!(
            parse(b"<!DOCTYPE x [<!ENTITY e \"v\">]><x>&e;</x>"),
            Err(DocxError::DoctypeForbidden)
        ));
        assert!(parse(b"<x>&nope;</x>").is_err());
        assert!(parse(b"<x/><y/>").is_err());
        assert!(parse(b"<x>").is_err());
        assert!(parse(b"junk<x/>").is_err());
    }

    #[test]
    fn depth_limit_is_an_error() {
        let mut deep = String::new();
        for _ in 0..(MAX_XML_DEPTH + 10) {
            deep.push_str("<a>");
        }
        for _ in 0..(MAX_XML_DEPTH + 10) {
            deep.push_str("</a>");
        }
        assert!(matches!(
            parse(deep.as_bytes()),
            Err(DocxError::DepthExceeded(_))
        ));
        let mut ok = String::new();
        for _ in 0..MAX_XML_DEPTH {
            ok.push_str("<a>");
        }
        for _ in 0..MAX_XML_DEPTH {
            ok.push_str("</a>");
        }
        assert!(parse(ok.as_bytes()).is_ok());
    }
}
