//! The choices of the Print page and the page sequence they produce.

use std::fmt;

use crate::range::{RangeError, parse_page_ranges};

/// Pages per sheet the Print page offers.
pub const PER_SHEET: [u8; 6] = [1, 2, 4, 6, 8, 16];

/// The most copies the Copies box takes.
pub const MAX_COPIES: u32 = 32_767;

/// The most pages one job may hold after copies are counted; a guard against a mistyped count.
pub const MAX_JOB_PAGES: usize = 50_000;

/// Which pages print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RangeMode {
    /// Print All Pages.
    #[default]
    All,
    /// Print Current Page.
    CurrentPage,
    /// Custom Print: the pages typed in the Pages box.
    Custom,
}

/// One-sided or two-sided printing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sides {
    #[default]
    OneSided,
    /// Flip on the long edge (books).
    TwoSidedLongEdge,
    /// Flip on the short edge (note pads).
    TwoSidedShortEdge,
}

impl Sides {
    /// The IPP `sides` keyword.
    #[must_use]
    pub fn ipp(self) -> &'static str {
        match self {
            Self::OneSided => "one-sided",
            Self::TwoSidedLongEdge => "two-sided-long-edge",
            Self::TwoSidedShortEdge => "two-sided-short-edge",
        }
    }
}

/// The scaling choices of the Sheets print page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scaling {
    /// No Scaling: print at the sheet's own scale.
    #[default]
    None,
    /// Fit Sheet on One Page.
    FitSheet,
    /// Fit All Columns on One Page (as many pages tall as needed).
    FitColumns,
    /// Fit All Rows on One Page (as many pages wide as needed).
    FitRows,
    /// Custom Scaling Options: a percentage from 10 to 400.
    Custom(u32),
}

impl Scaling {
    /// Pages wide, pages tall (0 is automatic) and the percentage, as the page setup stores them.
    #[must_use]
    pub fn page_setup(self, base_percent: u32) -> (u32, u32, u32) {
        match self {
            Self::None => (0, 0, base_percent),
            Self::FitSheet => (1, 1, base_percent),
            Self::FitColumns => (1, 0, base_percent),
            Self::FitRows => (0, 1, base_percent),
            Self::Custom(p) => (0, 0, p.clamp(10, 400)),
        }
    }
}

/// Why no page sequence could be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceError {
    Range(RangeError),
    /// The document has no pages.
    NoPages,
    /// Copies is 0 or past [`MAX_COPIES`].
    Copies,
    /// The job would hold more than [`MAX_JOB_PAGES`] pages.
    TooLarge,
}

impl fmt::Display for SequenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Range(e) => e.fmt(f),
            Self::NoPages => write!(f, "There is nothing to print."),
            Self::Copies => write!(f, "Copies must be a number from 1 to {MAX_COPIES}."),
            Self::TooLarge => write!(
                f,
                "That is more than {MAX_JOB_PAGES} pages; lower the copies."
            ),
        }
    }
}

impl std::error::Error for SequenceError {}

impl From<RangeError> for SequenceError {
    fn from(e: RangeError) -> Self {
        Self::Range(e)
    }
}

/// Everything the Print page lets the user choose.
#[derive(Debug, Clone, PartialEq)]
pub struct PrintSettings {
    /// The system name of the chosen printer; `None` until one is chosen.
    pub printer: Option<String>,
    pub copies: u32,
    pub range_mode: RangeMode,
    /// The Pages box, used by [`RangeMode::Custom`].
    pub range_text: String,
    pub sides: Sides,
    /// Collated: 1,2,3 1,2,3. Uncollated: 1,1 2,2 3,3.
    pub collate: bool,
    pub landscape: bool,
    /// An id of [`crate::paper::papers`].
    pub paper_id: String,
    /// An id of [`crate::paper::margin_presets`], or `custom` once the page setup holds other margins.
    pub margins_id: String,
    pub scaling: Scaling,
    /// One of [`PER_SHEET`].
    pub per_sheet: u8,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            printer: None,
            copies: 1,
            range_mode: RangeMode::All,
            range_text: String::new(),
            sides: Sides::OneSided,
            collate: true,
            landscape: false,
            paper_id: "a4".to_owned(),
            margins_id: "normal".to_owned(),
            scaling: Scaling::None,
            per_sheet: 1,
        }
    }
}

