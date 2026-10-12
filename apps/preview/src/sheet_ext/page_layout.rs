//! Page Layout tab state and operations of the sheets model: theme choice, margins, orientation,
//! paper size, print area, page breaks, background, print titles, scale to fit and the sheet
//! options. Printing and PDF export (`file.rs`) read it.
//!
//! The state is per worksheet (the theme is per workbook), not an undo step yet, and not yet
//! written to the file's `<pageSetup>`, `<pageMargins>` and defined names.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;

use super::super::{MAX_COL, MAX_ROW, Rect4, SheetModel};
use crate::dialogs::header_footer::HeaderFooter;

/// Page margins in inches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
    pub header: f64,
    pub footer: f64,
}

impl Margins {
    /// The default margins of a new worksheet.
    pub const NORMAL: Self = Self {
        top: 0.75,
        bottom: 0.75,
        left: 0.7,
        right: 0.7,
        header: 0.3,
        footer: 0.3,
    };
    pub const WIDE: Self = Self {
        top: 1.0,
        bottom: 1.0,
        left: 1.0,
        right: 1.0,
        header: 0.5,
        footer: 0.5,
    };
    pub const NARROW: Self = Self {
        top: 0.75,
        bottom: 0.75,
        left: 0.25,
        right: 0.25,
        header: 0.3,
        footer: 0.3,
    };
}

/// The paper sizes of the Size menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paper {
    Letter,
    Tabloid,
    Legal,
    Statement,
    Executive,
    A3,
    A4,
    A5,
    B4,
    B5,
}

impl Paper {
    /// Width and height in inches, portrait.
    #[must_use]
    pub fn inches(self) -> (f64, f64) {
        let mm = |w: f64, h: f64| (w / 25.4, h / 25.4);
        match self {
            Self::Letter => (8.5, 11.0),
            Self::Tabloid => (11.0, 17.0),
            Self::Legal => (8.5, 14.0),
            Self::Statement => (5.5, 8.5),
            Self::Executive => (7.25, 10.5),
            Self::A3 => mm(297.0, 420.0),
            Self::A4 => mm(210.0, 297.0),
            Self::A5 => mm(148.0, 210.0),
            Self::B4 => mm(250.0, 353.0),
            Self::B5 => mm(176.0, 250.0),
        }
    }

    /// The menu item id tail.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "letter" => Self::Letter,
            "tabloid" => Self::Tabloid,
            "legal" => Self::Legal,
            "statement" => Self::Statement,
            "executive" => Self::Executive,
            "a3" => Self::A3,
            "a4" => Self::A4,
            "a5" => Self::A5,
            "b4" => Self::B4,
            "b5" => Self::B5,
            _ => return None,
        })
    }
}

/// The Page Layout settings of one worksheet.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetPage {
    pub margins: Margins,
    pub landscape: bool,
    pub paper: Paper,
    /// Ranges to print; empty prints the used range.
    pub print_areas: Vec<Rect4>,
    /// A manual page break above each of these rows.
    pub row_breaks: BTreeSet<u32>,
    /// A manual page break left of each of these columns.
    pub col_breaks: BTreeSet<u32>,
    pub background: Option<PathBuf>,
    /// Rows repeated at the top of every page, `(first, last)`.
    pub title_rows: Option<(u32, u32)>,
    /// Columns repeated at the left of every page.
    pub title_cols: Option<(u32, u32)>,
    /// Pages wide and tall to fit to; 0 is Automatic.
    pub fit_width: u32,
    pub fit_height: u32,
    /// Print scale in percent when neither fit value is set.
    pub scale: u32,
    pub print_gridlines: bool,
    pub print_headings: bool,
    /// The header and footer, read from the worksheet's `<headerFooter>`.
    pub header_footer: HeaderFooter,
}

impl Default for SheetPage {
    fn default() -> Self {
        Self {
            margins: Margins::NORMAL,
            landscape: false,
            paper: Paper::A4,
            print_areas: Vec::new(),
            row_breaks: BTreeSet::new(),
            col_breaks: BTreeSet::new(),
            background: None,
            title_rows: None,
            title_cols: None,
            fit_width: 0,
            fit_height: 0,
            scale: 100,
            print_gridlines: false,
            print_headings: false,
            header_footer: HeaderFooter::default(),
        }
    }
}

/// Picks a picture for Background...; `None` when cancelled.
pub type PictureChooser = fn() -> Option<PathBuf>;

