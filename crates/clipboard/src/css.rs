//! A small, forgiving CSS reader for clipboard HTML: inline declarations, colours, lengths and
//! the `.class` / `tag` rules of a style block. It never follows `url(...)` or `@import`: no
//! network or file access is possible from here.

use crate::limits::MAX_CSS_RULES;
use crate::model::Rgb;

/// Splits `a:b;c:d` into lowercase names and trimmed values. `!important` is dropped.
pub fn declarations(style: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut start = 0;
    let mut pieces = Vec::new();
    for (i, c) in style.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ';' if depth == 0 => {
                    pieces.push(&style[start..i]);
                    start = i + 1;
                }
                _ => {}
            },
        }
    }
    pieces.push(&style[start..]);
    for p in pieces {
        if let Some((k, v)) = p.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let mut v = v.trim();
            if let Some(idx) = v.to_ascii_lowercase().find("!important") {
                v = v[..idx].trim();
            }
            if !k.is_empty() && !v.is_empty() {
                out.push((k, v.to_owned()));
            }
        }
    }
    out
}

fn named_color(name: &str) -> Option<Rgb> {
    Some(match name {
        "black" | "windowtext" | "buttontext" => Rgb::new(0, 0, 0),
        "white" | "window" => Rgb::new(255, 255, 255),
        "red" => Rgb::new(255, 0, 0),
        "green" => Rgb::new(0, 128, 0),
        "blue" => Rgb::new(0, 0, 255),
        "yellow" => Rgb::new(255, 255, 0),
        "gray" | "grey" => Rgb::new(128, 128, 128),
        "silver" => Rgb::new(192, 192, 192),
        "maroon" => Rgb::new(128, 0, 0),
        "navy" => Rgb::new(0, 0, 128),
        "purple" => Rgb::new(128, 0, 128),
        "teal" => Rgb::new(0, 128, 128),
        "olive" => Rgb::new(128, 128, 0),
        "lime" => Rgb::new(0, 255, 0),
        "aqua" | "cyan" => Rgb::new(0, 255, 255),
        "fuchsia" | "magenta" => Rgb::new(255, 0, 255),
        "orange" => Rgb::new(255, 165, 0),
        "darkgray" | "darkgrey" => Rgb::new(169, 169, 169),
        "lightgray" | "lightgrey" => Rgb::new(211, 211, 211),
        "darkblue" => Rgb::new(0, 0, 139),
        "darkred" => Rgb::new(139, 0, 0),
        "darkgreen" => Rgb::new(0, 100, 0),
        _ => return None,
    })
}

/// A CSS colour: `#rgb`, `#rrggbb`, `rgb(...)` or a common name. `None` for automatic,
/// transparent, inherited or unknown values.
pub fn parse_color(v: &str) -> Option<Rgb> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(h) = v.strip_prefix('#') {
        let h = h.split_whitespace().next()?;
        return match h.len() {
            3 => {
                let n = u16::from_str_radix(h, 16).ok()?;
                let e = |s: u16| ((s & 0xF) * 17) as u8;
                Some(Rgb::new(e(n >> 8), e(n >> 4), e(n)))
            }
            6 => {
                let n = u32::from_str_radix(h, 16).ok()?;
                Some(Rgb::new((n >> 16) as u8, (n >> 8) as u8, n as u8))
            }
            _ => None,
        };
    }
    if let Some(rest) = v.strip_prefix("rgb") {
        let inner = rest
            .trim_start_matches('a')
            .trim()
            .strip_prefix('(')?
            .split(')')
            .next()?;
        let parts: Vec<&str> = inner
            .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() < 3 {
            return None;
        }
        let ch = |s: &str| -> Option<u8> {
            if let Some(p) = s.strip_suffix('%') {
                let f: f32 = p.parse().ok()?;
                Some((f.clamp(0.0, 100.0) * 2.55).round() as u8)
            } else {
                let f: f32 = s.parse().ok()?;
                Some(f.clamp(0.0, 255.0).round() as u8)
            }
        };
        return Some(Rgb::new(ch(parts[0])?, ch(parts[1])?, ch(parts[2])?));
    }
    named_color(v.split_whitespace().next()?)
}

/// The first token of a value that is a colour (for shorthands like `background`).
pub fn first_color(v: &str) -> Option<Rgb> {
    let mut depth = 0;
    let mut tok = String::new();
    let mut toks = Vec::new();
    for c in v.chars() {
        match c {
            '(' => {
                depth += 1;
                tok.push(c);
            }
            ')' => {
                depth -= 1;
                tok.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !tok.is_empty() {
                    toks.push(std::mem::take(&mut tok));
                }
            }
            c => tok.push(c),
        }
    }
    if !tok.is_empty() {
        toks.push(tok);
    }
    toks.iter().find_map(|t| parse_color(t))
}

