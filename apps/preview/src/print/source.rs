//! What the print page needs from a document: its page setup, a way to change it, and a PDF of
//! its pages as the layout pipeline lays them out.
//!
//! Writer lays pages out with `open365-layout` and writes them with `open365-pdf` (through the
//! command layer's `share_send_pdf`, which never changes the document). Sheets pages the sheet by
//! its page-layout settings and writes them with its own PDF writer. Both give the print page
//! the same thing: a PDF whose pages are what the printer prints.

use open365_commands::{Command, LayoutOp};
use open365_print::{MarginPreset, Paper, Scaling};

use crate::doc::WriterPane;
use crate::sheet::SheetModel;
use crate::sheet::sheet_ext::page_layout::{Margins, Paper as SheetPaper};

/// The page setup a document holds, as the print page shows it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSetup {
    /// Paper size in points in the document's orientation (width, height).
    pub paper_pt: (f64, f64),
    pub landscape: bool,
    /// Margins in points: top, bottom, left, right.
    pub margins_pt: (f64, f64, f64, f64),
    /// Pages wide and tall to fit (0 is automatic) and the percentage; Sheets only.
    pub fit: (u32, u32),
    pub scale_percent: u32,
}

/// A document the print page can print.
pub trait PrintSource {
    fn product(&self) -> open365_print::Product;

    /// The job name the print queue shows.
    fn job_name(&self) -> String;

    /// The page being viewed (1-based), for Print Current Page.
    fn current_page(&self) -> usize;

    fn page_setup(&self) -> PageSetup;

    /// Changes the paper size, keeping the orientation.
    ///
    /// # Errors
    /// The reason the document refused.
    fn set_paper(&mut self, paper: &Paper) -> Result<(), String>;

    /// # Errors
    /// The reason the document refused.
    fn set_landscape(&mut self, landscape: bool) -> Result<(), String>;

    /// # Errors
    /// The reason the document refused.
    fn set_margins(&mut self, preset: &MarginPreset) -> Result<(), String>;

    /// Sheets only; Writer ignores it.
    fn set_scaling(&mut self, scaling: Scaling) -> Result<(), String>;

    /// A PDF of the pages as they print now.
    ///
    /// # Errors
    /// The reason the pages could not be laid out or written.
    fn render_pdf(&mut self) -> Result<Vec<u8>, String>;
}

// ---- Writer ---------------------------------------------------------------------------------

/// Print source over the Writer pane. Page setup changes edit the document, as Word's Print page
/// does.
#[derive(Debug)]
pub struct WriterSource<'a>(pub &'a mut WriterPane);

impl WriterSource<'_> {
    fn sections(&self) -> Vec<usize> {
        (0..self.0.section_count()).collect()
    }
}