/// The native Sheet Background picture dialog (rfd). The application installs it in
/// [`PageState::chooser`]; headless runs leave it unset so no dialog opens.
#[must_use]
pub fn system_picture_chooser() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Sheet Background")
        .add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "bmp"])
        .pick_file()
}

/// The theme gallery choices (names only: the workbook's theme part is not rewritten yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeChoice {
    pub theme: String,
    pub colours: String,
    pub fonts: String,
    pub effects: String,
}

impl Default for ThemeChoice {
    fn default() -> Self {
        let office = || "Office".to_owned();
        Self {
            theme: office(),
            colours: office(),
            fonts: office(),
            effects: office(),
        }
    }
}

/// Everything the Page Layout tab changes.
#[derive(Debug, Clone, Default)]
pub struct PageState {
    pub theme: ThemeChoice,
    pub chooser: Option<PictureChooser>,
    sheets: BTreeMap<usize, SheetPage>,
}

impl PageState {
    /// The settings of `sheet` (the defaults for a sheet never changed).
    #[must_use]
    pub fn of(&self, sheet: usize) -> SheetPage {
        self.sheets.get(&sheet).cloned().unwrap_or_default()
    }

    fn of_mut(&mut self, sheet: usize) -> &mut SheetPage {
        self.sheets.entry(sheet).or_default()
    }
}

/// A fit-to-pages value of the Width and Height boxes: `Automatic` or `1 page` to `9 pages`.
#[must_use]
pub fn parse_pages(text: &str) -> Option<u32> {
    let t = text.trim().to_ascii_lowercase();
    if t == "automatic" {
        return Some(0);
    }
    let n: u32 = t.split_whitespace().next()?.parse().ok()?;
    (1..=9).contains(&n).then_some(n)
}

/// `$1:$3` or `1:3` as rows, `$A:$C` or `A:C` as columns.
fn parse_titles(text: &str) -> Option<(bool, u32, u32)> {
    let t = text.replace('$', "");
    let (a, b) = t.split_once(':')?;
    if let (Ok(x), Ok(y)) = (a.parse::<u32>(), b.parse::<u32>()) {
        return (x >= 1 && y >= x && y <= MAX_ROW).then_some((true, x, y));
    }
    let x = open365_xlsx::col_number(a)?;
    let y = open365_xlsx::col_number(b)?;
    (y >= x && y <= MAX_COL).then_some((false, x, y))
}

impl SheetModel {
    /// The page setup of the active sheet.
    #[must_use]
    pub fn page(&self) -> SheetPage {
        let mut page = self.ext_page.of(self.sheet);
        page.header_footer = self.header_footer_of(self.sheet);
        page
    }

    /// The header and footer `sheet` holds in the file.
    #[must_use]
    pub fn header_footer_of(&self, sheet: usize) -> HeaderFooter {
        self.target.xlsx().map_or_else(HeaderFooter::default, |wb| {
            HeaderFooter::from_request(&open365_commands::header_footer(wb, sheet))
        })
    }

    /// Page Setup > Header/Footer > OK: writes the header and footer of `sheet` to the file as
    /// one undo step.
    ///
    /// # Errors
    /// The command layer's message.
    pub fn set_header_footer(&mut self, sheet: usize, hf: &HeaderFooter) -> Result<(), String> {
        if self.header_footer_of(sheet) == *hf {
            return Ok(());
        }
        self.apply(open365_commands::Command::SheetObjects {
            sheet,
            op: open365_commands::ObjectsOp::SetHeaderFooter(hf.to_request()),
        })
    }

    fn page_mut(&mut self) -> &mut SheetPage {
        let s = self.sheet;
        self.ext_page.of_mut(s)
    }

    pub fn set_margins(&mut self, m: Margins) {
        self.page_mut().margins = m;
    }

    pub fn set_landscape(&mut self, landscape: bool) {
        self.page_mut().landscape = landscape;
    }

    pub fn set_paper(&mut self, paper: Paper) {
        self.page_mut().paper = paper;
    }

    /// Print Area > Set Print Area: the selection replaces the areas.
    pub fn set_print_area(&mut self) {
        let r = self.range();
        self.page_mut().print_areas = vec![r];
    }

    /// Print Area > Add to Print Area.
    pub fn add_print_area(&mut self) {
        let r = self.range();
        let p = self.page_mut();
        if !p.print_areas.contains(&r) {
            p.print_areas.push(r);
        }
    }

