//! Shared test helpers: an infoset comparator that does not use the crate's own XML
//! parser, fixture discovery and a minimal DOCX builder.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use open365_opc::{Package, PartName};
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

#[derive(Debug, PartialEq, Eq)]
pub enum Node {
    Element {
        ns: String,
        local: String,
        attrs: BTreeMap<(String, String), String>,
        /// Namespace declarations (`xmlns` / `xmlns:p`) as prefix -> uri.
        decls: BTreeMap<String, String>,
        children: Vec<Node>,
    },
    Text(String),
    Comment(String),
    Pi(String),
}

fn owned_ns(result: &ResolveResult<'_>) -> String {
    match result {
        ResolveResult::Bound(ns) => ns.as_ref().to_owned(),
        ResolveResult::Unbound => String::new(),
        ResolveResult::Unknown(prefix) => format!("?{prefix}"),
    }
}

fn flush(pending: &mut String, stack: &mut [Node]) {
    if !pending.is_empty() {
        if let Some(Node::Element { children, .. }) = stack.last_mut() {
            children.push(Node::Text(std::mem::take(pending)));
        }
        pending.clear();
    }
}

fn element(
    reader: &NsReader<&[u8]>,
    ns: String,
    e: &quick_xml::events::BytesStart<'_>,
) -> Result<Node, String> {
    let mut attrs = BTreeMap::new();
    let mut decls = BTreeMap::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|e| e.to_string())?;
        let key = attr.key.as_ref();
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Explicit1_0)
            .map_err(|e| e.to_string())?
            .into_owned();
        if key == "xmlns" {
            decls.insert(String::new(), value);
        } else if let Some(prefix) = key.strip_prefix("xmlns:") {
            decls.insert(prefix.to_owned(), value);
        } else {
            let (res, local) = reader.resolver().resolve_attribute(attr.key);
            attrs.insert((owned_ns(&res), local.as_ref().to_owned()), value);
        }
    }
    Ok(Node::Element {
        ns,
        local: e.local_name().as_ref().to_owned(),
        attrs,
        decls,
        children: Vec::new(),
    })
}

/// Parses to a normalised infoset: resolved names, attribute sets, merged text.
pub fn infoset(bytes: &[u8]) -> Result<Node, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let mut reader = NsReader::from_str(text);
    let mut stack: Vec<Node> = Vec::new();
    let mut pending = String::new();
    let mut root = None;

    loop {
        let (res, event) = reader.read_resolved_event().map_err(|e| e.to_string())?;
        let ns = owned_ns(&res);
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                flush(&mut pending, &mut stack);
                // The resolver now includes this element's own declarations.
                stack.push(element(&reader, ns, &e)?);
            }
            Event::Empty(e) => {
                flush(&mut pending, &mut stack);
                let node = element(&reader, ns, &e)?;
                match stack.last_mut() {
                    Some(Node::Element { children, .. }) => children.push(node),
                    _ => root = Some(node),
                }
            }
            Event::End(_) => {
                flush(&mut pending, &mut stack);
                let node = stack.pop().ok_or("unbalanced")?;
                match stack.last_mut() {
                    Some(Node::Element { children, .. }) => children.push(node),
                    _ => root = Some(node),
                }
            }
            Event::Text(t) if !stack.is_empty() => pending.push_str(&t.xml10_content()),
            Event::CData(c) => pending.push_str(&c.into_inner()),
            Event::GeneralRef(r) => {
                if let Some(ch) = r.resolve_char_ref().map_err(|e| e.to_string())? {
                    pending.push(ch);
                } else {
                    pending.push(match &*r {
                        "lt" => '<',
                        "gt" => '>',
                        "amp" => '&',
                        "apos" => '\'',
                        "quot" => '"',
                        other => return Err(format!("entity {other}")),
                    });
                }
            }
            Event::Comment(c) if !stack.is_empty() => {
                flush(&mut pending, &mut stack);
                if let Some(Node::Element { children, .. }) = stack.last_mut() {
                    children.push(Node::Comment(c.to_string()));
                }
            }
            Event::PI(p) if !stack.is_empty() => {
                flush(&mut pending, &mut stack);
                if let Some(Node::Element { children, .. }) = stack.last_mut() {
                    children.push(Node::Pi(p.to_string()));
                }
            }
            _ => {}
        }
    }
    root.ok_or_else(|| "no root".to_owned())
}

fn first_difference(a: &Node, b: &Node, path: &mut Vec<String>) -> Option<String> {
    match (a, b) {
        (
            Node::Element {
                ns: na,
                local: la,
                attrs: aa,
                decls: da,
                children: ca,
            },
            Node::Element {
                ns: nb,
                local: lb,
                attrs: ab,
                decls: db,
                children: cb,
            },
        ) => {
            path.push(format!("{{{na}}}{la}"));
            let here = path.join("/");
            if na != nb || la != lb {
                return Some(format!("{here}: element name differs ({{{nb}}}{lb})"));
            }
            if aa != ab {
                return Some(format!("{here}: attributes differ\n  {aa:?}\n  {ab:?}"));
            }
            if da != db {
                return Some(format!("{here}: namespace declarations differ"));
            }
            if ca.len() != cb.len() {
                return Some(format!("{here}: {} children vs {}", ca.len(), cb.len()));
            }
            for (i, (x, y)) in ca.iter().zip(cb).enumerate() {
                path.push(format!("[{i}]"));
                if let Some(d) = first_difference(x, y, path) {
                    return Some(d);
                }
                path.pop();
            }
            path.pop();
            None
        }
        (x, y) if x == y => None,
        (x, y) => Some(format!("{}: {x:?} vs {y:?}", path.join("/"))),
    }
}

/// `Ok` when both byte strings have the same XML infoset (including namespace
/// declarations, which the model must keep verbatim).
pub fn assert_infoset_eq(a: &[u8], b: &[u8]) -> Result<(), String> {
    let (na, nb) = (infoset(a)?, infoset(b)?);
    match first_difference(&na, &nb, &mut Vec::new()) {
        None => Ok(()),
        Some(d) => Err(d),
    }
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Every `fixtures/docx/*.docx`, sorted by name.
pub fn fixtures() -> Vec<(String, Vec<u8>)> {
    let dir = repo_root().join("fixtures").join("docx");
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
        .expect("fixtures/docx exists")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "docx"))
        .map(|p| {
            let name = p.file_stem().expect("stem").to_string_lossy().into_owned();
            (name, std::fs::read(&p).expect("readable fixture"))
        })
        .collect();
    out.sort();
    out
}

pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="sigs" ContentType="application/vnd.openxmlformats-package.digital-signature-origin"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;

pub fn root_rels(main: &str, extra: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="{main}"/>{extra}</Relationships>"#
    )
}

/// A minimal package whose main part is `word/document.xml` wrapping `body`.
pub fn docx_with_body(body: &str) -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#
    );
    build(
        &document,
        "word/document.xml",
        &root_rels("word/document.xml", ""),
        &[],
    )
}

/// Builds a package; `main_name` is the main part's name, `extra` more parts.
pub fn build(document_xml: &str, main_name: &str, rels: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut package = Package::default();
    let mut put = |name: &str, data: &[u8]| {
        package.set_part(PartName::new(name).expect("valid name"), data.to_vec());
    };
    put("[Content_Types].xml", CONTENT_TYPES.as_bytes());
    put("_rels/.rels", rels.as_bytes());
    put(main_name, document_xml.as_bytes());
    for (name, data) in extra {
        put(name, data);
    }
    package.write().expect("package writes")
}
