//! Case classification and locale-sensitive case mapping.

/// Capitalisation pattern of a word (Hunspell's NOCAP, INITCAP, ALLCAP, HUHCAP, HUHINITCAP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaseKind {
    /// No upper-case letters.
    Lower,
    /// Exactly the first letter is upper case.
    Title,
    /// Two or more cased letters, all upper case.
    Upper,
    /// Mixed case that starts lower case or uncased, such as `iPhone`.
    MixedLower,
    /// Mixed case that starts upper case, such as `McDonald`.
    MixedTitle,
}

/// Case mapping with the Turkic dotted/dotless `i` rules where the dictionary language needs them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Caser {
    turkic: bool,
}

impl Caser {
    pub(crate) fn for_lang(lang: &str) -> Self {
        let primary = lang
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        Self {
            turkic: matches!(primary.as_str(), "tr" | "az" | "crh" | "kk" | "tt" | "ba"),
        }
    }

    pub(crate) fn lower(&self, text: &str) -> String {
        if self.turkic {
            text.chars()
                .flat_map(|c| match c {
                    'I' => vec!['\u{131}'],
                    '\u{130}' => vec!['i'],
                    other => other.to_lowercase().collect(),
                })
                .collect()
        } else {
            text.to_lowercase()
        }
    }

    pub(crate) fn upper(&self, text: &str) -> String {
        if self.turkic {
            text.chars()
                .flat_map(|c| match c {
                    'i' => vec!['\u{130}'],
                    '\u{131}' => vec!['I'],
                    other => other.to_uppercase().collect(),
                })
                .collect()
        } else {
            text.to_uppercase()
        }
    }

    /// Upper-cases the first character and lower-cases the rest.
    pub(crate) fn title(&self, text: &str) -> String {
        let mut chars = text.chars();
        match chars.next() {
            Some(first) => {
                let mut out = self.upper(first.encode_utf8(&mut [0; 4]));
                out.push_str(&self.lower(chars.as_str()));
                out
            }
            None => String::new(),
        }
    }

    /// Upper-cases only the first character.
    pub(crate) fn capitalize_first(&self, text: &str) -> String {
        let mut chars = text.chars();
        match chars.next() {
            Some(first) => {
                let mut out = self.upper(first.encode_utf8(&mut [0; 4]));
                out.push_str(chars.as_str());
                out
            }
            None => String::new(),
        }
    }

    pub(crate) fn kind(&self, text: &str) -> CaseKind {
        let first_upper = text.chars().next().is_some_and(char::is_uppercase);
        let mut upper = 0usize;
        let mut lower = 0usize;
        for c in text.chars() {
            if c.is_uppercase() {
                upper += 1;
            } else if c.is_lowercase() {
                lower += 1;
            }
        }
        if upper == 0 {
            CaseKind::Lower
        } else if lower == 0 {
            if upper == 1 && first_upper {
                CaseKind::Title
            } else if upper >= 2 {
                CaseKind::Upper
            } else {
                CaseKind::MixedLower
            }
        } else if first_upper && upper == 1 {
            CaseKind::Title
        } else if first_upper {
            CaseKind::MixedTitle
        } else {
            CaseKind::MixedLower
        }
    }
}

/// True when the text has an upper-case letter and no lower-case letter.
pub(crate) fn is_all_caps(text: &str) -> bool {
    text.chars().any(char::is_uppercase) && !text.chars().any(char::is_lowercase)
}
