//! Number symbols: separators, grouping, signs, digits, percent and list separator.

use icu_decimal::input::{Decimal, SignDisplay};
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_decimal::{DecimalFormatter, DecimalFormatterPreferences};
use icu_experimental::dimension::percent::formatter::{
    PercentFormatter, PercentFormatterPreferences,
};

use crate::{CurrencyFormat, Locale, LocaleError, office_overrides};

/// Number conventions of a locale, as Excel's number formatter needs them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberFormat {
    /// Decimal separator, for example `.` or `,`.
    pub decimal_separator: String,
    /// Thousands (group) separator; may be a no-break space.
    pub group_separator: String,
    /// Size of the lowest digit group (3 almost everywhere); 0 when the locale does not group.
    pub primary_grouping: u8,
    /// Size of the higher groups (2 in en-IN, otherwise equal to `primary_grouping`).
    pub secondary_grouping: u8,
    /// Minus sign, bidi controls removed and U+2212 normalised to `-` (see `office_overrides`).
    pub minus_sign: String,
    /// Plus sign with bidi controls removed.
    pub plus_sign: String,
    /// Percent sign with bidi controls removed (`%` or `٪`).
    pub percent_sign: String,
    /// Percent pattern with `#` for the number, for example `#%` or `# %`.
    pub percent_pattern: String,
    /// Native digits 0 to 9 of the locale's default numbering system.
    pub digits: [char; 10],
    /// Argument separator Excel uses in formulas typed in this locale (`,` or `;`).
    pub list_separator: String,
    /// Currency conventions for the locale's region.
    pub currency: CurrencyFormat,
}

pub(crate) fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

pub(crate) fn strip_bidi(s: &str) -> String {
    s.chars().filter(|c| !is_bidi_control(*c)).collect()
}

pub(crate) fn parse_decimal(text: &str) -> Result<Decimal, LocaleError> {
    Decimal::try_from_str(text).map_err(|_| LocaleError::Data(format!("decimal {text}")))
}

/// Formats single numbers with the locale's own decimal formatter.
pub(crate) struct Digits {
    formatter: DecimalFormatter,
    pub digits: [char; 10],
}

impl Digits {
    pub(crate) fn new(locale: &Locale, always_group: bool) -> Result<Self, LocaleError> {
        let prefs = DecimalFormatterPreferences::from(&locale.icu);
        let mut options = DecimalFormatterOptions::default();
        if always_group {
            options.grouping_strategy = Some(GroupingStrategy::Always);
        }
        let formatter = DecimalFormatter::try_new(prefs, options)
            .map_err(|e| LocaleError::Data(format!("decimal: {e}")))?;
        let mut digits = ['0'; 10];
        for (i, slot) in digits.iter_mut().enumerate() {
            let text = formatter
                .format(&parse_decimal(&i.to_string())?)
                .to_string();
            *slot = text.chars().next().unwrap_or('0');
        }
        Ok(Self { formatter, digits })
    }

    pub(crate) fn is_digit(&self, c: char) -> bool {
        self.digits.contains(&c) || c.is_ascii_digit()
    }

    pub(crate) fn render(&self, text: &str) -> Result<String, LocaleError> {
        Ok(self.formatter.format(&parse_decimal(text)?).to_string())
    }

    pub(crate) fn non_digits(&self, s: &str) -> String {
        s.chars().filter(|c| !self.is_digit(*c)).collect()
    }

    /// Byte range from the first to the last digit of `text`.
    fn span(&self, text: &str) -> (usize, usize) {
        let first = text.find(|c| self.is_digit(c)).unwrap_or(0);
        let last_char = text.rfind(|c| self.is_digit(c)).unwrap_or(0);
        let last = last_char + text[last_char..].chars().next().map_or(0, char::len_utf8);
        (first, last.max(first))
    }

    /// Number of digits after the last separator inside the digit span (0 when there is none).
    pub(crate) fn fraction_digits(&self, text: &str) -> usize {
        let (first, last) = self.span(text);
        let number = &text[first..last];
        match number.rfind(|c| !self.is_digit(c)) {
            Some(i) => number[i..].chars().skip(1).count(),
            None => 0,
        }
    }

    /// Replaces the digit span of `text` with `#`, bidi controls removed, minus signs
    /// normalised, and `symbol` (when non-empty) replaced by `marker`.
    pub(crate) fn pattern_of(&self, text: &str, symbol: &str, marker: &str) -> String {
        let (first, last) = self.span(text);
        let clean = |s: &str| {
            let s = office_overrides::normalize_minus(&strip_bidi(s));
            if symbol.is_empty() {
                s
            } else {
                s.replace(symbol, marker)
            }
        };
        format!("{}#{}", clean(&text[..first]), clean(&text[last..]))
    }

    /// The text outside the digit span, minus signs, parentheses and spaces: a currency or
    /// percent symbol.
    pub(crate) fn symbol_of(&self, text: &str) -> String {
        let (first, last) = self.span(text);
        let outside = format!("{}{}", &text[..first], &text[last..]);
        strip_bidi(&outside)
            .trim_matches(|c: char| {
                c.is_whitespace() || matches!(c, '-' | '+' | '\u{2212}' | '(' | ')')
            })
            .to_owned()
    }
}

impl Locale {
    /// Number conventions for this locale. Builds several ICU4X formatters; cache the result.
    pub fn number_format(&self) -> Result<NumberFormat, LocaleError> {
        let d = Digits::new(self, true)?;

        let decimal_separator = strip_bidi(&d.non_digits(&d.render("1.5")?));

        // Group sizes from a 15-digit number: chunks split on the group separator.
        let grouped = d.render("123456789012345")?;
        let group_separator =
            strip_bidi(&d.non_digits(&grouped).chars().take(1).collect::<String>());
        let (primary_grouping, secondary_grouping) = {
            let separator: String = d.non_digits(&grouped).chars().take(1).collect();
            if separator.is_empty() {
                (0, 0)
            } else {
                let chunks: Vec<usize> = grouped
                    .split(separator.as_str())
                    .map(|c| c.chars().filter(|ch| d.is_digit(*ch)).count())
                    .collect();
                let primary = chunks.last().copied().unwrap_or(0);
                let secondary = if chunks.len() >= 3 {
                    chunks[chunks.len() - 2]
                } else {
                    primary
                };
                (primary as u8, secondary as u8)
            }
        };

        let minus_sign =
            office_overrides::normalize_minus(&strip_bidi(&d.non_digits(&d.render("-1")?)));
        let plus_sign = {
            let mut plus = parse_decimal("1")?;
            plus.apply_sign_display(SignDisplay::Always);
            let text = d.formatter.format(&plus).to_string();
            strip_bidi(&d.non_digits(&text))
        };

        let pct = PercentFormatter::try_new(
            PercentFormatterPreferences::from(&self.icu),
            Default::default(),
        )
        .map_err(|e| LocaleError::Data(format!("percent: {e}")))?;
        let pct_text = pct.format(&parse_decimal("1")?).to_string();
        let percent_sign = d.symbol_of(&pct_text);
        let percent_pattern = d.pattern_of(&pct_text, "", "");

        let list_separator = office_overrides::list_separator(self, &decimal_separator);
        let currency = self.currency_format()?;

        Ok(NumberFormat {
            decimal_separator,
            group_separator,
            primary_grouping,
            secondary_grouping,
            minus_sign,
            plus_sign,
            percent_sign,
            percent_pattern,
            digits: d.digits,
            list_separator,
            currency,
        })
    }
}
