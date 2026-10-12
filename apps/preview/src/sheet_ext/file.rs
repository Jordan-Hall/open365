//! File tab operations of the sheets model: New, Open, Save, Save As, Print, Export to PDF.
//!
//! Printing lays the active sheet out by its Page Layout settings (paper, orientation, margins,
//! print area, manual and automatic page breaks, print titles, scale or fit to pages, gridlines
//! and headings) into pages; the PDF writer here draws them with the standard Helvetica font.
//! There is no printer driver yet: Print counts the pages and reports them.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use open365_commands::Command;

use super::super::{MAX_COL, SheetModel};
use super::page_layout::SheetPage;
use crate::dialogs::header_footer::{self as hf, Context};

/// Picks the workbook to open; `None` when cancelled.
pub type OpenChooser = fn() -> Option<PathBuf>;
/// Picks the PDF to write from a suggested file name; `None` when cancelled.
pub type PdfChooser = fn(&str) -> Option<PathBuf>;

/// The native Open dialog (rfd).
#[must_use]
pub fn system_open_chooser() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open")
        .add_filter("Excel workbooks", &["xlsx", "xlsm", "xltx", "xltm"])
        .pick_file()
}

/// The native Export to PDF dialog (rfd).
#[must_use]
pub fn system_pdf_chooser(name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Publish as PDF")
        .set_file_name(name)
        .add_filter("PDF (*.pdf)", &["pdf"])
        .save_file()
}

/// The dialogs the File tab opens. The application installs the native ones; headless runs
/// leave them unset so no dialog opens (the commands then take the path as an argument).
#[derive(Debug, Clone, Copy, Default)]
pub struct FileState {
    pub open_chooser: Option<OpenChooser>,
    pub pdf_chooser: Option<PdfChooser>,
}

/// One printed page: the rows and columns it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintPage {
    pub rows: Vec<u32>,
    pub cols: Vec<u32>,
}

/// Pixels (96 dpi) to points.
const PT_PER_PX: f64 = 0.75;
/// Width of the row-number column and height of the column-letter row when headings print.
const HEADING_W: f64 = 26.0;
const HEADING_H: f64 = 15.0;
const FONT_PT: f64 = 11.0;

impl SheetModel {
    // ---- New, Open, Save -----------------------------------------------------------------

    /// Carries the host-installed pieces (clipboard, dialogs, locale) over to `next`.
    fn take_over(&mut self, mut next: Self) {
        std::mem::swap(&mut self.clipboard, &mut next.clipboard);
        std::mem::swap(&mut self.chooser, &mut next.chooser);
        std::mem::swap(&mut self.locale, &mut next.locale);
        next.ext_file = self.ext_file;
        next.ext_page.chooser = self.ext_page.chooser;
        next.custom_lists = std::mem::take(&mut self.custom_lists);
        *self = next;
    }

    /// File > New: a blank workbook. There is one window, so unsaved changes are never dropped.
    pub fn file_new(&mut self) -> Result<(), String> {
        self.refuse_when_dirty()?;
        self.take_over(Self::blank());
        Ok(())
    }

    /// File > Open: `path`, or the chosen file; `Ok(None)` when the dialog was cancelled.
    pub fn file_open(&mut self, path: Option<&Path>) -> Result<Option<PathBuf>, String> {
        let Some(path) = path
            .map(Path::to_path_buf)
            .or_else(|| self.ext_file.open_chooser.and_then(|c| c()))
        else {
            return Ok(None);
        };
        self.refuse_when_dirty()?;
        let next = Self::open(&path)?;
        self.take_over(next);
        Ok(Some(path))
    }

    fn refuse_when_dirty(&mut self) -> Result<(), String> {
        if self.is_dirty() {
            let msg = "Save the changes to this workbook first".to_owned();
            self.status.clone_from(&msg);
            return Err(msg);
        }
        Ok(())
    }

    /// File > Save: to the workbook's file, or Save As for a new workbook.
    pub fn file_save(&mut self) -> Result<Option<PathBuf>, String> {
        match self.path() {
            Some(path) => {
                self.apply(Command::Save { path: path.clone() })?;
                Ok(Some(path))
            }
            None => self.save_as(),
        }
    }

    /// File > Save As: to `path`, or where the dialog says; `Ok(None)` when cancelled.
    pub fn file_save_as(&mut self, path: Option<&Path>) -> Result<Option<PathBuf>, String> {
        let Some(path) = path else {
            return self.save_as();
        };
        let mut path = path.to_path_buf();
        if path.extension().is_none() {
            path.set_extension("xlsx");
        }
        self.apply(Command::Save { path: path.clone() })?;
        Ok(Some(path))
    }