    pub fn clear_print_area(&mut self) {
        self.page_mut().print_areas.clear();
    }

    /// Breaks > Insert Page Break: above the active row and left of the active column (a
    /// break at the first row or column does not exist).
    pub fn insert_page_break(&mut self) {
        let (r, c) = self.active;
        let p = self.page_mut();
        if r > 1 {
            p.row_breaks.insert(r);
        }
        if c > 1 {
            p.col_breaks.insert(c);
        }
    }

    /// Breaks > Remove Page Break: the breaks above and left of the active cell.
    pub fn remove_page_break(&mut self) {
        let (r, c) = self.active;
        let p = self.page_mut();
        p.row_breaks.remove(&r);
        p.col_breaks.remove(&c);
    }

    /// Breaks > Reset All Page Breaks.
    pub fn reset_page_breaks(&mut self) {
        let p = self.page_mut();
        p.row_breaks.clear();
        p.col_breaks.clear();
    }

    /// Whether Remove Page Break applies at the active cell.
    #[must_use]
    pub fn on_page_break(&self) -> bool {
        let (r, c) = self.active;
        let p = self.page();
        p.row_breaks.contains(&r) || p.col_breaks.contains(&c)
    }

    pub fn set_background(&mut self, path: Option<PathBuf>) {
        self.page_mut().background = path;
    }

    /// Print Titles from `$1:$2` / `$A:$B` text; `clear` removes both.
    pub fn set_print_titles_text(&mut self, text: &str) -> Result<(), String> {
        if text.trim().eq_ignore_ascii_case("clear") {
            let p = self.page_mut();
            p.title_rows = None;
            p.title_cols = None;
            return Ok(());
        }
        let (rows, a, b) =
            parse_titles(text).ok_or_else(|| format!("`{text}` is not a row or column range"))?;
        let p = self.page_mut();
        if rows {
            p.title_rows = Some((a, b));
        } else {
            p.title_cols = Some((a, b));
        }
        Ok(())
    }

    /// Print Titles with entire rows or columns selected (the Page Setup dialog's Rows to
    /// repeat at top and Columns to repeat at left boxes taken from the selection).
    pub fn set_print_titles_from_selection(&mut self) -> Result<(), String> {
        let (r1, c1, r2, c2) = self.range();
        let whole_rows = c1 == 1 && c2 == MAX_COL;
        let whole_cols = r1 == 1 && r2 == MAX_ROW;
        let p = self.page_mut();
        if whole_rows && !whole_cols {
            p.title_rows = Some((r1, r2));
        } else if whole_cols && !whole_rows {
            p.title_cols = Some((c1, c2));
        } else {
            return Err("Select entire rows or columns to repeat".to_owned());
        }
        Ok(())
    }

    /// Width box: pages wide, 0 for Automatic. Fitting to pages replaces the percentage scale.
    pub fn set_fit_width(&mut self, pages: u32) {
        self.page_mut().fit_width = pages.min(9);
    }

    pub fn set_fit_height(&mut self, pages: u32) {
        self.page_mut().fit_height = pages.min(9);
    }

    /// Scale box (10 to 400 percent); it resets Width and Height to Automatic.
    pub fn set_print_scale(&mut self, percent: u32) {
        let p = self.page_mut();
        p.scale = percent.clamp(10, 400);
        p.fit_width = 0;
        p.fit_height = 0;
    }

    /// The scale printing uses: the percentage, or the largest that fits the page counts.
    #[must_use]
    pub fn effective_scale(
        &self,
        content_w_in: f64,
        content_h_in: f64,
        page_w_in: f64,
        page_h_in: f64,
    ) -> f64 {
        let p = self.page();
        if p.fit_width == 0 && p.fit_height == 0 {
            return f64::from(p.scale) / 100.0;
        }
        let mut s: f64 = 4.0;
        if p.fit_width > 0 {
            s = s.min(f64::from(p.fit_width) * page_w_in / content_w_in.max(1e-6));
        }
        if p.fit_height > 0 {
            s = s.min(f64::from(p.fit_height) * page_h_in / content_h_in.max(1e-6));
        }
        s.clamp(0.1, 1.0)
    }

    pub fn set_print_gridlines(&mut self, on: bool) {
        self.page_mut().print_gridlines = on;
    }

    pub fn set_print_headings(&mut self, on: bool) {
        self.page_mut().print_headings = on;
    }
}
