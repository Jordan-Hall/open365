//! A tolerant HTML tokenizer for clipboard fragments. It builds no DOM, runs no script, and
//! reads no URL: tags, attributes, text and `<style>` blocks come out as tokens, bounded by
//! [`crate::limits`].

use crate::limits::{MAX_HTML_BYTES, MAX_TOKENS};
use crate::model::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Open {
        name: String,
        attrs: Vec<(String, String)>,
        self_close: bool,
    },
    Close(String),
    Text(String),
    Style(String),
    /// `<!--StartFragment-->`
    FragStart,
    /// `<!--EndFragment-->`
    FragEnd,
}

pub(crate) const VOID: &[&str] = &[
    "br", "img", "hr", "meta", "link", "col", "input", "area", "base", "wbr", "param", "source",
    "track", "embed",
];

pub(crate) fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "copy" => '\u{a9}',
        "reg" => '\u{ae}',
        "trade" => '\u{2122}',
        "hellip" => '\u{2026}',
        "ndash" => '\u{2013}',
        "mdash" => '\u{2014}',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201c}',
        "rdquo" => '\u{201d}',
        "bull" => '\u{2022}',
        "middot" => '\u{b7}',
        "euro" => '\u{20ac}',
        "pound" => '\u{a3}',
        "yen" => '\u{a5}',
        "cent" => '\u{a2}',
        "deg" => '\u{b0}',
        "plusmn" => '\u{b1}',
        "times" => '\u{d7}',
        "divide" => '\u{f7}',
        "sect" => '\u{a7}',
        "para" => '\u{b6}',
        "laquo" => '\u{ab}',
        "raquo" => '\u{bb}',
        "emsp" => '\u{2003}',
        "ensp" => '\u{2002}',
        "thinsp" => '\u{2009}',
        "shy" => '\u{ad}',
        _ => return None,
    })
}

/// Decodes character references. Unknown or malformed ones stay as typed.
pub(crate) fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest.char_indices().take(12).find(|(_, c)| *c == ';');
        let decoded = end.and_then(|(e, _)| {
            let name = &rest[1..e];
            let ch = if let Some(num) = name.strip_prefix('#') {
                let v = if let Some(h) = num.strip_prefix(['x', 'X']) {
                    u32::from_str_radix(h, 16).ok()
                } else {
                    num.parse().ok()
                };
                v.and_then(char::from_u32)
            } else {
                entity(name)
            };
            ch.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b':' | b'-' | b'_' | b'.')
}

fn find_ci(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len())
        .find(|&i| hay[i..i + needle.len()].eq_ignore_ascii_case(needle))
}