    // ---- printing --------------------------------------------------------------------------

    /// The paper size in points after the orientation.
    fn paper_pt(&self) -> (f64, f64) {
        let p = self.page();
        let (w, h) = p.paper.inches();
        let (w, h) = if p.landscape { (h, w) } else { (w, h) };
        (w * 72.0, h * 72.0)
    }

    /// The ranges that print: the print areas, or the used range.
    fn print_ranges(&self) -> Vec<(u32, u32, u32, u32)> {
        let areas = self.page().print_areas;
        if !areas.is_empty() {
            return areas;
        }
        self.target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .and_then(|ws| ws.used_range())
            .map(|r| vec![r])
            .unwrap_or_default()
    }

    /// Lays the sheet out into pages; the scale used is returned with them.
    pub fn print_pages(&self) -> Vec<PrintPage> {
        self.print_layout().0
    }

    fn print_layout(&self) -> (Vec<PrintPage>, f64) {
        let page = self.page();
        let geo = self.geometry();
        let col_w = |c: u32| f64::from(geo.col_px(c)) * PT_PER_PX;
        let row_h = |r: u32| f64::from(geo.row_px(r)) * PT_PER_PX;
        let (pw, ph) = self.paper_pt();
        let m = page.margins;
        let (avail_w, avail_h) = (
            pw - (m.left + m.right) * 72.0,
            ph - (m.top + m.bottom) * 72.0,
        );
        let ranges = self.print_ranges();
        // Fit to pages: the largest scale at which the first range fits the pages asked for.
        let first = ranges.first().copied().unwrap_or((1, 1, 1, 1));
        let cw: f64 = (first.1..=first.3)
            .filter(|&c| !geo.col_hidden(c))
            .map(col_w)
            .sum();
        let ch: f64 = (first.0..=first.2)
            .filter(|&r| !geo.row_hidden(r))
            .map(row_h)
            .sum();
        let scale = self.effective_scale(cw / 72.0, ch / 72.0, avail_w / 72.0, avail_h / 72.0);
        let heading = if page.print_headings {
            (HEADING_W, HEADING_H)
        } else {
            (0.0, 0.0)
        };
        let (aw, ah) = (avail_w / scale - heading.0, avail_h / scale - heading.1);
        let mut pages = Vec::new();
        for (r1, c1, r2, c2) in ranges {
            let rows: Vec<u32> = (r1..=r2).filter(|&r| !geo.row_hidden(r)).collect();
            let cols: Vec<u32> = (c1..=c2).filter(|&c| !geo.col_hidden(c)).collect();
            let title_rows: Vec<u32> = page
                .title_rows
                .map_or_else(Vec::new, |(a, b)| (a..=b).collect());
            let title_cols: Vec<u32> = page
                .title_cols
                .map_or_else(Vec::new, |(a, b)| (a..=b).collect());
            let row_groups = split_pages(&rows, &title_rows, &page.row_breaks, ah, &row_h);
            let col_groups = split_pages(&cols, &title_cols, &page.col_breaks, aw, &col_w);
            // Down, then over.
            for cg in &col_groups {
                for rg in &row_groups {
                    pages.push(PrintPage {
                        rows: rg.clone(),
                        cols: cg.clone(),
                    });
                }
            }
        }
        (pages, scale)
    }

    /// File > Print: counts the pages the printer would get.
    pub fn file_print(&mut self) -> Result<usize, String> {
        let n = self.print_pages().len();
        if n == 0 {
            self.status = "We didn't find anything to print.".to_owned();
            return Err(self.status.clone());
        }
        self.status = format!("{n} page{} to print", if n == 1 { "" } else { "s" });
        Ok(n)
    }

