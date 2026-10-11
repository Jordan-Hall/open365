//! The locale data the formula engine uses (ADR-011): separators, currency, month and day names, date order and
//! the built-in date formats, derived from an [`open365_locale::Locale`].
//!
//! What was measured against Excel (the oracle's system locale is en-GB; `[$-LCID]` prefixes reach every LCID):
//!
//! * A `[$-LCID]` prefix changes the month, day and AM/PM names (and `ggg` eras) of a date section. It does not
//!   change the decimal or group separator of a number: `TEXT(1234567.891,"[$-407]#,##0.00")` is `1,234,567.89`.
//! * Date text follows the system locale: under en-GB `1/2/2024` is 1 February, `15.03.2024` and month-first
//!   names (`March 15, 2024`) are refused, `$1` is refused (`£1` and `€5` are numbers).
//! * The built-in format ids (14, 22, ...) cannot be set from the oracle (Excel keeps the en-US code it is given).
//!   Their locale forms come from `open365_locale::builtin_number_format_in`: measured under en-GB, the
//!   specification's tables for the East Asian and Thai locales, derived from Windows locale data elsewhere
//!   (`docs/testing/xlsx-numfmt-locale.md`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use open365_locale::Locale;

/// The order of day, month and year in a numeric date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOrder {
    /// Month, day, year (en-US).
    Mdy,
    /// Day, month, year (en-GB, most of Europe).
    Dmy,
    /// Year, month, day (ja-JP, zh-CN, ...).
    Ymd,
}

/// Locale data for number formatting and text conversion.
#[derive(Debug, Clone)]
pub struct FormatLocale {
    /// BCP-47 tag.
    pub tag: String,
    /// Office LCID, when the locale has one.
    pub lcid: Option<u32>,
    /// Decimal separator.
    pub decimal: String,
    /// Group (thousands) separator.
    pub group: String,
    /// Currency symbol (`$`, `£`, `€`).
    pub currency: String,
    /// Currency uses a negative pattern with parentheses (`($1,200)`), as Excel's en-US does.
    pub currency_parens: bool,
    /// Full month names.
    pub month_full: [String; 12],
    /// Abbreviated month names.
    pub month_abbr: [String; 12],
    /// Full day names, Sunday first.
    pub day_full: [String; 7],
    /// Abbreviated day names, Sunday first.
    pub day_abbr: [String; 7],
    /// Ante meridiem designator.
    pub am: String,
    /// Post meridiem designator.
    pub pm: String,
    /// Order of the fields of a numeric date.
    pub date_order: DateOrder,
    /// The short date as an Excel format code (`dd/mm/yyyy`).
    pub short_date: String,
    /// The long date as an Excel format code.
    pub long_date: String,
    /// The time without seconds as an Excel format code.
    pub short_time: String,
    /// The time with seconds as an Excel format code.
    pub time: String,
}

fn strings<const N: usize>(src: &[String; N]) -> [String; N] {
    src.clone()
}

impl FormatLocale {
    /// Builds the data for a locale. Missing CLDR data falls back to en-US values.
    pub fn new(locale: &Locale) -> Self {
        let mut out = Self::en_us_data();
        out.tag = locale.bcp47().to_owned();
        out.lcid = locale.lcid();
        if let Ok(n) = locale.number_format() {
            out.decimal = n.decimal_separator.clone();
            out.group = n.group_separator.clone();
            if !n.currency.symbol.is_empty() {
                out.currency = n.currency.symbol.clone();
            }
        }
        out.currency_parens = out.tag == "en-US";
        if let Ok(c) = locale.calendar_names() {
            out.month_full = strings(&c.month_full);
            out.month_abbr = strings(&c.month_abbr);
            out.day_full = strings(&c.day_full);
            out.day_abbr = strings(&c.day_abbr);
            out.am = c.am.clone();
            out.pm = c.pm.clone();
        }
        if let Ok(p) = locale.date_time_patterns() {
            out.date_order = date_order(&p.short_date);
            out.short_date = cldr_to_excel(&p.short_date);
            out.long_date = cldr_to_excel(&p.long_date);
            out.short_time = cldr_to_excel(&p.short_time);
            out.time = cldr_to_excel(&p.time);
        }
        out
    }

    /// The data for an Office LCID, cached; `None` for an LCID the locale table does not know.
    pub fn for_lcid(lcid: u32) -> Option<Arc<FormatLocale>> {
        static CACHE: OnceLock<Mutex<HashMap<u32, Option<Arc<FormatLocale>>>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = cache.lock().ok()?;
        guard
            .entry(lcid)
            .or_insert_with(|| {
                Locale::from_lcid(lcid)
                    .ok()
                    .map(|l| Arc::new(FormatLocale::new(&l)))
            })
            .clone()
    }

    /// The default locale: en-US.
    pub fn en_us() -> Arc<FormatLocale> {
        static EN_US: OnceLock<Arc<FormatLocale>> = OnceLock::new();
        EN_US
            .get_or_init(|| Arc::new(FormatLocale::new(&Locale::en_us())))
            .clone()
    }

