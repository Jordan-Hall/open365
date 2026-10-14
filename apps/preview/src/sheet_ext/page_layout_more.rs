//! Page Layout tab, file side: the page setup is read from and written to the worksheet
//! (`pageMargins`, `pageSetup`, `printOptions`, `rowBreaks`, `colBreaks`, and the print area and
//! titles as defined names), as one undo step per change.
//!
//! The in-memory [`SheetPage`] of `page_layout.rs` stays the working copy the print path reads.
//! [`SheetModel::page_sync`] copies the file into it before a Page Layout command runs, and
//! [`SheetModel::page_save`] writes it back after the command. [`SheetModel::page_from_file`] is the
//! same data without touching the working copy (what `SheetModel::page` should return once the
//! file is the source of truth).

use open365_commands::{Command, SheetPageOp, sheet_page_ops};
use open365_xlsx::{PageMargins, PageSetup, col_letters};

use super::super::SheetModel;
use super::page_layout::{Margins, Paper, SheetPage};

/// The `paperSize` code of a paper in the Size menu.
#[must_use]
pub fn paper_code(p: Paper) -> u32 {
    match p {
        Paper::Letter => 1,
        Paper::Tabloid => 3,
        Paper::Legal => 5,
        Paper::Statement => 6,
        Paper::Executive => 7,
        Paper::A3 => 8,
        Paper::A4 => 9,
        Paper::A5 => 11,
        Paper::B4 => 12,
        Paper::B5 => 13,
    }
}