    /// File > Export > Create PDF: `path`, or the chosen file; `Ok(None)` when cancelled.
    pub fn file_export_pdf(
        &mut self,
        path: Option<&Path>,
    ) -> Result<Option<(PathBuf, usize)>, String> {
        let path = match path {
            Some(p) => p.to_path_buf(),
            None => {
                let stem = self
                    .path()
                    .and_then(|p| p.file_stem().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| "Book1".to_owned());
                match self
                    .ext_file
                    .pdf_chooser
                    .and_then(|c| c(&format!("{stem}.pdf")))
                {
                    Some(p) => p,
                    None => return Ok(None),
                }
            }
        };
        let path = if path.extension().is_none() {
            path.with_extension("pdf")
        } else {
            path
        };
        let (pages, _) = self.print_layout();
        if pages.is_empty() {
            self.status = "We didn't find anything to print.".to_owned();
            return Err(self.status.clone());
        }
        let bytes = self.pdf_bytes();
        let tmp = path.with_extension("pdf.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        Ok(Some((path, pages.len())))
    }

    /// What the header and footer field codes print for the active sheet.
    #[must_use]
    pub fn header_context(&self) -> Context {
        let names = self.sheet_names();
        let path = self.path();
        let file = path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| "Book1".to_owned(), |n| n.to_string_lossy().into_owned());
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        Context {
            sheet: names.get(self.sheet).cloned().unwrap_or_default(),
            path: path.map_or_else(|| file.clone(), |p| p.to_string_lossy().into_owned()),
            file,
            user: std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .ok()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| "Author".to_owned()),
            date: hf::date_text(unix),
            time: hf::time_text(unix),
        }
    }

    /// The header and footer of page `index` (0-based) of `count`, as page content in points
    /// with the origin at the page's bottom-left corner. The first page uses the first-page
    /// header when "Different first page" is on, even pages the even header when "Different odd
    /// & even pages" is on.
    fn header_footer_stream(
        &self,
        page: &SheetPage,
        index: usize,
        count: usize,
        (pw, ph): (f64, f64),
    ) -> String {
        let h = &page.header_footer;
        let number = index + 1;
        let (header, footer) = if h.different_first && index == 0 {
            (&h.first_header, &h.first_footer)
        } else if h.different_odd_even && number.is_multiple_of(2) {
            (&h.even_header, &h.even_footer)
        } else {
            (&h.header, &h.footer)
        };
        let ctx = self.header_context();
        let (page_no, pages_no) = (number.to_string(), count.to_string());
        let m = page.margins;
        let (x0, x1) = if h.align_with_margins {
            (m.left * 72.0, pw - m.right * 72.0)
        } else {
            (0.0, pw)
        };
        let line_h = FONT_PT + 4.0;
        let mut s = format!(
            "q 1 0 0 1 0 {ph:.2} cm
"
        );
        for (sections, y) in [
            (header, m.header * 72.0),
            (footer, ph - m.footer * 72.0 - line_h),
        ] {
            for (i, code) in sections.iter().enumerate() {
                let text = hf::render(code, &ctx, &page_no, &pages_no);
                if text.is_empty() {
                    continue;
                }
                let width: f64 = text.chars().map(char_em).sum::<f64>() * FONT_PT + 4.0;
                match i {
                    0 => cell_text(&mut s, &text, x0, y, x1 - x0, line_h, false),
                    1 => cell_text(
                        &mut s,
                        &text,
                        x0 + ((x1 - x0 - width) / 2.0).max(0.0),
                        y,
                        width,
                        line_h,
                        false,
                    ),
                    _ => cell_text(&mut s, &text, x0, y, x1 - x0, line_h, true),
                }
            }
        }
        s.push_str(
            "Q
",
        );
        s
    }

    /// The PDF of the printed pages.
    pub fn pdf_bytes(&self) -> Vec<u8> {
        let (pages, scale) = self.print_layout();
        let page = self.page();
        let geo = self.geometry();
        let (pw, ph) = self.paper_pt();
        let m = page.margins;
        let heading = if page.print_headings {
            (HEADING_W, HEADING_H)
        } else {
            (0.0, 0.0)
        };
        let mut streams = Vec::new();
        for (index, p) in pages.iter().enumerate() {
            let mut s = self.header_footer_stream(&page, index, pages.len(), (pw, ph));
            // Origin: the top-left of the printable area, scaled.
            let _ = writeln!(
                s,
                "q {scale:.4} 0 0 {scale:.4} {:.3} {:.3} cm",
                m.left * 72.0,
                ph - m.top * 72.0
            );
            let _ = writeln!(s, "0.5 w 0.6 G 0 g");
            let mut x = heading.0;
            let mut xs = Vec::new();
            for &c in &p.cols {
                let w = f64::from(geo.col_px(c)) * PT_PER_PX;
                xs.push((c, x, w));
                x += w;
            }
            let mut y = heading.1;
            let mut ys = Vec::new();
            for &r in &p.rows {
                let h = f64::from(geo.row_px(r)) * PT_PER_PX;
                ys.push((r, y, h));
                y += h;
            }
            if page.print_headings {
                for &(c, x, w) in &xs {
                    cell_text(&mut s, &col_letters(c), x, 0.0, w, HEADING_H, false);
                    let _ = writeln!(s, "{x:.2} {:.2} {w:.2} {:.2} re S", 0.0, -HEADING_H);
                }
                for &(r, y, h) in &ys {
                    cell_text(&mut s, &r.to_string(), 0.0, y, HEADING_W, h, false);
                    let _ = writeln!(s, "0 {:.2} {HEADING_W:.2} {:.2} re S", -y, -h);
                }
            }
            for &(r, y, h) in &ys {
                for &(c, x, w) in &xs {
                    if page.print_gridlines {
                        let _ = writeln!(s, "{x:.2} {:.2} {w:.2} {:.2} re S", -y, -h);
                    }
                    if let Some((text, numeric)) = self.shown(r, c) {
                        cell_text(&mut s, &text, x, y, w, h, numeric);
                    }
                }
            }
            s.push_str("Q\n");
            streams.push(s);
        }
        build_pdf(&streams, pw, ph)
    }
}

