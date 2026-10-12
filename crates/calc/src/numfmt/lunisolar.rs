//! The lunisolar calendars of a `[$-xx0409]` tag (calendar bytes 14, 15, 17, 18 and 19), as Excel renders them.
//!
//! Excel does not use a published table of the Chinese calendar for these. Measured for every day from serial 1 to 2958465
//! (9999-12-31), the month boundaries follow the real calendar only from about 1960 to 2100; before and after that the year
//! boundary and the month lengths come from two different approximations, so a year can end with a one-day month 13 or a
//! 28-day month 12. The result is therefore stored as measured: one byte per month (`tools/office/calc-calendar-tables.py`
//! writes `lunisolar_f.bin` and `lunisolar_e.bin` from `make-numfmt-calc-refs.ps1 -Mode Series`):
//!
//! * bits 0 to 4: the length of the month in days,
//! * bit 7: the year number goes up by one,
//! * bit 6: the month number restarts at 1 (otherwise it goes up by one). Both bits are set at an ordinary new year; the
//!   year 1960 begins with a 15-day month numbered 13.
//!
//! A header lists the 89 days (1 January of 1960, 2051, 2142, ... every 91 years) on which Excel raises the year number in
//! the middle of month 12, and whether the day number stays the same on that day (a 31-day month).
//!
//! The month number counts a leap month in its place (the month after a leap month 2 is number 4), so it runs from 1 to 13
//! (14 in some approximated years). Two tables exist: `F` is used by byte 15 and 17 (Chinese), `E` by 14, 18 and 19
//! (Japanese and Korean tags); they agree on all but 80 month starts.

use std::sync::OnceLock;

/// Real day number (the serial number of the 1900 date system, minus one from serial 61 on) of the first day of the first month of each table, the month number and the year of
/// that month. Serial 1 is day 16 of month 12 of lunar year 1899.
const FIRST_START: i64 = -14;
const FIRST_YEAR: i64 = 1899;
const FIRST_MONTH: i64 = 12;

static TABLE_F: &[u8] = include_bytes!("lunisolar_f.bin");
static TABLE_E: &[u8] = include_bytes!("lunisolar_e.bin");

struct Table {
    /// First real day of each month.
    start: Vec<i64>,
    /// Year and month number at the first day of each month.
    year: Vec<i32>,
    month: Vec<u8>,
    /// (real day, day number repeats) of the mid-month year changes, ascending.
    bumps: Vec<(i64, bool)>,
}

fn build(bytes: &[u8]) -> Table {
    let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let mut bumps = Vec::with_capacity(count);
    for chunk in bytes[4..4 + 4 * count].chunks_exact(4) {
        let v = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        bumps.push((i64::from(v & 0x7FFF_FFFF), v >> 31 != 0));
    }
    let bytes = &bytes[4 + 4 * count..];
    let mut start = Vec::with_capacity(bytes.len());
    let mut year = Vec::with_capacity(bytes.len());
    let mut month = Vec::with_capacity(bytes.len());
    let (mut s, mut y, mut m) = (FIRST_START, FIRST_YEAR, FIRST_MONTH);
    let mut next_bump = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 {
            y += i64::from(b >> 7);
            m = if b & 0x40 != 0 { 1 } else { m + 1 };
        }
        start.push(s);
        year.push(y as i32);
        month.push(m as u8);
        let end = s + i64::from(b & 0x1F);
        while next_bump < bumps.len() && bumps[next_bump].0 < end {
            y += 1;
            next_bump += 1;
        }
        s = end;
    }
    Table {
        start,
        year,
        month,
        bumps,
    }
}

fn table(chinese: bool) -> &'static Table {
    static F: OnceLock<Table> = OnceLock::new();
    static E: OnceLock<Table> = OnceLock::new();
    if chinese {
        F.get_or_init(|| build(TABLE_F))
    } else {
        E.get_or_init(|| build(TABLE_E))
    }
}

/// (year, month, day) of a real day number (see `FIRST_START`). `chinese` selects the table of calendar bytes 15 and 17.
pub(super) fn from_day(day: i64, chinese: bool) -> Option<(i64, i64, i64)> {
    let t = table(chinese);
    let i = t.start.partition_point(|&s| s <= day).checked_sub(1)?;
    let (mut year, mut label) = (i64::from(t.year[i]), day - t.start[i] + 1);
    let from = t.bumps.partition_point(|b| b.0 < t.start[i]);
    if let Some(&(bump, repeats)) = t.bumps.get(from)
        && bump <= day
        && t.start.get(i + 1).is_none_or(|&next| bump < next)
    {
        year += 1;
        if repeats {
            label -= 1;
        }
    }
    Some((year, i64::from(t.month[i]), label))
}

#[cfg(test)]
mod tests {
    use super::from_day;

    #[test]
    fn matches_measured_excel_dates() {
        // serial 45366 is 2024-03-15 (day 45365), lunar month 2 day 6; 2024-02-10 is the first day of year 2024
        assert_eq!(from_day(45365, true), Some((2024, 2, 6)));
        assert_eq!(from_day(45331, true), Some((2024, 1, 1)));
        assert_eq!(from_day(45330, true), Some((2023, 13, 30)));
        // the leap month 2 of 2023 is month 3 (serial 45017 is 2023-04-01)
        assert_eq!(from_day(45016, true), Some((2023, 3, 11)));
        assert_eq!(from_day(1, true), Some((1899, 12, 16)));
    }
}
