//! A small order-preserving XML tree used for everything the typed model does not
//! interpret, plus the hardened pull parser the worksheet reader streams from.
//! Qualified names, attribute order and namespace declarations are kept exactly as
//! written; namespaces are resolved only so that elements can be recognised.

use std::sync::Arc;

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

use crate::XlsxError;

/// Deepest element nesting accepted. Real workbooks stay far below this; the limit keeps
/// recursive code (conversion, serialisation, drop) clear of stack exhaustion.
pub const MAX_XML_DEPTH: usize = 128;

/// SpreadsheetML main namespaces (transitional and strict).
pub const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
pub const MAIN_NS_STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const R_NS_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const BOM: &str = "\u{feff}";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// True for either SpreadsheetML main namespace.
pub fn is_main_ns(ns: Option<&str>) -> bool {
    matches!(ns, Some(MAIN_NS | MAIN_NS_STRICT))
}

fn is_r_ns(ns: Option<&str>) -> bool {
    matches!(ns, Some(R_NS | R_NS_STRICT))
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
    /// An unprefixed attribute.
    pub fn plain(name: &str, value: &str) -> Self {
        Self {
            name: name.to_owned(),
            ns: None,
            value: value.to_owned(),
        }
    }

    /// Name without prefix.
    pub fn local_name(&self) -> &str {
        local_part(&self.name)
    }
}

/// Name part after the prefix: `x:c` gives `c`.
pub fn local_part(qname: &str) -> &str {
    qname.rsplit_once(':').map_or(qname, |(_, local)| local)
}

/// `x:c` + `row` gives `x:row`; an unprefixed name stays unprefixed.
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

    /// True when this is the SpreadsheetML element `local`.
    pub fn is_main(&self, local: &str) -> bool {
        is_main_ns(self.ns.as_deref()) && self.local_name() == local
    }

    /// Value of the unprefixed attribute `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns.is_none() && a.name == name)
            .map(|a| a.value.as_str())
    }

    /// Value of the relationships-namespaced attribute `local` (for example `r:id`).
    pub fn attr_r(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| is_r_ns(a.ns.as_deref()) && a.local_name() == local)
            .map(|a| a.value.as_str())
    }

    /// Sets (or adds) the unprefixed attribute `name`.
    pub fn set_attr(&mut self, name: &str, value: &str) {
        if let Some(attr) = self
            .attrs
            .iter_mut()
            .find(|a| a.ns.is_none() && a.name == name)
        {
            value.clone_into(&mut attr.value);
            return;
        }
        self.attrs.push(Attr::plain(name, value));
    }

    /// Removes the unprefixed attribute `name`.
    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|a| !(a.ns.is_none() && a.name == name));
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

    pub fn as_element_mut(&mut self) -> Option<&mut XmlElement> {
        match self {
            Self::Element(e) => Some(e),
            _ => None,
        }
    }
}

impl XmlElement {
    /// An empty element in the same namespace and prefix as `self`.
    pub fn sibling(&self, local: &str) -> Self {
        Self {
            head: self.head.sibling(local),
            children: Vec::new(),
        }
    }

    /// Child elements in order.
    pub fn elements(&self) -> impl Iterator<Item = &XmlElement> {
        self.children.iter().filter_map(RawXml::as_element)
    }

    /// First child that is the SpreadsheetML element `local`.
    pub fn child_main(&self, local: &str) -> Option<&XmlElement> {
        self.elements().find(|e| e.head.is_main(local))
    }

    /// Concatenated text of direct text children.
    pub fn own_text(&self) -> String {
        let mut out = String::new();
        for child in &self.children {
            if let RawXml::Text(t) = child {
                out.push_str(t);
            }
        }
        out
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
        self.lookup_ref(prefix).cloned()
    }

    fn lookup_ref(&self, prefix: &str) -> Option<&Arc<str>> {
        if prefix == "xml" {
            return None;
        }
        self.bindings
            .iter()
            .rev()
            .find(|(p, _)| p == prefix)
            .and_then(|(_, uri)| uri.as_ref())
    }
}

