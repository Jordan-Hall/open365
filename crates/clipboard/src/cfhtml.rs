//! The Windows "HTML Format" clipboard container: a short ASCII header of byte offsets in front
//! of an HTML document, with the copied part between `<!--StartFragment-->` and
//! `<!--EndFragment-->`.

use crate::limits::MAX_HTML_BYTES;
use crate::model::ParseError;

const START: &str = "<!--StartFragment-->";
const END: &str = "<!--EndFragment-->";

/// What the document is, so the root element can declare the namespaces its consumers expect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Text,
    Sheet,
}

/// Wraps an HTML fragment in the container. The result is UTF-8 and ends with a NUL byte, as
/// the format's producers write it.
pub fn wrap(fragment: &str, flavor: Flavor) -> Vec<u8> {
    let root = match flavor {
        Flavor::Text => "<html>",
        Flavor::Sheet => {
            "<html xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\" xmlns=\"http://www.w3.org/TR/REC-html40\">"
        }
    };
    let prefix =
        format!("{root}\r\n<head>\r\n<meta charset=\"utf-8\">\r\n</head>\r\n<body>\r\n{START}");
    let suffix = format!("{END}\r\n</body>\r\n</html>");
    let header = |a: usize, b: usize, c: usize, d: usize| {
        format!(
            "Version:1.0\r\nStartHTML:{a:010}\r\nEndHTML:{b:010}\r\nStartFragment:{c:010}\r\nEndFragment:{d:010}\r\n"
        )
    };
    let hl = header(0, 0, 0, 0).len();
    let start_html = hl;
    let start_frag = hl + prefix.len();
    let end_frag = start_frag + fragment.len();
    let end_html = end_frag + suffix.len();
    let mut out = header(start_html, end_html, start_frag, end_frag).into_bytes();
    out.extend_from_slice(prefix.as_bytes());
    out.extend_from_slice(fragment.as_bytes());
    out.extend_from_slice(suffix.as_bytes());
    out.push(0);
    out
}

fn field(header: &str, key: &str) -> Option<usize> {
    header.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(key))
            .then(|| v.trim().parse::<usize>().ok())
            .flatten()
    })
}

/// Reads the container. The returned HTML always carries the fragment markers when the header
/// named a valid fragment. Data without a header is taken as plain HTML.
pub fn unwrap(data: &[u8]) -> Result<String, ParseError> {
    if data.len() > MAX_HTML_BYTES {
        return Err(ParseError::TooLarge);
    }
    let mut data = data;
    while let [rest @ .., 0] = data {
        data = rest;
    }
    // The header is the leading run of `Key:digits` lines.
    let mut header_end = 0;
    let mut pos = 0;
    while pos < data.len() {
        let line_end = data[pos..]
            .iter()
            .position(|&c| c == b'\n')
            .map_or(data.len(), |p| pos + p + 1);
        let line = std::str::from_utf8(&data[pos..line_end]).unwrap_or("");
        let t = line.trim();
        let ok = t.split_once(':').is_some_and(|(k, v)| {
            !k.is_empty()
                && k.bytes().all(|c| c.is_ascii_alphabetic())
                && (v.bytes().all(|c| c.is_ascii_digit())
                    || k.eq_ignore_ascii_case("version")
                    || k.eq_ignore_ascii_case("sourceurl"))
        });
        if !ok {
            break;
        }
        pos = line_end;
        header_end = pos;
    }
    let header = String::from_utf8_lossy(&data[..header_end]);
    let len = data.len();
    let sh = field(&header, "StartHTML").filter(|&s| s >= header_end && s < len);
    let eh = field(&header, "EndHTML")
        .filter(|&e| e <= len)
        .unwrap_or(len);
    let from = sh.unwrap_or(header_end);
    let to = if eh > from { eh } else { len };
    let whole = &data[from..to];
    let text = String::from_utf8_lossy(whole).into_owned();
    if text.contains(START) {
        return Ok(text);
    }
    let (sf, ef) = (
        field(&header, "StartFragment"),
        field(&header, "EndFragment"),
    );
    if let (Some(sf), Some(ef)) = (sf, ef)
        && sf >= from
        && ef >= sf
        && ef <= to
    {
        let (a, b) = (sf - from, ef - from);
        let mut out = String::with_capacity(text.len() + START.len() + END.len());
        out.push_str(&String::from_utf8_lossy(&whole[..a]));
        out.push_str(START);
        out.push_str(&String::from_utf8_lossy(&whole[a..b]));
        out.push_str(END);
        out.push_str(&String::from_utf8_lossy(&whole[b..]));
        return Ok(out);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_are_exact() {
        let w = wrap("<b>h\u{e9}</b>", Flavor::Text);
        let s = std::str::from_utf8(&w[..w.len() - 1]).unwrap();
        let get = |k: &str| field(s, k).unwrap();
        assert_eq!(get("EndHTML"), w.len() - 1);
        assert!(s[get("StartFragment")..].starts_with("<b>"));
        assert!(s[..get("EndFragment")].ends_with("</b>"));
        assert!(s[get("StartHTML")..].starts_with("<html"));
    }

    #[test]
    fn round_trip() {
        let w = wrap("<p>x</p>", Flavor::Sheet);
        let html = unwrap(&w).unwrap();
        assert!(html.contains("<!--StartFragment--><p>x</p><!--EndFragment-->"));
    }

    #[test]
    fn headerless_and_bad_offsets() {
        assert_eq!(unwrap(b"<p>a</p>").unwrap(), "<p>a</p>");
        let bad = b"Version:0.9\r\nStartHTML:0000000999\r\nEndHTML:0000009999\r\nStartFragment:0000000001\r\nEndFragment:0000000002\r\n<p>a</p>";
        assert!(unwrap(bad).unwrap().contains("<p>a</p>"));
    }

    #[test]
    fn fragment_markers_are_added_when_missing() {
        let body = "<html><body><p>x</p></body></html>";
        let h = |a: usize, b: usize, c: usize, d: usize| {
            format!(
                "Version:0.9\r\nStartHTML:{a:010}\r\nEndHTML:{b:010}\r\nStartFragment:{c:010}\r\nEndFragment:{d:010}\r\n"
            )
        };
        let hl = h(0, 0, 0, 0).len();
        let data = format!("{}{body}", h(hl, hl + body.len(), hl + 12, hl + 12 + 8));
        let out = unwrap(data.as_bytes()).unwrap();
        assert!(out.contains("<!--StartFragment--><p>x</p><!--EndFragment-->"));
    }

    #[test]
    fn too_large() {
        assert_eq!(
            unwrap(&vec![b'a'; MAX_HTML_BYTES + 1]),
            Err(ParseError::TooLarge)
        );
    }
}
