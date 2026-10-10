//! Minimal, non-recursive XML element scanner used by inspection. DOCTYPEs are rejected.

use quick_xml::XmlVersion;
use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// One element: local name, attributes keyed by local name, and its direct text.
#[derive(Debug, Clone, Default)]
pub(crate) struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
}

impl El {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

fn element(e: &quick_xml::events::BytesStart<'_>) -> Result<El, String> {
    let mut attrs = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|e| e.to_string())?;
        let key = attr.key.local_name().as_ref().to_owned();
        let value = attr
            .normalized_value(XmlVersion::Explicit1_0)
            .map_err(|e| e.to_string())?
            .into_owned();
        attrs.push((key, value));
    }
    Ok(El {
        name: e.local_name().as_ref().to_owned(),
        attrs,
        text: String::new(),
    })
}

/// Every element in document order.
pub(crate) fn scan(xml: &[u8]) -> Result<Vec<El>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut out: Vec<El> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?
        {
            Event::DocType(_) => return Err("DOCTYPE is not allowed".to_owned()),
            Event::Start(e) => {
                out.push(element(&e)?);
                stack.push(out.len() - 1);
            }
            Event::Empty(e) => out.push(element(&e)?),
            Event::End(_) => {
                stack.pop();
            }
            Event::Text(t) => {
                if let Some(&i) = stack.last() {
                    out[i].text.push_str(&t.xml10_content());
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}