/// The paper of a `paperSize` code; codes the menu does not list give `None`.
#[must_use]
pub fn paper_of_code(code: u32) -> Option<Paper> {
    Some(match code {
        1 => Paper::Letter,
        3 => Paper::Tabloid,
        5 => Paper::Legal,
        6 => Paper::Statement,
        7 => Paper::Executive,
        8 => Paper::A3,
        9 => Paper::A4,
        11 => Paper::A5,
        12 => Paper::B4,
        13 => Paper::B5,
        _ => return None,
    })
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

impl SheetModel {
    /// The page setup `sheet` holds in the file.
    #[must_use]
    pub fn page_file_setup(&self, sheet: usize) -> PageSetup {
        self.target
            .xlsx()
            .map(|wb| wb.page_setup(sheet))
            .unwrap_or_default()
    }

    /// The page settings of the active sheet as the file has them (the background picture and
    /// the header and footer come from the working copy as before).
    #[must_use]
    pub fn page_from_file(&self) -> SheetPage {
        self.page()
    }

    /// Puts the file's page setup of the active sheet over `page`, the working copy.
    pub(super) fn page_over_file(&self, mut page: SheetPage) -> SheetPage {
        if self.target.xlsx().is_none() {
            return page;
        }
        let f = self.page_file_setup(self.sheet);
        page.margins = Margins {
            top: f.margins.top,
            bottom: f.margins.bottom,
            left: f.margins.left,
            right: f.margins.right,
            header: f.margins.header,
            footer: f.margins.footer,
        };
        page.landscape = f.landscape;
        page.paper = f.paper_size.and_then(paper_of_code).unwrap_or(Paper::A4);
        page.print_areas.clone_from(&f.print_area);
        page.row_breaks = f.row_breaks.iter().copied().collect();
        page.col_breaks = f.col_breaks.iter().copied().collect();
        page.title_rows = f.title_rows;
        page.title_cols = f.title_cols;
        if f.fit_to_page {
            page.fit_width = f.fit_width;
            page.fit_height = f.fit_height;
        } else {
            page.fit_width = 0;
            page.fit_height = 0;
        }
        page.scale = f.scale.clamp(10, 400);
        page.print_gridlines = f.print_gridlines;
        page.print_headings = f.print_headings;
        page
    }

    /// Copies the file's page setup of the active sheet into the working copy.
    pub fn page_sync(&mut self) {
        if self.target.xlsx().is_none() {
            return;
        }
        let want = self.page_from_file();
        if self.ext_page.of(self.sheet) == self.page_working_equal(&want) {
            return;
        }
        let saved = (self.active, self.anchor, self.focus);
        self.set_margins(want.margins);
        self.set_landscape(want.landscape);
        self.set_paper(want.paper);
        self.set_print_scale(want.scale);
        self.set_fit_width(want.fit_width);
        self.set_fit_height(want.fit_height);
        self.set_print_gridlines(want.print_gridlines);
        self.set_print_headings(want.print_headings);
        self.clear_print_area();
        for (r1, c1, r2, c2) in &want.print_areas {
            self.anchor = (*r1, *c1);
            self.focus = (*r2, *c2);
            self.active = (*r1, *c1);
            self.add_print_area();
        }
        self.reset_page_breaks();
        for r in &want.row_breaks {
            self.active = (*r, 1);
            self.insert_page_break();
        }
        for c in &want.col_breaks {
            self.active = (1, *c);
            self.insert_page_break();
        }
        let _ = self.set_print_titles_text("clear");
        if let Some((a, b)) = want.title_rows {
            let _ = self.set_print_titles_text(&format!("${a}:${b}"));
        }
        if let Some((a, b)) = want.title_cols {
            let _ = self.set_print_titles_text(&format!("${}:${}", col_letters(a), col_letters(b)));
        }
        (self.active, self.anchor, self.focus) = saved;
    }

    /// `want` with the fields the working copy keeps that the file does not.
    fn page_working_equal(&self, want: &SheetPage) -> SheetPage {
        let mut w = self.ext_page.of(self.sheet);
        w.margins = want.margins;
        w.landscape = want.landscape;
        w.paper = want.paper;
        w.print_areas.clone_from(&want.print_areas);
        w.row_breaks.clone_from(&want.row_breaks);
        w.col_breaks.clone_from(&want.col_breaks);
        w.title_rows = want.title_rows;
        w.title_cols = want.title_cols;
        w.fit_width = want.fit_width;
        w.fit_height = want.fit_height;
        w.scale = want.scale;
        w.print_gridlines = want.print_gridlines;
        w.print_headings = want.print_headings;
        w
    }

    /// Writes the working copy of the active sheet to the file as one undo step; nothing happens
    /// when the file already says the same.
    pub fn page_save(&mut self) -> Result<(), String> {
        if self.target.xlsx().is_none() {
            return Ok(());
        }
        let sheet = self.sheet;
        let now = self.ext_page.of(sheet);
        let file = self.page_file_setup(sheet);
        let mut next = file.clone();
        next.margins = PageMargins {
            left: now.margins.left,
            right: now.margins.right,
            top: now.margins.top,
            bottom: now.margins.bottom,
            header: now.margins.header,
            footer: now.margins.footer,
        };
        next.landscape = now.landscape;
        // A code the Size menu cannot show (envelopes, A4 small) stays while the menu shows A4.
        let shown = file.paper_size.and_then(paper_of_code).unwrap_or(Paper::A4);
        if now.paper != shown {
            next.paper_size = Some(paper_code(now.paper));
        }
        next.print_gridlines = now.print_gridlines;
        next.print_headings = now.print_headings;
        next.fit_to_page = now.fit_width > 0 || now.fit_height > 0;
        if next.fit_to_page {
            next.fit_width = now.fit_width;
            next.fit_height = now.fit_height;
        } else {
            next.fit_width = 0;
            next.fit_height = 0;
            next.scale = now.scale;
        }
        next.print_area.clone_from(&now.print_areas);
        next.row_breaks = now.row_breaks.iter().copied().collect();
        next.col_breaks = now.col_breaks.iter().copied().collect();
        next.title_rows = now.title_rows;
        next.title_cols = now.title_cols;
        let (a, b) = (file.margins, next.margins);
        if close(a.left, b.left)
            && close(a.right, b.right)
            && close(a.top, b.top)
            && close(a.bottom, b.bottom)
            && close(a.header, b.header)
            && close(a.footer, b.footer)
        {
            next.margins = a;
        }
        if next == file {
            return Ok(());
        }
        self.apply(Command::SheetPage {
            sheet,
            op: SheetPageOp::SetPageSetup(sheet_page_ops::PageSetupRequest::from(&next)),
        })
    }
}
