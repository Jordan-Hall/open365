//! Calendar names, date/time patterns and first day of week.

use icu_calendar::week::{WeekInformation, WeekPreferences};
use icu_calendar::{Date, Gregorian};
use icu_datetime::fieldsets::{T, YMD};
use icu_datetime::input::{DateTime, Time};
use icu_datetime::options::TimePrecision;
use icu_datetime::pattern::{DateTimePattern, FixedCalendarDateTimeNames};
use icu_datetime::{DateTimeFormatter, DateTimeFormatterPreferences};

use writeable::TryWriteable;

use crate::{Locale, LocaleError, office_overrides};

/// Day of week; the discriminant is the Excel `WEEKDAY(.., 1)` number minus one (Sunday = 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weekday {
    Sunday = 0,
    Monday = 1,
    Tuesday = 2,
    Wednesday = 3,
    Thursday = 4,
    Friday = 5,
    Saturday = 6,
}

/// Month, day and AM/PM names for the Gregorian calendar in a locale.
///
/// `month_full`, `month_abbr`, `day_full`, `day_abbr`, `am` and `pm` are what Excel's `TEXT()`
/// produces for `mmmm`, `mmm`, `dddd`, `ddd` and `AM/PM` under this locale (Office overrides applied,
/// see [`crate::office_overrides`]). Excel never inflects month names (measured: `d mmmm` equals
/// `mmmm` in every locale), so the genitive forms are CLDR data for typography and Word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarNames {
    /// Full month names, nominative (`март`).
    pub month_full: [String; 12],
    /// Abbreviated month names.
    pub month_abbr: [String; 12],
    /// Full month names as used next to a day number, genitive where the language has one (`марта`).
    pub month_full_genitive: [String; 12],
    /// Abbreviated month names as used next to a day number.
    pub month_abbr_genitive: [String; 12],
    /// Full day names, Sunday first.
    pub day_full: [String; 7],
    /// Abbreviated day names, Sunday first.
    pub day_abbr: [String; 7],
    /// Ante-meridiem designator.
    pub am: String,
    /// Post-meridiem designator.
    pub pm: String,
}

/// CLDR date and time patterns (UTS 35 field syntax, for example `d/M/y`, `HH:mm:ss`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatePatterns {
    /// Short date, for example `M/d/y`.
    pub short_date: String,
    /// Long date, for example `MMMM d, y`.
    pub long_date: String,
    /// Time with seconds, for example `h:mm:ss a`.
    pub time: String,
    /// Time without seconds.
    pub short_time: String,
}

fn data_error(what: &str, e: impl core::fmt::Display) -> LocaleError {
    LocaleError::Data(format!("{what}: {e}"))
}

impl Locale {
    fn datetime_prefs(&self) -> DateTimeFormatterPreferences {
        DateTimeFormatterPreferences::from(&self.icu)
    }

    /// Formats `values` (one date-time each) with one CLDR pattern string.
    fn format_pattern(
        &self,
        pattern: &str,
        values: &[DateTime<Gregorian>],
    ) -> Result<Vec<String>, LocaleError> {
        let pattern =
            DateTimePattern::try_from_pattern_str(pattern).map_err(|e| data_error(pattern, e))?;
        let mut names = FixedCalendarDateTimeNames::<Gregorian>::try_new(self.datetime_prefs())
            .map_err(|e| data_error("names", e))?;
        let formatter = names
            .include_for_pattern(&pattern)
            .map_err(|e| data_error("pattern names", e))?;
        Ok(values
            .iter()
            .map(
                |value| match formatter.format(value).try_write_to_string() {
                    Ok(text) | Err((_, text)) => text.into_owned(),
                },
            )
            .collect())
    }

    /// Month, day and AM/PM names as Office shows them: CLDR data with the measured Office
    /// overrides applied. Evaluates ~60 strings; cache the result.
    pub fn calendar_names(&self) -> Result<CalendarNames, LocaleError> {
        let mut names = self.calendar_names_cldr()?;
        if let Some(lcid) = self.lcid {
            office_overrides::apply_names(lcid, &mut names);
        }
        Ok(names)
    }