    fn en_us_data() -> Self {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        let arr12 = |v: &[&str]| -> [String; 12] { s(v).try_into().unwrap_or_default() };
        let arr7 = |v: &[&str]| -> [String; 7] { s(v).try_into().unwrap_or_default() };
        FormatLocale {
            tag: "en-US".into(),
            lcid: Some(0x409),
            decimal: ".".into(),
            group: ",".into(),
            currency: "$".into(),
            currency_parens: true,
            month_full: arr12(&[
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ]),
            month_abbr: arr12(&[
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ]),
            day_full: arr7(&[
                "Sunday",
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
            ]),
            day_abbr: arr7(&["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]),
            am: "AM".into(),
            pm: "PM".into(),
            date_order: DateOrder::Mdy,
            short_date: "m/d/yyyy".into(),
            long_date: "dddd, mmmm dd, yyyy".into(),
            short_time: "h:mm AM/PM".into(),
            time: "h:mm:ss AM/PM".into(),
        }
    }

    /// The Excel format code of a built-in `numFmtId` in this locale, in the invariant syntax a file stores. `None`
    /// for ids that are not built in (23 to 26 and 82 up show as General).
    ///
    /// A locale with an LCID uses the shared table of `open365-locale` (the same rules and data as the XLSX reader:
    /// ECMA-376 18.8.30 tables for the East Asian and Thai locales, the measured en-GB rules and the NLS data for
    /// the others). A locale without one derives 14, 22 and the currency ids from CLDR.
    pub fn builtin_format_code(&self, id: u32) -> Option<String> {
        if let Some(lcid) = self.lcid {
            return open365_locale::builtin_number_format_in(id, lcid);
        }
        let en = crate::numfmt::builtin_format_code(id)?;
        let sym = format!("\"{}\"", self.currency);
        Some(match id {
            14 => self.short_date.clone(),
            22 => format!(
                "{} {}",
                self.short_date,
                self.short_time.replace("AM/PM", "").trim()
            ),
            5..=8 | 42 | 44 if self.currency != "$" => en.replace('$', &sym),
            _ => en.to_owned(),
        })
    }
}

fn date_order(short_date: &str) -> DateOrder {
    let first = short_date
        .chars()
        .find(|c| matches!(c, 'y' | 'M' | 'd' | 'L'));
    match first {
        Some('y') => DateOrder::Ymd,
        Some('d') => DateOrder::Dmy,
        _ => DateOrder::Mdy,
    }
}

/// Converts a CLDR date pattern (`dd/MM/y`, `h:mm:ss a`) to an Excel format code.
pub fn cldr_to_excel(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let has_meridiem = chars.contains(&'a');
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        match c {
            '\'' => {
                // Quoted literal.
                let mut j = i + 1;
                let mut lit = String::new();
                while j < chars.len() && chars[j] != '\'' {
                    lit.push(chars[j]);
                    j += 1;
                }
                if lit.is_empty() {
                    out.push('\'');
                } else {
                    out.push_str(&format!("\"{lit}\""));
                }
                i = j + 1;
                continue;
            }
            'y' | 'u' | 'U' => out.push_str(if n == 2 { "yy" } else { "yyyy" }),
            'M' | 'L' => out.push_str(&"m".repeat(n.min(5))),
            'd' => out.push_str(&"d".repeat(n.min(2))),
            'E' | 'c' | 'e' => out.push_str(if n <= 3 { "ddd" } else { "dddd" }),
            'a' | 'b' | 'B' => out.push_str("AM/PM"),
            'H' | 'K' | 'k' => out.push_str(&"h".repeat(n.min(2))),
            'h' => out.push_str(&"h".repeat(n.min(2))),
            'm' => out.push_str(&"m".repeat(n.min(2))),
            's' => out.push_str(&"s".repeat(n.min(2))),
            '\u{200e}' | '\u{200f}' | '\u{061c}' => {}
            '\u{202f}' | '\u{a0}' => out.push(' '),
            c if c.is_ascii_alphabetic() => {}
            c => out.extend(std::iter::repeat_n(c, n)),
        }
        i += n;
    }
    let _ = has_meridiem;
    out
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<FormatLocale>>> = const { RefCell::new(None) };
}

/// The locale in force on this thread: the one of the evaluation in progress, otherwise en-US.
pub fn current() -> Arc<FormatLocale> {
    CURRENT
        .with(|c| c.borrow().clone())
        .unwrap_or_else(FormatLocale::en_us)
}

/// Makes `locale` the current one until the guard is dropped (nested evaluations restore the outer one).
pub(crate) struct Scope {
    previous: Option<Arc<FormatLocale>>,
}

impl Scope {
    pub(crate) fn enter(locale: &Arc<FormatLocale>) -> Scope {
        let previous = CURRENT.with(|c| c.borrow_mut().replace(Arc::clone(locale)));
        Scope { previous }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|c| *c.borrow_mut() = self.previous.take());
    }
}