/// Splits `items` into pages of at most `limit` points: a manual break starts a page, an
/// automatic one starts where the next item does not fit, and `titles` repeat on the pages
/// after the one that holds them.
fn split_pages(
    items: &[u32],
    titles: &[u32],
    breaks: &std::collections::BTreeSet<u32>,
    limit: f64,
    size: &dyn Fn(u32) -> f64,
) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = Vec::new();
    let mut cur: Vec<u32> = Vec::new();
    let mut used = 0.0;
    for &i in items {
        let w = size(i);
        let manual = breaks.contains(&i) && !cur.is_empty();
        let overflow = used + w > limit && !cur.is_empty();
        if manual || overflow {
            out.push(std::mem::take(&mut cur));
            used = 0.0;
            // Titles above this item repeat at the top of the new page.
            for &t in titles.iter().filter(|&&t| t < i) {
                cur.push(t);
                used += size(t);
            }
        }
        if !cur.contains(&i) {
            cur.push(i);
            used += w;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(Vec::new());
    }
    out
}

fn col_letters(c: u32) -> String {
    open365_xlsx::col_letters(c.clamp(1, MAX_COL))
}

/// Approximate Helvetica advance in em.
fn char_em(ch: char) -> f64 {
    match ch {
        '0'..='9' => 0.556,
        ' ' | '.' | ',' | ':' | ';' | '!' | '|' => 0.278,
        '-' | '(' | ')' => 0.333,
        'i' | 'j' | 'l' => 0.222,
        'm' | 'w' | 'M' | 'W' | '%' => 0.833,
        'A'..='Z' => 0.667,
        _ => 0.5,
    }
}

/// Draws `text` clipped to the cell `(x, y, w, h)` (`y` down from the page origin).
fn cell_text(s: &mut String, text: &str, x: f64, y: f64, w: f64, h: f64, right: bool) {
    let width: f64 = text.chars().map(char_em).sum::<f64>() * FONT_PT;
    let tx = if right {
        (x + w - 2.0 - width).max(x + 1.0)
    } else {
        x + 2.0
    };
    let base = -(y + h - (h - FONT_PT) / 2.0 - 2.5);
    let _ = write!(
        s,
        "q {x:.2} {:.2} {w:.2} {h:.2} re W n BT /F1 {FONT_PT} Tf {tx:.2} {base:.2} Td (",
        -(y + h)
    );
    for ch in text.chars() {
        match ch {
            '(' | ')' | '\\' => {
                s.push('\\');
                s.push(ch);
            }
            c if (' '..='~').contains(&c) => s.push(c),
            c if (c as u32) < 256 && (c as u32) >= 160 => {
                let _ = write!(s, "\\{:03o}", c as u32);
            }
            _ => s.push('?'),
        }
    }
    s.push_str(") Tj ET Q\n");
}

/// A PDF 1.4 file of `streams` (one page each) on `pw` by `ph` points.
fn build_pdf(streams: &[String], pw: f64, ph: f64) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let n = streams.len();
    // Objects: 1 catalog, 2 pages, 3 font, then (page, content) pairs.
    let kids: String = (0..n).map(|i| format!("{} 0 R ", 4 + 2 * i)).collect();
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    for (i, s) in streams.iter().enumerate() {
        objs.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                5 + 2 * i
            )
            .into_bytes(),
        );
        let mut c = format!("<< /Length {} >>\nstream\n", s.len()).into_bytes();
        c.extend_from_slice(s.as_bytes());
        c.extend_from_slice(b"endstream");
        objs.push(c);
    }
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}