pub(crate) fn xml_err(e: impl std::fmt::Display) -> XlsxError {
    XlsxError::Xml(e.to_string())
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

/// True for text that may sit between the elements of an element-only content model.
pub(crate) fn is_blank(text: &str) -> bool {
    is_xml_blank(text)
}

/// The root element's start, as returned by [`Parser::start_document`].
pub struct RootStart {
    pub prolog: String,
    pub head: ElementHead,
    pub empty: bool,
}

/// Hardened pull parser over UTF-8 text. Rejects DOCTYPE, undefined entities and nesting
/// beyond [`MAX_XML_DEPTH`]; tracks namespace scope.
pub struct Parser<'a> {
    reader: Reader<&'a [u8]>,
    text: &'a str,
    bom: &'static str,
    scope: Scope,
    depth: usize,
    epilogue_start: usize,
    /// Offset of the reader's input within `text` (non-zero after `fast_forward`).
    base: usize,
}

impl<'a> Parser<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, XlsxError> {
        let full = std::str::from_utf8(bytes).map_err(|_| XlsxError::NotUtf8)?;
        // The reader's offsets exclude a byte order mark, so split it off and put it back.
        let (bom, text) = match full.strip_prefix(BOM) {
            Some(rest) => (BOM, rest),
            None => ("", full),
        };
        Ok(Self {
            reader: Reader::from_str(text),
            text,
            bom,
            scope: Scope {
                bindings: Vec::new(),
                marks: Vec::new(),
            },
            depth: 0,
            epilogue_start: text.len(),
            base: 0,
        })
    }

    /// Byte offset just after the last event, within [`text`](Self::text).
    pub fn position(&self) -> Result<usize, XlsxError> {
        let in_slice = usize::try_from(self.reader.buffer_position()).map_err(xml_err)?;
        Ok(self.base + in_slice)
    }

    /// The document text (without a byte order mark).
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// Current element nesting depth.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Continues reading at byte offset `at` of [`text`](Self::text), skipping everything
    /// in between. The caller has parsed that range itself and keeps scope and depth
    /// consistent.
    pub fn fast_forward(&mut self, at: usize) -> Result<(), XlsxError> {
        let rest = self
            .text
            .get(at..)
            .ok_or_else(|| xml_err("fast-forward target is not a character boundary"))?;
        self.reader = Reader::from_str(rest);
        // The new reader has not seen the enclosing start tags, so it cannot match end
        // tags; callers check end names themselves (see `check_end`).
        self.reader.config_mut().check_end_names = false;
        self.reader.config_mut().allow_unmatched_ends = true;
        self.base = at;
        Ok(())
    }

    /// Next event; DOCTYPE is an error.
    pub fn next_event(&mut self) -> Result<Event<'a>, XlsxError> {
        let event = self.reader.read_event().map_err(xml_err)?;
        if matches!(event, Event::DocType(_)) {
            return Err(XlsxError::DoctypeForbidden);
        }
        Ok(event)
    }

    /// Opens the scope of a start tag and checks the depth limit. Returns the head.
    pub fn enter(&mut self, e: &BytesStart<'_>) -> Result<ElementHead, XlsxError> {
        self.push_scope()?;
        let mut raw: Vec<(String, String)> = Vec::new();
        for attr in e.attributes() {
            let attr = attr.map_err(xml_err)?;
            let key = attr.key.as_ref().to_owned();
            let value = attr
                .normalized_value(XmlVersion::Explicit1_0)
                .map_err(xml_err)?
                .into_owned();
            self.declare(&key, &value);
            raw.push((key, value));
        }
        let name = e.name().as_ref().to_owned();
        let ns = self.resolve_element(&name);
        let attrs = raw
            .into_iter()
            .map(|(name, value)| {
                let ns = self.resolve_attr(&name);
                Attr { name, ns, value }
            })
            .collect();
        Ok(ElementHead { name, ns, attrs })
    }

    /// Opens a scope (depth check plus marker) without building a head.
    pub fn push_scope(&mut self) -> Result<(), XlsxError> {
        if self.depth >= MAX_XML_DEPTH {
            return Err(XlsxError::DepthExceeded(MAX_XML_DEPTH));
        }
        self.depth += 1;
        self.scope.marks.push(self.scope.bindings.len());
        Ok(())
    }

    /// Records a namespace declaration when `key` is `xmlns` or `xmlns:p`.
    pub fn declare(&mut self, key: &str, value: &str) {
        let prefix = if key == "xmlns" {
            ""
        } else if let Some(prefix) = key.strip_prefix("xmlns:") {
            prefix
        } else {
            return;
        };
        let uri = (!value.is_empty()).then(|| Arc::<str>::from(value));
        self.scope.bindings.push((prefix.to_owned(), uri));
    }

    /// Namespace of an element name in the current scope.
    pub fn resolve_element(&self, qname: &str) -> Option<Arc<str>> {
        match qname.split_once(':') {
            Some((prefix, _)) => self.scope.lookup(prefix),
            None => self.scope.lookup(""),
        }
    }

    /// True when the element name is in a SpreadsheetML main namespace.
    pub fn is_main_element(&self, qname: &str) -> bool {
        let prefix = qname.split_once(':').map_or("", |(p, _)| p);
        is_main_ns(self.scope.lookup_ref(prefix).map(AsRef::as_ref))
    }

    /// Namespace of an attribute name (`None` for unprefixed names and declarations).
    pub fn resolve_attr(&self, name: &str) -> Option<Arc<str>> {
        if name == "xmlns" || name.starts_with("xmlns:") {
            return None;
        }
        let (prefix, _) = name.split_once(':')?;
        if prefix == "xml" {
            return Some(Arc::from(XML_NS));
        }
        self.scope.lookup(prefix)
    }

    /// Closes the scope opened by [`enter`](Self::enter) or [`push_scope`](Self::push_scope).
    pub fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        if let Some(mark) = self.scope.marks.pop() {
            self.scope.bindings.truncate(mark);
        }
    }

    /// Reads the children of `el` up to and including its end tag, then closes its scope.
    /// The element must have been opened with [`enter`](Self::enter).
    pub fn read_content(&mut self, el: &mut XmlElement) -> Result<(), XlsxError> {
        let mut pending = String::new();
        loop {
            match self.next_event()? {
                Event::Start(e) => {
                    flush(&mut pending, &mut el.children);
                    let head = self.enter(&e)?;
                    let mut child = XmlElement {
                        head,
                        children: Vec::new(),
                    };
                    self.read_content(&mut child)?;
                    el.children.push(RawXml::Element(child));
                }
                Event::Empty(e) => {
                    flush(&mut pending, &mut el.children);
                    let head = self.enter(&e)?;
                    self.leave();
                    el.children.push(RawXml::Element(XmlElement {
                        head,
                        children: Vec::new(),
                    }));
                }
                Event::End(e) => {
                    check_end(e.name().as_ref(), &el.head.name)?;
                    flush(&mut pending, &mut el.children);
                    self.leave();
                    return Ok(());
                }
                Event::Text(t) => pending.push_str(&t.xml10_content()),
                Event::CData(c) => pending.push_str(&c.into_inner()),
                Event::GeneralRef(r) => push_entity(&r, &mut pending)?,
                Event::Comment(c) => {
                    flush(&mut pending, &mut el.children);
                    el.children.push(RawXml::Comment(c.to_string()));
                }
                Event::PI(p) => {
                    flush(&mut pending, &mut el.children);
                    el.children
                        .push(RawXml::ProcessingInstruction(p.to_string()));
                }
                Event::Decl(_) => {}
                Event::Eof => return Err(xml_err("unexpected end of document")),
                Event::DocType(_) => return Err(XlsxError::DoctypeForbidden),
            }
        }
    }

    /// Reads an element whose start tag `e` was just returned (`empty` for `<x/>`).
    pub fn read_element(
        &mut self,
        e: &BytesStart<'_>,
        empty: bool,
    ) -> Result<XmlElement, XlsxError> {
        let head = self.enter(e)?;
        let mut el = XmlElement {
            head,
            children: Vec::new(),
        };
        if empty {
            self.leave();
        } else {
            self.read_content(&mut el)?;
        }
        Ok(el)
    }

    /// Reads the declaration, comments and whitespace before the root element and opens
    /// the root. For a non-empty root the caller reads its content with
    /// [`read_content`](Self::read_content) (or by hand, calling [`leave`](Self::leave)).
    pub fn start_document(&mut self) -> Result<RootStart, XlsxError> {
        loop {
            let before = self.position()?;
            let event = self.next_event()?;
            let (e, empty) = match event {
                Event::Start(e) => (e, false),
                Event::Empty(e) => (e, true),
                Event::Decl(_) | Event::Comment(_) | Event::PI(_) => continue,
                Event::Text(t) => {
                    if is_xml_blank(&t.xml10_content()) {
                        continue;
                    }
                    return Err(xml_err("text outside the root element"));
                }
                Event::Eof => return Err(xml_err("no root element")),
                Event::CData(_) => return Err(xml_err("CDATA outside the root element")),
                Event::GeneralRef(_) => {
                    return Err(xml_err("entity reference outside the root element"));
                }
                Event::End(_) => return Err(xml_err("unbalanced end tag")),
                Event::DocType(_) => return Err(XlsxError::DoctypeForbidden),
            };
            let head = self.enter(&e)?;
            let prolog = format!(
                "{}{}",
                self.bom,
                self.text.get(..before).unwrap_or_default()
            );
            if empty {
                self.leave();
                self.epilogue_start = self.position()?;
            }
            return Ok(RootStart {
                prolog,
                head,
                empty,
            });
        }
    }

    /// Call after the root's end tag was consumed by [`read_content`](Self::read_content)
    /// (or when the root was empty). Validates the rest of the document and returns it
    /// verbatim as the epilogue.
    pub fn finish_document(&mut self, root_was_empty: bool) -> Result<String, XlsxError> {
        if !root_was_empty {
            self.epilogue_start = self.position()?;
        }
        loop {
            match self.next_event()? {
                Event::Eof => break,
                Event::Start(_) | Event::Empty(_) => {
                    return Err(xml_err("more than one root element"));
                }
                Event::Text(t) => {
                    if !is_xml_blank(&t.xml10_content()) {
                        return Err(xml_err("text outside the root element"));
                    }
                }
                Event::CData(_) => return Err(xml_err("CDATA outside the root element")),
                Event::GeneralRef(_) => {
                    return Err(xml_err("entity reference outside the root element"));
                }
                Event::End(_) => return Err(xml_err("unbalanced end tag")),
                Event::Comment(_) | Event::PI(_) | Event::Decl(_) | Event::DocType(_) => {}
            }
        }
        Ok(self
            .text
            .get(self.epilogue_start..)
            .unwrap_or_default()
            .to_owned())
    }
}