impl PrintSettings {
    /// The pages the range choice selects, 1-based and ascending. `total` is the page count of
    /// the document and `current` the page being viewed (1-based).
    ///
    /// # Errors
    /// [`SequenceError`] when the document is empty or the typed range is not valid.
    pub fn pages(&self, total: usize, current: usize) -> Result<Vec<usize>, SequenceError> {
        if total == 0 {
            return Err(SequenceError::NoPages);
        }
        Ok(match self.range_mode {
            RangeMode::All => (1..=total).collect(),
            RangeMode::CurrentPage => vec![current.clamp(1, total)],
            RangeMode::Custom => parse_page_ranges(&self.range_text, total)?,
        })
    }

    /// The source pages in the order they leave the printer, 0-based, with the copies counted.
    /// Collated, the whole selection repeats `copies` times; uncollated each page repeats.
    ///
    /// # Errors
    /// [`SequenceError`] when copies is out of range, the range is invalid or the job is too large.
    pub fn sequence(&self, total: usize, current: usize) -> Result<Vec<usize>, SequenceError> {
        if self.copies == 0 || self.copies > MAX_COPIES {
            return Err(SequenceError::Copies);
        }
        let pages = self.pages(total, current)?;
        let copies = self.copies as usize;
        if pages.len().saturating_mul(copies) > MAX_JOB_PAGES {
            return Err(SequenceError::TooLarge);
        }
        let mut out = Vec::with_capacity(pages.len() * copies);
        if self.collate {
            for _ in 0..copies {
                out.extend(pages.iter().map(|p| p - 1));
            }
        } else {
            for p in &pages {
                out.extend(std::iter::repeat_n(p - 1, copies));
            }
        }
        Ok(out)
    }

    /// The number of sheets of paper one copy of `pages` pages needs.
    #[must_use]
    pub fn sheets_for(&self, pages: usize) -> usize {
        let n = usize::from(self.per_sheet.max(1));
        let faces = pages.div_ceil(n);
        match self.sides {
            Sides::OneSided => faces,
            _ => faces.div_ceil(2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(f: impl FnOnce(&mut PrintSettings)) -> PrintSettings {
        let mut s = PrintSettings::default();
        f(&mut s);
        s
    }

    #[test]
    fn collated_and_uncollated_copies() {
        let s = with(|s| s.copies = 2);
        assert_eq!(s.sequence(3, 1), Ok(vec![0, 1, 2, 0, 1, 2]));
        let s = with(|s| {
            s.copies = 2;
            s.collate = false;
        });
        assert_eq!(s.sequence(3, 1), Ok(vec![0, 0, 1, 1, 2, 2]));
    }

    #[test]
    fn ranges_and_current_page() {
        let s = with(|s| {
            s.range_mode = RangeMode::Custom;
            s.range_text = "2-3".to_owned();
        });
        assert_eq!(s.sequence(4, 1), Ok(vec![1, 2]));
        let s = with(|s| s.range_mode = RangeMode::CurrentPage);
        assert_eq!(s.sequence(4, 3), Ok(vec![2]));
        assert_eq!(s.sequence(4, 99), Ok(vec![3]));
    }

    #[test]
    fn rejects_bad_counts() {
        assert_eq!(
            with(|s| s.copies = 0).sequence(2, 1),
            Err(SequenceError::Copies)
        );
        assert_eq!(
            PrintSettings::default().sequence(0, 1),
            Err(SequenceError::NoPages)
        );
        let s = with(|s| s.copies = MAX_COPIES);
        assert_eq!(s.sequence(10, 1), Err(SequenceError::TooLarge));
        let s = with(|s| {
            s.range_mode = RangeMode::Custom;
            s.range_text = "9".to_owned();
        });
        assert!(matches!(s.sequence(2, 1), Err(SequenceError::Range(_))));
    }

    #[test]
    fn sheet_count_follows_sides_and_n_up() {
        let s = with(|s| s.sides = Sides::TwoSidedLongEdge);
        assert_eq!(s.sheets_for(5), 3);
        let s = with(|s| {
            s.sides = Sides::TwoSidedLongEdge;
            s.per_sheet = 4;
        });
        assert_eq!(s.sheets_for(9), 2);
        assert_eq!(PrintSettings::default().sheets_for(5), 5);
    }

    #[test]
    fn scaling_maps_to_page_setup() {
        assert_eq!(Scaling::FitSheet.page_setup(100), (1, 1, 100));
        assert_eq!(Scaling::FitColumns.page_setup(100), (1, 0, 100));
        assert_eq!(Scaling::FitRows.page_setup(100), (0, 1, 100));
        assert_eq!(Scaling::Custom(900).page_setup(100), (0, 0, 400));
        assert_eq!(Scaling::None.page_setup(80), (0, 0, 80));
    }
}
