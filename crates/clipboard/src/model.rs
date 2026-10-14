//! The neutral document-fragment model the Writer formats convert to and from.

use serde::{Deserialize, Serialize};

use crate::limits::{MAX_CHARS, MAX_FONT_NAME, MAX_PARAS};

/// An opaque 8-bit colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Self = Self::new(0, 0, 0);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `#RRGGBB`.
    pub fn css(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// Character formatting. `None` means "inherit".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CharFmt {
    pub font: Option<String>,
    /// Size in points.
    pub size_pt: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub color: Option<Rgb>,
    pub highlight: Option<Rgb>,
}

impl CharFmt {
    /// Clamps values to what a document can hold and removes control characters from names.
    pub fn sanitize(&mut self) {
        if let Some(f) = &mut self.font {
            let clean: String = f
                .chars()
                .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ';' | '{' | '}' | '\\'))
                .take(MAX_FONT_NAME)
                .collect();
            let clean = clean.trim().to_owned();
            self.font = (!clean.is_empty()).then_some(clean);
        }
        if let Some(s) = &mut self.size_pt {
            *s = if s.is_finite() {
                s.clamp(1.0, 1638.0)
            } else {
                11.0
            };
        }
        if self.superscript && self.subscript {
            self.subscript = false;
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListKind {
    Bullet,
    Numbered,
}

/// Text with one set of character formatting. `\n` is a line break inside the paragraph and
/// `\t` a tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    pub fmt: CharFmt,
    /// A hyperlink target; only `http`, `https` and `mailto` survive parsing.
    pub link: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Para {
    pub runs: Vec<Run>,
    pub align: Align,
    /// Heading level 1 to 6.
    pub heading: Option<u8>,
    pub list: Option<ListKind>,
}

impl Para {
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

/// A fragment of a document: paragraphs of formatted runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RichText {
    pub paras: Vec<Para>,
}

/// A link target is kept only when it uses a scheme that opens no local resource.
pub fn safe_link(url: &str) -> Option<String> {
    let t = url.trim();
    let lower = t.to_ascii_lowercase();
    let ok = ["http://", "https://", "mailto:"]
        .iter()
        .any(|p| lower.starts_with(p));
    (ok && !t
        .chars()
        .any(|c| c.is_control() || c == '"' || c == '<' || c == '>')
        && t.len() <= 2048)
        .then(|| t.to_owned())
}

impl RichText {
    /// Plain text: paragraphs joined with `\n`.
    pub fn plain(&self) -> String {
        let mut out = String::new();
        for (i, p) in self.paras.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&p.text());
        }
        out
    }

    /// Unformatted paragraphs from plain text.
    pub fn from_plain(text: &str) -> Self {
        let norm = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut paras: Vec<Para> = norm
            .split('\n')
            .take(MAX_PARAS)
            .map(|l| Para {
                runs: if l.is_empty() {
                    Vec::new()
                } else {
                    vec![Run {
                        text: l
                            .chars()
                            .filter(|c| !c.is_control() || *c == '\t')
                            .collect(),
                        ..Run::default()
                    }]
                },
                ..Para::default()
            })
            .collect();
        if paras.len() > 1 && norm.ends_with('\n') {
            paras.pop();
        }
        Self { paras }
    }

    pub fn char_count(&self) -> usize {
        self.paras
            .iter()
            .flat_map(|p| p.runs.iter())
            .map(|r| r.text.chars().count())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.char_count() == 0 && self.paras.len() <= 1
    }

    /// Checks and repairs data that did not come from our own code (the private format can be
    /// written by any program). Returns false when it is beyond the limits.
    pub fn sanitize(&mut self) -> bool {
        if self.paras.len() > MAX_PARAS || self.char_count() > MAX_CHARS {
            return false;
        }
        for p in &mut self.paras {
            if let Some(h) = p.heading
                && !(1..=6).contains(&h)
            {
                p.heading = None;
            }
            for r in &mut p.runs {
                r.fmt.sanitize();
                r.text
                    .retain(|c| !c.is_control() || matches!(c, '\t' | '\n'));
                r.link = r.link.as_deref().and_then(safe_link);
            }
            p.runs.retain(|r| !r.text.is_empty());
        }
        true
    }

    /// Merges neighbouring runs that carry equal formatting.
    pub fn normalize(&mut self) {
        for p in &mut self.paras {
            let mut out: Vec<Run> = Vec::with_capacity(p.runs.len());
            for r in p.runs.drain(..) {
                if r.text.is_empty() {
                    continue;
                }
                match out.last_mut() {
                    Some(last) if last.fmt == r.fmt && last.link == r.link => {
                        last.text.push_str(&r.text)
                    }
                    _ => out.push(r),
                }
            }
            p.runs = out;
        }
    }
}

/// Why a clipboard payload was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    TooLarge,
    TooDeep,
    TooManyItems,
    Malformed(&'static str),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => f.write_str("clipboard data is too large"),
            Self::TooDeep => f.write_str("clipboard data is nested too deeply"),
            Self::TooManyItems => f.write_str("clipboard data has too many items"),
            Self::Malformed(m) => write!(f, "clipboard data is malformed: {m}"),
        }
    }
}

impl std::error::Error for ParseError {}
