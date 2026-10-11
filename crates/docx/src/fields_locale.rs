//! Month, day and AM/PM names per language for date pictures.
//!
//! Word picks the names from the language of the field code's text (`w:lang/@w:val` on the
//! `w:instrText` run, then the paragraph and character styles, then the document defaults).
//! The built-in table in `fields_locale_data.rs` is generated from Word itself by
//! `tools/office/fields-oracle.ps1 -LocaleTable`; callers can supply more through
//! [`crate::DateNames`].

use crate::fields_locale_data::{DATA, TAGS};

/// Calendar names of one language (Gregorian).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleNames {
    pub month_full: [String; 12],
    pub month_abbr: [String; 12],
    /// Month names Word uses when the picture also holds a day number (genitive in Slavic,
    /// Baltic and some other languages); equal to `month_full` where the language has no
    /// separate form.
    pub month_full_genitive: [String; 12],
    /// Abbreviated counterpart of `month_full_genitive`.
    pub month_abbr_genitive: [String; 12],
    /// Sunday first.
    pub day_full: [String; 7],
    pub day_abbr: [String; 7],
    pub am: String,
    pub pm: String,
    /// Short date picture, e.g. `M/d/yyyy`.
    pub date_picture: String,
    /// Time picture, e.g. `h:mm AM/PM`.
    pub time_picture: String,
}

impl LocaleNames {
    /// English (United States) names.
    #[must_use]
    pub fn english() -> Self {
        Self::from_packed(DATA[0])
    }

    /// Names for a BCP 47 tag such as `de-DE` or `pt`. Matches the full tag first, then the
    /// language with its most common region.
    #[must_use]
    pub fn for_tag(tag: &str) -> Option<Self> {
        let wanted = tag.trim().replace('_', "-").to_ascii_lowercase();
        if wanted.is_empty() {
            return None;
        }
        let find = |needle: &str| TAGS.iter().find(|(t, _)| t.eq_ignore_ascii_case(needle));
        if let Some((_, idx)) = find(&wanted) {
            return Some(Self::from_packed(DATA[*idx]));
        }
        let language = wanted.split('-').next().unwrap_or(&wanted);
        if let Some((_, idx)) = find(language) {
            return Some(Self::from_packed(DATA[*idx]));
        }
        // Any entry of the same language.
        TAGS.iter()
            .find(|(t, _)| t.to_ascii_lowercase().split('-').next() == Some(language))
            .map(|(_, idx)| Self::from_packed(DATA[*idx]))
    }

    /// The picture Word uses for a date field without `\@`.
    #[must_use]
    pub fn default_date_picture(&self, kind: crate::fields::FieldKind) -> String {
        use crate::fields::FieldKind;
        match kind {
            FieldKind::Time => self.time_picture.clone(),
            FieldKind::Date => self.date_picture.clone(),
            _ => format!("{} {}", self.date_picture, self.time_picture),
        }
    }

    /// Unpacks `month_full|month_abbr|day_full|day_abbr|am|pm|date|time|genitive_full|genitive_abbr`
    /// records, separated by `|` with names separated by `,`. An empty genitive list means the
    /// language has no separate form.
    fn from_packed(packed: &str) -> Self {
        let mut parts = packed.split('|');
        let mut take = |n: usize| -> Vec<String> {
            let mut names: Vec<String> = parts
                .next()
                .unwrap_or_default()
                .split(',')
                .map(str::to_owned)
                .collect();
            names.resize(n, String::new());
            names
        };
        let month_full = take(12);
        let month_abbr = take(12);
        let day_full = take(7);
        let day_abbr = take(7);
        let am = take(1).remove(0);
        let pm = take(1).remove(0);
        let date_picture = take(1).remove(0);
        let time_picture = take(1).remove(0);
        let genitive_full = take(12);
        let genitive_abbr = take(12);
        let or_same = |list: Vec<String>, base: &[String]| -> Vec<String> {
            if list.iter().all(String::is_empty) {
                base.to_vec()
            } else {
                list
            }
        };
        Self {
            month_full_genitive: to_array(or_same(genitive_full, &month_full)),
            month_abbr_genitive: to_array(or_same(genitive_abbr, &month_abbr)),
            month_full: to_array(month_full),
            month_abbr: to_array(month_abbr),
            day_full: to_array(day_full),
            day_abbr: to_array(day_abbr),
            am,
            pm,
            date_picture,
            time_picture,
        }
    }
}

fn to_array<const N: usize>(names: Vec<String>) -> [String; N] {
    let mut iter = names.into_iter();
    std::array::from_fn(|_| iter.next().unwrap_or_default())
}