impl PrintSource for WriterSource<'_> {
    fn product(&self) -> open365_print::Product {
        open365_print::Product::Writer
    }

    fn job_name(&self) -> String {
        self.0.path().and_then(|p| p.file_name()).map_or_else(
            || "Document1".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    fn current_page(&self) -> usize {
        let para = self.0.caret().para;
        self.0
            .paragraph_pages()
            .get(para)
            .map_or(1, |p| *p as usize)
            .max(1)
    }

    fn page_setup(&self) -> PageSetup {
        let sec = self.0.caret_section().unwrap_or_default();
        let t = |v: i32| f64::from(v) / 20.0;
        PageSetup {
            paper_pt: (
                f64::from(sec.page.width_pt()),
                f64::from(sec.page.height_pt()),
            ),
            landscape: sec.page.width_twips > sec.page.height_twips,
            margins_pt: (
                t(sec.margins.top_twips),
                t(sec.margins.bottom_twips),
                t(sec.margins.left_twips),
                t(sec.margins.right_twips),
            ),
            fit: (0, 0),
            scale_percent: 100,
        }
    }

    fn set_paper(&mut self, paper: &Paper) -> Result<(), String> {
        // The pane stores the portrait size; the orientation op swaps it when landscape.
        let landscape = self.page_setup().landscape;
        let (w, h) = if landscape {
            (paper.height_pt, paper.width_pt)
        } else {
            (paper.width_pt, paper.height_pt)
        };
        let sections = self.sections();
        let code = open365_commands::layout_ops::paper_sizes()
            .iter()
            .find(|p| p.id == paper.id)
            .map(|p| p.code);
        if self.0.layout_apply(LayoutOp::PageSize {
            sections,
            width_pt: w,
            height_pt: h,
            code,
        }) {
            Ok(())
        } else {
            Err(self
                .0
                .status
                .clone()
                .unwrap_or_else(|| "The paper size was refused".to_owned()))
        }
    }

    fn set_landscape(&mut self, landscape: bool) -> Result<(), String> {
        let sections = self.sections();
        if self.0.layout_apply(LayoutOp::Orientation {
            sections,
            landscape,
        }) {
            Ok(())
        } else {
            Err(self
                .0
                .status
                .clone()
                .unwrap_or_else(|| "The orientation was refused".to_owned()))
        }
    }

    fn set_margins(&mut self, preset: &MarginPreset) -> Result<(), String> {
        let sections = self.sections();
        if self.0.layout_apply(LayoutOp::Margins {
            sections,
            top_pt: preset.top,
            bottom_pt: preset.bottom,
            left_pt: preset.left,
            right_pt: preset.right,
            gutter_pt: None,
            mirrored: Some(preset.mirrored),
        }) {
            Ok(())
        } else {
            Err(self
                .0
                .status
                .clone()
                .unwrap_or_else(|| "The margins were refused".to_owned()))
        }
    }

    fn set_scaling(&mut self, _scaling: Scaling) -> Result<(), String> {
        Ok(())
    }

    fn render_pdf(&mut self) -> Result<Vec<u8>, String> {
        let out = self.0.apply_share(Command::ShareSendPdf {
            path: None,
            overwrite: false,
        })?;
        let path = out
            .saved_to
            .ok_or_else(|| "The pages were not written".to_owned())?;
        let bytes =
            std::fs::read(&path).map_err(|e| format!("The pages could not be read back: {e}"));
        // The copy was only for this job.
        let _ = std::fs::remove_file(&path);
        bytes
    }
}

// ---- Sheets ---------------------------------------------------------------------------------

/// Print source over the Sheets model, for the active sheet.
#[derive(Debug)]
pub struct SheetsSource<'a>(pub &'a mut SheetModel);

impl PrintSource for SheetsSource<'_> {
    fn product(&self) -> open365_print::Product {
        open365_print::Product::Sheets
    }

    fn job_name(&self) -> String {
        self.0
            .path()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Book1".to_owned())
    }

    fn current_page(&self) -> usize {
        1
    }

    fn page_setup(&self) -> PageSetup {
        let p = self.0.page();
        let (w, h) = p.paper.inches();
        let (w, h) = if p.landscape { (h, w) } else { (w, h) };
        PageSetup {
            paper_pt: (w * 72.0, h * 72.0),
            landscape: p.landscape,
            margins_pt: (
                p.margins.top * 72.0,
                p.margins.bottom * 72.0,
                p.margins.left * 72.0,
                p.margins.right * 72.0,
            ),
            fit: (p.fit_width, p.fit_height),
            scale_percent: p.scale,
        }
    }

    fn set_paper(&mut self, paper: &Paper) -> Result<(), String> {
        let sheet_paper = SheetPaper::from_key(paper.id)
            .ok_or_else(|| format!("{} is not a worksheet paper size", paper.label))?;
        self.0.set_paper(sheet_paper);
        Ok(())
    }

    fn set_landscape(&mut self, landscape: bool) -> Result<(), String> {
        self.0.set_landscape(landscape);
        Ok(())
    }

    fn set_margins(&mut self, preset: &MarginPreset) -> Result<(), String> {
        let keep = self.0.page().margins;
        self.0.set_margins(Margins {
            top: preset.top / 72.0,
            bottom: preset.bottom / 72.0,
            left: preset.left / 72.0,
            right: preset.right / 72.0,
            header: keep.header,
            footer: keep.footer,
        });
        Ok(())
    }

    fn set_scaling(&mut self, scaling: Scaling) -> Result<(), String> {
        let base = 100;
        let (wide, tall, percent) = scaling.page_setup(base);
        // Setting the percentage clears the fit counts, so set it first.
        self.0.set_print_scale(percent);
        self.0.set_fit_width(wide);
        self.0.set_fit_height(tall);
        Ok(())
    }

    fn render_pdf(&mut self) -> Result<Vec<u8>, String> {
        if self.0.print_pages().is_empty() {
            return Err("We didn't find anything to print.".to_owned());
        }
        Ok(self.0.pdf_bytes())
    }
}