pub(crate) fn tokenize(input: &str) -> Result<Vec<Tok>, ParseError> {
    if input.len() > MAX_HTML_BYTES {
        return Err(ParseError::TooLarge);
    }
    let b = input.as_bytes();
    let n = b.len();
    let mut toks: Vec<Tok> = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    macro_rules! push {
        ($t:expr) => {{
            if toks.len() >= MAX_TOKENS {
                return Err(ParseError::TooManyItems);
            }
            toks.push($t);
        }};
    }
    macro_rules! flush_text {
        ($upto:expr) => {{
            if text_start < $upto {
                push!(Tok::Text(decode_entities(&input[text_start..$upto])));
            }
        }};
    }
    while i < n {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let next = b.get(i + 1).copied().unwrap_or(0);
        if input[i..].starts_with("<!--") {
            flush_text!(i);
            let close = input[i + 4..].find("-->").map(|e| i + 4 + e);
            let end = close.map_or(n, |c| c + 3);
            let body = input[i + 4..close.unwrap_or(n)].trim();
            if body == "StartFragment" {
                push!(Tok::FragStart);
            } else if body == "EndFragment" {
                push!(Tok::FragEnd);
            }
            i = end;
            text_start = i;
        } else if next.is_ascii_alphabetic() {
            flush_text!(i);
            let mut j = i + 1;
            while j < n && is_name_char(b[j]) {
                j += 1;
            }
            let name = input[i + 1..j].to_ascii_lowercase();
            let mut attrs = Vec::new();
            let mut self_close = false;
            loop {
                while j < n && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= n {
                    break;
                }
                if b[j] == b'>' {
                    j += 1;
                    break;
                }
                if b[j] == b'/' {
                    if b.get(j + 1) == Some(&b'>') {
                        self_close = true;
                        j += 2;
                        break;
                    }
                    j += 1;
                    continue;
                }
                let ks = j;
                while j < n && !b[j].is_ascii_whitespace() && !matches!(b[j], b'=' | b'>' | b'/') {
                    j += 1;
                }
                if j == ks {
                    j += 1;
                    continue;
                }
                let key = input[ks..j].to_ascii_lowercase();
                while j < n && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                let mut val = String::new();
                if j < n && b[j] == b'=' {
                    j += 1;
                    while j < n && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < n && (b[j] == b'"' || b[j] == b'\'') {
                        let q = b[j];
                        let vs = j + 1;
                        let ve = b[vs..].iter().position(|&c| c == q).map_or(n, |p| vs + p);
                        val = decode_entities(&input[vs..ve]);
                        j = (ve + 1).min(n);
                    } else {
                        let vs = j;
                        while j < n && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                            j += 1;
                        }
                        val = decode_entities(&input[vs..j]);
                    }
                }
                if attrs.len() < 64 {
                    attrs.push((key, val));
                }
            }
            if matches!(name.as_str(), "script" | "style" | "textarea" | "title") && !self_close {
                let close = format!("</{name}");
                let ce = find_ci(b, j, close.as_bytes()).unwrap_or(n);
                if name == "style" {
                    push!(Tok::Style(input[j..ce].to_owned()));
                }
                let gt = input[ce..].find('>').map_or(n, |g| ce + g + 1);
                i = gt;
            } else {
                push!(Tok::Open {
                    name,
                    attrs,
                    self_close,
                });
                i = j;
            }
            text_start = i;
        } else if next == b'/' && b.get(i + 2).is_some_and(u8::is_ascii_alphabetic) {
            flush_text!(i);
            let mut j = i + 2;
            while j < n && is_name_char(b[j]) {
                j += 1;
            }
            let name = input[i + 2..j].to_ascii_lowercase();
            let gt = input[j..].find('>').map_or(n, |g| j + g + 1);
            push!(Tok::Close(name));
            i = gt;
            text_start = i;
        } else if next == b'!' || next == b'?' {
            flush_text!(i);
            let gt = input[i..].find('>').map_or(n, |g| i + g + 1);
            i = gt;
            text_start = i;
        } else {
            i += 1;
        }
    }
    flush_text!(n);
    Ok(toks)
}

/// All `<style>` text of a token list.
pub(crate) fn style_text(toks: &[Tok]) -> impl Iterator<Item = &str> {
    toks.iter().filter_map(|t| match t {
        Tok::Style(s) => Some(s.as_str()),
        _ => None,
    })
}

/// The tokens between the fragment markers, or all of them when there are none.
pub(crate) fn fragment(toks: &[Tok]) -> &[Tok] {
    let start = toks.iter().position(|t| *t == Tok::FragStart);
    let Some(s) = start else { return toks };
    let end = toks[s..]
        .iter()
        .position(|t| *t == Tok::FragEnd)
        .map_or(toks.len(), |e| s + e);
    &toks[s + 1..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_attributes_entities() {
        let t = tokenize("<P class=a STYLE='x:y'>a &amp; b&#65;&bogus;</p>").unwrap();
        assert_eq!(
            t[0],
            Tok::Open {
                name: "p".into(),
                attrs: vec![("class".into(), "a".into()), ("style".into(), "x:y".into())],
                self_close: false
            }
        );
        assert_eq!(t[1], Tok::Text("a & bA&bogus;".into()));
        assert_eq!(t[2], Tok::Close("p".into()));
    }

    #[test]
    fn script_is_dropped_style_is_kept() {
        let t = tokenize("<script>alert('<b>')</script><style>p{}</style>x").unwrap();
        assert_eq!(t, vec![Tok::Style("p{}".into()), Tok::Text("x".into())]);
    }

    #[test]
    fn comments_and_conditionals_are_skipped() {
        let t = tokenize("a<!--[if gte mso 9]><xml>z</xml><![endif]-->b<![if !vml]>c").unwrap();
        let text: String = t
            .iter()
            .filter_map(|t| match t {
                Tok::Text(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "abc");
    }

    #[test]
    fn markers_select_the_fragment() {
        let t = tokenize("x<!--StartFragment-->y<!--EndFragment-->z").unwrap();
        assert_eq!(fragment(&t), &[Tok::Text("y".into())]);
    }

    #[test]
    fn truncated_input_does_not_panic() {
        for s in [
            "<", "<a", "<a b='", "<a b=\"c", "</", "<!--", "<style>", "&#x", "<a/",
        ] {
            let _ = tokenize(s).unwrap();
        }
    }

    #[test]
    fn token_limit() {
        let big = "<b>".repeat(MAX_TOKENS + 1);
        assert_eq!(tokenize(&big), Err(ParseError::TooManyItems));
    }
}
