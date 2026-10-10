//! Default and supported calendars.

use icu_calendar::AnyCalendarKind;
use icu_calendar::preferences::CalendarPreferences;

use crate::Locale;

/// Calendars Excel and Word can show dates in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CalendarKind {
    Gregorian,
    /// Hijri (Islamic) calendar, including the Umm al-Qura variant.
    Hijri,
    /// Japanese Emperor era years on the Gregorian calendar.
    JapaneseEra,
    /// Thai Buddhist calendar.
    ThaiBuddhist,
    /// Republic of China (Taiwan) era years.
    Minguo,
    /// Korean Dangi/Tangun calendar.
    Korean,
    Hebrew,
    Persian,
}

impl Locale {
    /// The locale's default calendar (what a new workbook in this locale shows).
    pub fn default_calendar(&self) -> CalendarKind {
        // Match on the variant name so new ICU4X calendar kinds fall back to Gregorian.
        let kind = AnyCalendarKind::try_new(CalendarPreferences::from(&self.icu))
            .unwrap_or(AnyCalendarKind::Gregorian);
        let name = format!("{kind:?}");
        match name.as_str() {
            "Buddhist" => CalendarKind::ThaiBuddhist,
            "Japanese" | "JapaneseExtended" => CalendarKind::JapaneseEra,
            "Roc" => CalendarKind::Minguo,
            "Dangi" | "KoreanTraditional" => CalendarKind::Korean,
            "Hebrew" => CalendarKind::Hebrew,
            "Persian" => CalendarKind::Persian,
            n if n.starts_with("Hijri") => CalendarKind::Hijri,
            _ => CalendarKind::Gregorian,
        }
    }

    /// Calendars offered for this locale: Gregorian, the default and the locale's regional ones
    /// (per the calendar choices Office offers; see `office_overrides` for what was measured).
    pub fn supported_calendars(&self) -> Vec<CalendarKind> {
        let mut out = vec![CalendarKind::Gregorian];
        let language = self.icu.id.language.as_str();
        let region_subtag = self.icu.id.region;
        let region = region_subtag.as_ref().map(|r| r.as_str());
        let regional = match (language, region) {
            ("ja", _) => Some(CalendarKind::JapaneseEra),
            ("th", _) => Some(CalendarKind::ThaiBuddhist),
            ("zh", Some("TW")) => Some(CalendarKind::Minguo),
            ("ko", _) => Some(CalendarKind::Korean),
            ("ar" | "ur" | "ps" | "ckb" | "ku" | "sd" | "ug" | "dv", _) => {
                Some(CalendarKind::Hijri)
            }
            ("he", _) => Some(CalendarKind::Hebrew),
            ("fa" | "prs", _) => Some(CalendarKind::Persian),
            _ => None,
        };
        out.extend(regional);
        let default = self.default_calendar();
        if !out.contains(&default) {
            out.push(default);
        }
        out
    }
}