/// A length in points (`pt`, `px`, `in`, `cm`, `mm`, `pc`). Relative units give `None`.
pub fn length_pt(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let split = v
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(v.len());
    let (num, unit) = v.split_at(split);
    let n: f32 = num.parse().ok()?;
    let f = match unit.trim() {
        "pt" => 1.0,
        "px" => 0.75,
        "in" => 72.0,
        "cm" => 28.346_457,
        "mm" => 2.834_645_7,
        "pc" => 12.0,
        _ => return None,
    };
    let r = n * f;
    r.is_finite().then_some(r)
}

/// The first family of a `font-family` list, without quotes; generic families give `None`.
pub fn first_family(v: &str) -> Option<String> {
    for part in v.split(',') {
        let name = part.trim().trim_matches(['"', '\'']).trim();
        if name.is_empty() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" | "system-ui" | "inherit"
        ) {
            continue;
        }
        return Some(name.to_owned());
    }
    None
}

#[derive(Debug, Clone, Default)]
struct Rule {
    tag: Option<String>,
    class: Option<String>,
    decls: Vec<(String, String)>,
}

/// The `tag` and `.class` rules of the style blocks of a document.
#[derive(Debug, Clone, Default)]
pub struct StyleSheet {
    rules: Vec<Rule>,
}

impl StyleSheet {
    /// Adds the rules of one style block. At-rules and complex selectors are skipped.
    pub fn add(&mut self, css: &str) {
        let mut text = css.to_owned();
        while let Some(a) = text.find("/*") {
            let end = text[a + 2..]
                .find("*/")
                .map_or(text.len(), |e| a + 2 + e + 2);
            text.replace_range(a..end, " ");
        }
        let text = text.replace("<!--", " ").replace("-->", " ");
        let mut rest = text.as_str();
        while let Some(open) = rest.find('{') {
            if self.rules.len() >= MAX_CSS_RULES {
                return;
            }
            let selectors = rest[..open].trim();
            // Find the matching close brace (at-rules nest).
            let mut depth = 1;
            let mut close = rest.len();
            for (i, c) in rest[open + 1..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            close = open + 1 + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let body = &rest[open + 1..close.min(rest.len())];
            rest = rest.get(close + 1..).unwrap_or("");
            if selectors.starts_with('@') {
                continue;
            }
            let decls = declarations(body);
            for sel in selectors.split(',') {
                let sel = sel.trim();
                if sel.is_empty() || sel.contains([' ', '>', '+', '~', '#', '[', ':']) {
                    continue;
                }
                let (tag, class) = match sel.split_once('.') {
                    Some((t, c)) => ((!t.is_empty()).then(|| t.to_ascii_lowercase()), Some(c)),
                    None => (Some(sel.to_ascii_lowercase()), None),
                };
                if class.is_some_and(|c| c.contains('.')) || tag.as_deref() == Some("*") {
                    continue;
                }
                self.rules.push(Rule {
                    tag,
                    class: class.map(str::to_owned),
                    decls: decls.clone(),
                });
            }
        }
    }

    /// Declarations that apply to an element, lowest priority first: tag rules, then class
    /// rules (tag-qualified ones last).
    pub fn matching(&self, tag: &str, classes: &[&str]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        // Specificity: tag rules, then class rules, then tag-qualified class rules.
        for rank in 0..3 {
            for r in &self.rules {
                let hit = match (&r.class, rank) {
                    (None, 0) => r.tag.as_deref() == Some(tag),
                    (Some(c), 1) => r.tag.is_none() && classes.iter().any(|k| k == c),
                    (Some(c), 2) => r.tag.as_deref() == Some(tag) && classes.iter().any(|k| k == c),
                    _ => false,
                };
                if hit {
                    out.extend(r.decls.iter().cloned());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(parse_color("#fa0"), Some(Rgb::new(255, 170, 0)));
        assert_eq!(parse_color("#FFFF00"), Some(Rgb::new(255, 255, 0)));
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(Rgb::new(1, 2, 3)));
        assert_eq!(parse_color("windowtext"), Some(Rgb::BLACK));
        assert_eq!(parse_color("transparent"), None);
        assert_eq!(
            first_color("#ff0000 url(a;b) no-repeat"),
            Some(Rgb::new(255, 0, 0))
        );
    }

    #[test]
    fn declarations_respect_quotes_and_parens() {
        let d = declarations("font-family:'A;B', serif; background:url(x;y) red !important");
        assert_eq!(d[0].0, "font-family");
        assert_eq!(d[0].1, "'A;B', serif");
        assert_eq!(d[1].1, "url(x;y) red");
    }

    #[test]
    fn sheet_rules_cascade() {
        let mut s = StyleSheet::default();
        s.add("<!-- td {color:red} .xl65 {color:blue} td.xl65 {font-weight:700} @page {margin:1in} -->");
        let m = s.matching("td", &["xl65"]);
        assert_eq!(m[0].1, "red");
        assert_eq!(m[1].1, "blue");
        assert_eq!(m[2].1, "700");
        assert!(s.matching("p", &["xl65"]).len() == 1);
    }

    #[test]
    fn lengths() {
        assert_eq!(length_pt(".5pt"), Some(0.5));
        assert_eq!(length_pt("16px"), Some(12.0));
        assert_eq!(length_pt("2em"), None);
    }
}
