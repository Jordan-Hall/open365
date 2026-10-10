//! Document settings (`word/settings.xml`, ECMA-376 Part 1 §17.15) that affect layout.

use crate::DocxError;
use crate::xml::{self, RawXml, XmlElement};

/// Compatibility mode assumed when `w:compatSetting` `compatibilityMode` is absent. The
/// standard does not define it; Word treats such a file as written by Word 2007 (12).
pub const DEFAULT_COMPATIBILITY_MODE: u32 = 12;

/// Layout-relevant document settings. A missing settings part gives [`Settings::default`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `w:defaultTabStop`, §17.15.1.25, twips. Default 720 (half an inch).
    pub default_tab_stop_twips: u32,
    /// `w:compat/w:compatSetting` named `compatibilityMode` (§17.15.3). 11 is Word 2003/2007
    /// SP2 style, 12 Word 2007, 14 Word 2010, 15 Word 2013 and later.
    pub compatibility_mode: u32,
    /// `w:evenAndOddHeaders`, §17.10.1. Default false.
    pub even_and_odd_headers: bool,
    /// `w:autoHyphenation`, §17.15.1.10. Default false.
    pub auto_hyphenation: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_tab_stop_twips: 720,
            compatibility_mode: DEFAULT_COMPATIBILITY_MODE,
            even_and_odd_headers: false,
            auto_hyphenation: false,
        }
    }
}

impl Settings {
    pub fn default_tab_stop_pt(&self) -> f32 {
        self.default_tab_stop_twips as f32 / 20.0
    }

    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, DocxError> {
        let parsed = xml::parse(bytes)?;
        let root = &parsed.root;
        let mut out = Self::default();
        for c in root.children.iter().filter_map(RawXml::as_element) {
            match c.head.local_name() {
                "defaultTabStop" if c.head.is_w("defaultTabStop") => {
                    if let Some(v) = c
                        .head
                        .attr_w("val")
                        .and_then(|v| v.trim().parse::<u32>().ok())
                        .filter(|v| *v > 0)
                    {
                        out.default_tab_stop_twips = v;
                    }
                }
                "evenAndOddHeaders" if c.head.is_w("evenAndOddHeaders") => {
                    out.even_and_odd_headers = on_off(c);
                }
                "autoHyphenation" if c.head.is_w("autoHyphenation") => {
                    out.auto_hyphenation = on_off(c);
                }
                "compat" if c.head.is_w("compat") => {
                    if let Some(mode) = compat_mode(c) {
                        out.compatibility_mode = mode;
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }
}

fn on_off(e: &XmlElement) -> bool {
    !matches!(e.head.attr_w("val"), Some("0" | "false" | "off"))
}

fn compat_mode(compat: &XmlElement) -> Option<u32> {
    compat
        .children
        .iter()
        .filter_map(RawXml::as_element)
        .filter(|e| e.head.is_w("compatSetting"))
        .find(|e| e.head.attr_w("name") == Some("compatibilityMode"))
        .and_then(|e| e.head.attr_w("val"))
        .and_then(|v| v.trim().parse().ok())
}