/// An end tag must carry the name of the element it closes.
pub(crate) fn check_end(found: &str, expected: &str) -> Result<(), XlsxError> {
    if found == expected {
        Ok(())
    } else {
        Err(xml_err(format!(
            "end tag </{found}> does not match <{expected}>"
        )))
    }
}

/// Appends the character a general reference stands for.
pub(crate) fn push_entity(
    r: &quick_xml::events::BytesRef<'_>,
    out: &mut String,
) -> Result<(), XlsxError> {
    if let Some(ch) = r.resolve_char_ref().map_err(xml_err)? {
        out.push(ch);
    } else if let Some(ch) = predefined_entity(r) {
        out.push(ch);
    } else {
        return Err(xml_err(format!("undefined entity &{};", &**r)));
    }
    Ok(())
}

fn flush(pending: &mut String, children: &mut Vec<RawXml>) {
    if !pending.is_empty() {
        children.push(RawXml::Text(std::mem::take(pending)));
    }
}

/// Parses `bytes` as UTF-8 XML. Rejects DOCTYPE, undefined entities, several roots and
/// nesting beyond [`MAX_XML_DEPTH`].
pub fn parse(bytes: &[u8]) -> Result<ParsedXml, XlsxError> {
    let mut parser = Parser::new(bytes)?;
    let start = parser.start_document()?;
    let mut root = XmlElement {
        head: start.head,
        children: Vec::new(),
    };
    if !start.empty {
        parser.read_content(&mut root)?;
    }
    let epilogue = parser.finish_document(start.empty)?;
    Ok(ParsedXml {
        prolog: start.prolog,
        root,
        epilogue,
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

pub(crate) fn escape_attr(value: &str, out: &mut String) {
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

/// Writes ` name="value"` for each attribute.
pub(crate) fn write_attrs(attrs: &[Attr], out: &mut String) {
    for attr in attrs {
        out.push(' ');
        out.push_str(&attr.name);
        out.push_str("=\"");
        escape_attr(&attr.value, out);
        out.push('"');
    }
}

/// Writes a start tag (or an empty-element tag when `empty`).
pub fn write_head(head: &ElementHead, empty: bool, out: &mut String) {
    out.push('<');
    out.push_str(&head.name);
    write_attrs(&head.attrs, out);
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
            Err(XlsxError::DoctypeForbidden)
        ));
        assert!(parse(b"<x>&nope;</x>").is_err());
        assert!(parse(b"<x/><y/>").is_err());
        assert!(parse(b"<x>").is_err());
        assert!(parse(b"junk<x/>").is_err());
    }

    #[test]
    fn depth_limit_is_an_error() {
        let nested = |n: usize| format!("{}{}", "<a>".repeat(n), "</a>".repeat(n));
        assert!(matches!(
            parse(nested(MAX_XML_DEPTH + 10).as_bytes()),
            Err(XlsxError::DepthExceeded(_))
        ));
        assert!(parse(nested(MAX_XML_DEPTH).as_bytes()).is_ok());
    }
}
