//! A minimal element tree for reading DrawingML fragments.
//!
//! Only elements and attributes are kept, matched by local name (the host validates
//! namespaces before it hands a fragment over). Every element remembers its byte span
//! in the source, so the verbatim text of any sub-tree can be recovered for round trips.

use std::ops::Range;

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

use crate::error::ParseError;

/// Deepest element nesting accepted.
pub const MAX_DEPTH: usize = 64;

/// One element: qualified name, attributes in source order and child elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    /// Byte range of this element in the source text, start tag to end tag inclusive.
    pub span: Range<usize>,
}

fn local(q: &str) -> &str {
    q.rsplit_once(':').map_or(q, |(_, l)| l)
}

impl Node {
    /// Name without prefix.
    pub fn local_name(&self) -> &str {
        local(&self.name)
    }

    /// Value of the attribute whose local name is `name` (prefix ignored).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| local(k) == name)
            .map(|(_, v)| v.as_str())
    }

    /// Parsed attribute.
    pub fn attr_i64(&self, name: &str) -> Option<i64> {
        self.attr(name)?.trim().parse().ok()
    }

    /// Boolean attribute (`1`/`true`).
    pub fn attr_bool(&self, name: &str) -> Option<bool> {
        match self.attr(name)? {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        }
    }

    /// First child with this local name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.local_name() == name)
    }

    /// All children with this local name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.local_name() == name)
    }

    /// First descendant (document order, not including `self`) with this local name.
    pub fn find(&self, name: &str) -> Option<&Node> {
        for c in &self.children {
            if c.local_name() == name {
                return Some(c);
            }
            if let Some(f) = c.find(name) {
                return Some(f);
            }
        }
        None
    }
}

fn head(e: &BytesStart<'_>, start: usize, end: usize) -> Result<Node, ParseError> {
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|x| ParseError::Xml(x.to_string()))?;
        let key = a.key.as_ref().to_owned();
        let value = a
            .normalized_value(XmlVersion::Explicit1_0)
            .map_err(|x| ParseError::Xml(x.to_string()))?
            .into_owned();
        attrs.push((key, value));
    }
    Ok(Node {
        name: e.name().as_ref().to_owned(),
        attrs,
        children: Vec::new(),
        span: start..end,
    })
}

/// Parses a fragment with exactly one root element. DOCTYPE is rejected.
pub fn parse(src: &str) -> Result<Node, ParseError> {
    let mut reader = Reader::from_str(src);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    loop {
        let before = usize::try_from(reader.buffer_position())
            .map_err(|e| ParseError::Xml(e.to_string()))?;
        let ev = reader
            .read_event()
            .map_err(|e| ParseError::Xml(e.to_string()))?;
        let after = usize::try_from(reader.buffer_position())
            .map_err(|e| ParseError::Xml(e.to_string()))?;
        match ev {
            Event::DocType(_) => return Err(ParseError::Doctype),
            Event::Eof => break,
            Event::Start(e) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(ParseError::TooDeep);
                }
                if root.is_some() && stack.is_empty() {
                    return Err(ParseError::Xml("more than one root element".into()));
                }
                stack.push(head(&e, before, after)?);
            }
            Event::Empty(e) => {
                if root.is_some() && stack.is_empty() {
                    return Err(ParseError::Xml("more than one root element".into()));
                }
                let n = head(&e, before, after)?;
                match stack.last_mut() {
                    Some(p) => p.children.push(n),
                    None => root = Some(n),
                }
            }
            Event::End(_) => {
                let mut n = stack
                    .pop()
                    .ok_or_else(|| ParseError::Xml("unbalanced end tag".into()))?;
                n.span.end = after;
                match stack.last_mut() {
                    Some(p) => p.children.push(n),
                    None => root = Some(n),
                }
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(ParseError::Xml("unclosed element".into()));
    }
    root.ok_or_else(|| ParseError::Xml("empty document".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_recover_verbatim_text() {
        let src = r#"<a:x k="1"><a:y/> <b:z>t</b:z></a:x>"#;
        let n = parse(src).unwrap();
        assert_eq!(&src[n.span.clone()], src);
        let z = n.child("z").unwrap();
        assert_eq!(&src[z.span.clone()], "<b:z>t</b:z>");
        let y = n.child("y").unwrap();
        assert_eq!(&src[y.span.clone()], "<a:y/>");
        assert_eq!(n.attr("k"), Some("1"));
    }

    #[test]
    fn rejects_doctype_and_depth() {
        assert_eq!(parse("<!DOCTYPE x><x/>"), Err(ParseError::Doctype));
        let deep = "<a>".repeat(100);
        assert_eq!(parse(&deep), Err(ParseError::TooDeep));
    }
}
