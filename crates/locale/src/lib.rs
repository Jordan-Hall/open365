//! Locale data for Office semantics (ADR-011).
//!
//! A [`Locale`] is built from a BCP-47 tag or an Office LCID and carries no global state: the
//! document, the user and the system locale are separate inputs, and callers pass `&Locale`.
//! Data comes from Unicode CLDR through ICU4X, baked into the binary at build time (no runtime file
//! I/O, so the crate stays wasm- and mobile-portable). Where Excel measurably differs from CLDR the
//! [`office_overrides`] table wins.
//!
//! Most accessors compute their answer on demand from the compiled data; the ones returning large
//! structures ([`Locale::number_format`], [`Locale::calendar_names`], [`Locale::date_time_patterns`])
//! cost a fraction of a millisecond to a few milliseconds, so callers should cache the result per
//! document or per sheet.

mod builtin_numfmt;
mod builtin_numfmt_nls;
mod calendar;
mod casing;
mod currency;
mod datetime;
mod lcid_table;
mod numbers;
pub mod office_overrides;

pub use builtin_numfmt::{
    builtin_number_format_in, builtin_number_format_lcids, nls_pattern_to_excel,
    resolve_number_format_lcid,
};
pub use calendar::CalendarKind;
pub use casing::{CollationStrength, Collator};
pub use currency::CurrencyFormat;
pub use datetime::{CalendarNames, DatePatterns, Weekday};
pub use numbers::NumberFormat;

use icu_locale::{Locale as IcuLocale, LocaleDirectionality};

/// Text direction of a locale's default script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDirection {
    LeftToRight,
    RightToLeft,
}

/// Errors from building a [`Locale`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocaleError {
    /// The string is not a valid BCP-47 language tag.
    InvalidTag(String),
    /// The LCID is not in the [MS-LCID] table.
    UnknownLcid(u32),
    /// ICU4X has no compiled data for a required category.
    Data(String),
}

impl core::fmt::Display for LocaleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidTag(tag) => write!(f, "invalid BCP-47 tag: {tag}"),
            Self::UnknownLcid(lcid) => write!(f, "unknown LCID: 0x{lcid:04X}"),
            Self::Data(what) => write!(f, "missing locale data: {what}"),
        }
    }
}

impl core::error::Error for LocaleError {}

/// A locale: BCP-47 identity plus the Office LCID when one exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locale {
    tag: String,
    lcid: Option<u32>,
    icu: IcuLocale,
}

impl Locale {
    /// Builds a locale from a BCP-47 tag such as `de-DE` or `sr-Cyrl-RS`.
    pub fn from_bcp47(tag: &str) -> Result<Self, LocaleError> {
        let icu: IcuLocale = tag
            .parse()
            .map_err(|_| LocaleError::InvalidTag(tag.to_owned()))?;
        let lcid = lcid_for(&icu);
        Ok(Self {
            tag: icu.to_string(),
            lcid,
            icu,
        })
    }

    /// Builds a locale from an Office / Windows LCID. Sort-order bits (the high word, as in
    /// `0x10407`) are ignored; use [`Locale::collator`] with the tag extension for alternate sorts.
    pub fn from_lcid(lcid: u32) -> Result<Self, LocaleError> {
        let base = lcid & 0xFFFF;
        let tag = lcid_table::LCID_TABLE
            .iter()
            .find(|(id, _)| *id == base)
            .map(|(_, tag)| *tag)
            .ok_or(LocaleError::UnknownLcid(lcid))?;
        let icu: IcuLocale = tag
            .parse()
            .map_err(|_| LocaleError::InvalidTag(tag.to_owned()))?;
        Ok(Self {
            tag: tag.to_owned(),
            lcid: Some(base),
            icu,
        })
    }

    /// The fallback locale used when nothing else is known: `en-US` (LCID 0x0409).
    pub fn en_us() -> Self {
        Self {
            tag: "en-US".to_owned(),
            lcid: Some(0x0409),
            icu: icu_locale::locale!("en-US"),
        }
    }

    /// Every `(LCID, BCP-47 tag)` pair in the table, ordered by LCID.
    pub fn all_lcids() -> impl Iterator<Item = (u32, &'static str)> {
        lcid_table::LCID_TABLE.iter().copied()
    }

    /// The BCP-47 tag.
    pub fn bcp47(&self) -> &str {
        &self.tag
    }

    /// The Office LCID, when the tag maps to one.
    pub fn lcid(&self) -> Option<u32> {
        self.lcid
    }

    /// The underlying ICU4X locale, for callers that need other ICU4X services.
    pub fn icu(&self) -> &IcuLocale {
        &self.icu
    }

    /// Text direction of the locale's script.
    pub fn text_direction(&self) -> TextDirection {
        if LocaleDirectionality::new_common().is_right_to_left(&self.icu.id) {
            TextDirection::RightToLeft
        } else {
            TextDirection::LeftToRight
        }
    }
}

/// Finds the LCID for a parsed locale, trying the exact tag then dropping variants and extensions.
fn lcid_for(icu: &IcuLocale) -> Option<u32> {
    let id = &icu.id;
    let mut candidates = Vec::new();
    candidates.push(id.to_string());
    let mut no_variants = id.clone();
    no_variants.variants.clear();
    candidates.push(no_variants.to_string());
    let lang = id.language.as_str();
    if let Some(region) = id.region {
        candidates.push(format!("{lang}-{}", region.as_str()));
    }
    candidates.push(lang.to_owned());
    candidates.into_iter().find_map(|candidate| {
        lcid_table::LCID_TABLE
            .iter()
            .find(|(_, tag)| tag.eq_ignore_ascii_case(&candidate))
            .map(|(lcid, _)| *lcid)
    })
}

#[cfg(test)]
mod tests;
