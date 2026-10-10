//! Locale-aware case mapping and collation.

use core::cmp::Ordering;

use icu_casemap::CaseMapper;
use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::{Collator as IcuCollator, CollatorBorrowed, CollatorPreferences};

use crate::{Locale, LocaleError};

/// Comparison strength for [`Locale::collator_with`].
pub use icu_collator::options::Strength as CollationStrength;

/// A collator for one locale. Cheap to compare with; build once and reuse.
#[derive(Debug)]
pub struct Collator {
    inner: CollatorBorrowed<'static>,
}

impl Collator {
    /// Compares two strings in the locale's sort order.
    pub fn compare(&self, left: &str, right: &str) -> Ordering {
        self.inner.compare(left, right)
    }
}

impl Locale {
    /// Upper-cases `text` with this locale's rules (Turkish `i` becomes `İ`, German `ß` becomes `SS`).
    pub fn to_uppercase(&self, text: &str) -> String {
        CaseMapper::new()
            .uppercase_to_string(text, &self.icu.id)
            .into_owned()
    }

    /// Lower-cases `text` with this locale's rules (Turkish `I` becomes `ı`).
    pub fn to_lowercase(&self, text: &str) -> String {
        CaseMapper::new()
            .lowercase_to_string(text, &self.icu.id)
            .into_owned()
    }

    /// Collator at full (tertiary) strength: case and accent differences order strings.
    pub fn collator(&self) -> Result<Collator, LocaleError> {
        self.collator_with(Strength::Tertiary)
    }

    /// Collator at a chosen strength. Excel's default compare ignores case, which is
    /// [`CollationStrength::Secondary`].
    pub fn collator_with(&self, strength: Strength) -> Result<Collator, LocaleError> {
        let mut options = CollatorOptions::default();
        options.strength = Some(strength);
        let inner = IcuCollator::try_new(CollatorPreferences::from(&self.icu), options)
            .map_err(|e| LocaleError::Data(format!("collator: {e}")))?;
        Ok(Collator { inner })
    }
}