    /// The raw CLDR names, before Office overrides. `month_full` is the stand-alone (nominative) form.
    pub fn calendar_names_cldr(&self) -> Result<CalendarNames, LocaleError> {
        let day = |y, m, d, h| -> Result<DateTime<Gregorian>, LocaleError> {
            let date = Date::try_new_gregorian(y, m, d).map_err(|e| data_error("date", e))?;
            let time = Time::try_new(h, 0, 0, 0).map_err(|e| data_error("time", e))?;
            Ok(DateTime { date, time })
        };
        // 2024-03-03 is a Sunday.
        let months: Vec<DateTime<Gregorian>> = (1..=12)
            .map(|m| day(2024, m, 1, 9))
            .collect::<Result<_, _>>()?;
        let days: Vec<DateTime<Gregorian>> = (0..7)
            .map(|d| day(2024, 3, 3 + d, 9))
            .collect::<Result<_, _>>()?;
        let months_of = |pattern: &str| -> Result<[String; 12], LocaleError> {
            self.format_pattern(pattern, &months)?
                .try_into()
                .map_err(|_| LocaleError::Data(format!("12 months for {pattern}")))
        };
        let days_of = |pattern: &str| -> Result<[String; 7], LocaleError> {
            self.format_pattern(pattern, &days)?
                .try_into()
                .map_err(|_| LocaleError::Data(format!("7 days for {pattern}")))
        };
        let periods = self.format_pattern("a", &[day(2024, 3, 3, 9)?, day(2024, 3, 3, 15)?])?;
        let (am, pm) = match periods.as_slice() {
            [am, pm] => (am.clone(), pm.clone()),
            _ => return Err(LocaleError::Data("day periods".into())),
        };
        Ok(CalendarNames {
            month_full: months_of("LLLL")?,
            month_abbr: months_of("LLL")?,
            month_full_genitive: months_of("MMMM")?,
            month_abbr_genitive: months_of("MMM")?,
            day_full: days_of("EEEE")?,
            day_abbr: days_of("EEE")?,
            am,
            pm,
        })
    }

    /// Short and long date patterns and the time patterns.
    pub fn date_time_patterns(&self) -> Result<DatePatterns, LocaleError> {
        let date = Date::try_new_gregorian(2024, 3, 5).map_err(|e| data_error("date", e))?;
        let time = Time::try_new(13, 4, 5, 0).map_err(|e| data_error("time", e))?;
        let ymd = |fs| -> Result<String, LocaleError> {
            let formatter = DateTimeFormatter::try_new(self.datetime_prefs(), fs)
                .map_err(|e| data_error("date formatter", e))?;
            Ok(formatter.format(&date).pattern().to_string())
        };
        let t = |fs| -> Result<String, LocaleError> {
            let formatter = DateTimeFormatter::try_new(self.datetime_prefs(), fs)
                .map_err(|e| data_error("time formatter", e))?;
            Ok(formatter.format(&time).pattern().to_string())
        };
        Ok(DatePatterns {
            short_date: ymd(YMD::short())?,
            long_date: ymd(YMD::long())?,
            time: t(T::medium())?,
            short_time: t(T::medium().with_time_precision(TimePrecision::Minute))?,
        })
    }

    /// First day of the week for calendar display and `WEEKDAY` return types.
    pub fn first_day_of_week(&self) -> Result<Weekday, LocaleError> {
        let info = WeekInformation::try_new(WeekPreferences::from(&self.icu))
            .map_err(|e| data_error("week", e))?;
        Ok(match info.first_weekday {
            icu_calendar::types::Weekday::Monday => Weekday::Monday,
            icu_calendar::types::Weekday::Tuesday => Weekday::Tuesday,
            icu_calendar::types::Weekday::Wednesday => Weekday::Wednesday,
            icu_calendar::types::Weekday::Thursday => Weekday::Thursday,
            icu_calendar::types::Weekday::Friday => Weekday::Friday,
            icu_calendar::types::Weekday::Saturday => Weekday::Saturday,
            icu_calendar::types::Weekday::Sunday => Weekday::Sunday,
        })
    }
}
