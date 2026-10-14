//! The state of the print page, without any drawing: the settings, the printers, the PDF the
//! pages come from and the preview position. Everything the page shows is derived here, so it
//! is tested without a window.

use open365_print::{
    JobOptions, MarginPreset, PdfSource, PrintSettings, PrintSystem, PrinterInfo, Product,
    RangeMode, Scaling, SequenceError, SubmitReport, compose, margin_presets, paper_by_id,
    paper_matching,
};

use super::source::PrintSource;

/// Id of the "paper the document already has" choice, used when the document's size is not in
/// the paper list.
pub const DOCUMENT_PAPER: &str = "document";
/// Id of the margins choice for margins that match no preset.
pub const CUSTOM_MARGINS: &str = "custom";

/// What the print page edits.
#[derive(Debug)]
pub struct PrintModel {
    pub product: Product,
    pub settings: PrintSettings,
    pub printers: Vec<PrinterInfo>,
    /// The laid-out pages; `None` until rendered and after a page-setup change.
    pdf: Option<PdfSource>,
    /// Why the pages could not be rendered.
    pub pdf_error: Option<String>,
    /// Zero-based sheet shown in the preview.
    pub viewing: usize,
    /// The last message for the user (a refused job, a sent job).
    pub message: Option<String>,
    pub job_name: String,
    pub current_page: usize,
    /// The document's own paper size in points (portrait order), when it is not a listed size.
    document_paper: Option<(f64, f64)>,
}

impl PrintModel {
    /// Reads the document's page setup and the installed printers.
    pub fn new(source: &dyn PrintSource, system: &dyn PrintSystem) -> Self {
        let setup = source.page_setup();
        let product = source.product();
        let (w, h) = setup.paper_pt;
        let paper = paper_matching(w, h);
        let (pw, ph) = if w <= h { (w, h) } else { (h, w) };
        let margins_id = margin_presets(product)
            .iter()
            .find(|m| close(m, setup.margins_pt))
            .map_or(CUSTOM_MARGINS, |m| m.id);
        let scaling = match setup.fit {
            (1, 1) => Scaling::FitSheet,
            (1, 0) => Scaling::FitColumns,
            (0, 1) => Scaling::FitRows,
            _ if setup.scale_percent != 100 => Scaling::Custom(setup.scale_percent),
            _ => Scaling::None,
        };
        let printers = system.printers();
        let settings = PrintSettings {
            printer: printers
                .iter()
                .find(|p| p.is_default)
                .or(printers.first())
                .map(|p| p.system_name.clone()),
            landscape: setup.landscape,
            paper_id: paper.map_or(DOCUMENT_PAPER, |p| p.id).to_owned(),
            margins_id: margins_id.to_owned(),
            scaling,
            ..PrintSettings::default()
        };
        Self {
            product,
            settings,
            printers,
            pdf: None,
            pdf_error: None,
            viewing: 0,
            message: None,
            job_name: source.job_name(),
            current_page: source.current_page(),
            document_paper: paper.is_none().then_some((pw, ph)),
        }
    }

    /// Renders the pages if the page setup changed since the last render.
    pub fn refresh(&mut self, source: &mut dyn PrintSource) {
        if self.pdf.is_some() || self.pdf_error.is_some() {
            return;
        }
        self.current_page = source.current_page();
        match source
            .render_pdf()
            .and_then(|b| PdfSource::load(&b).map_err(|e| e.to_string()))
        {
            Ok(pdf) => self.pdf = Some(pdf),
            Err(e) => self.pdf_error = Some(e),
        }
        let last = self.sheet_count().saturating_sub(1);
        self.viewing = self.viewing.min(last);
    }

    fn invalidate(&mut self) {
        self.pdf = None;
        self.pdf_error = None;
    }

    #[must_use]
    pub fn pdf(&self) -> Option<&PdfSource> {
        self.pdf.as_ref()
    }

    #[must_use]
    pub fn total_pages(&self) -> usize {
        self.pdf.as_ref().map_or(0, PdfSource::page_count)
    }

    /// The paper size of a sheet in points, in the chosen orientation.
    #[must_use]
    pub fn sheet_size(&self) -> (f64, f64) {
        let (w, h) = paper_by_id(&self.settings.paper_id)
            .map(|p| (p.width_pt, p.height_pt))
            .or(self.document_paper)
            .unwrap_or((595.3, 841.9));
        if self.settings.landscape {
            (h, w)
        } else {
            (w, h)
        }
    }

    // ---- page setup changes (they reach the document) -----------------------------------------

    pub fn set_paper(&mut self, source: &mut dyn PrintSource, id: &str) {
        let Some(paper) = paper_by_id(id) else { return };
        match source.set_paper(paper) {
            Ok(()) => {
                self.settings.paper_id = id.to_owned();
                self.invalidate();
            }
            Err(e) => self.message = Some(e),
        }
    }

    pub fn set_landscape(&mut self, source: &mut dyn PrintSource, landscape: bool) {
        match source.set_landscape(landscape) {
            Ok(()) => {
                self.settings.landscape = landscape;
                self.invalidate();
            }
            Err(e) => self.message = Some(e),
        }
    }

    pub fn set_margins(&mut self, source: &mut dyn PrintSource, id: &str) {
        let Some(preset) = margin_presets(self.product).iter().find(|m| m.id == id) else {
            return;
        };
        match source.set_margins(preset) {
            Ok(()) => {
                self.settings.margins_id = id.to_owned();
                self.invalidate();
            }
            Err(e) => self.message = Some(e),
        }
    }

    pub fn set_scaling(&mut self, source: &mut dyn PrintSource, scaling: Scaling) {
        match source.set_scaling(scaling) {
            Ok(()) => {
                self.settings.scaling = scaling;
                self.invalidate();
            }
            Err(e) => self.message = Some(e),
        }
    }

    // ---- choices that only change the job -----------------------------------------------------

    pub fn set_range_mode(&mut self, mode: RangeMode) {
        self.settings.range_mode = mode;
        self.viewing = 0;
    }

    pub fn set_range_text(&mut self, text: &str) {
        self.settings.range_text = text.to_owned();
        self.viewing = 0;
    }

    pub fn set_per_sheet(&mut self, n: u8) {
        self.settings.per_sheet = n;
        self.viewing = 0;
    }

    // ---- derived ------------------------------------------------------------------------------

    /// The pages one copy prints, 1-based.
    ///
    /// # Errors
    /// [`SequenceError`] for a bad range or an empty document.
    pub fn selected_pages(&self) -> Result<Vec<usize>, SequenceError> {
        self.settings.pages(self.total_pages(), self.current_page)
    }

    /// The pages of each sheet one copy prints (0-based source pages).
    #[must_use]
    pub fn sheets(&self) -> Vec<Vec<usize>> {
        let n = usize::from(self.settings.per_sheet.max(1));
        self.selected_pages()
            .map(|pages| {
                let zero: Vec<usize> = pages.iter().map(|p| p - 1).collect();
                zero.chunks(n).map(<[usize]>::to_vec).collect()
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub fn sheet_count(&self) -> usize {
        self.sheets().len()
    }

    pub fn step(&mut self, delta: isize) {
        let last = self.sheet_count().saturating_sub(1);
        self.viewing = self.viewing.saturating_add_signed(delta).min(last);
    }

    /// The text under the Print button: the page and sheet counts, or why nothing prints.
    #[must_use]
    pub fn summary(&self) -> String {
        if let Some(e) = &self.pdf_error {
            return e.clone();
        }
        match self.selected_pages() {
            Err(e) => e.to_string(),
            Ok(pages) => {
                let s = &self.settings;
                let sheets = s.sheets_for(pages.len()) * s.copies.max(1) as usize;
                format!(
                    "{} page{} on {} sheet{} of paper",
                    pages.len(),
                    if pages.len() == 1 { "" } else { "s" },
                    sheets,
                    if sheets == 1 { "" } else { "s" },
                )
            }
        }
    }

    #[must_use]
    pub fn printer(&self) -> Option<&PrinterInfo> {
        let name = self.settings.printer.as_deref()?;
        self.printers.iter().find(|p| p.system_name == name)
    }

    /// A warning about the chosen printer, if there is one.
    #[must_use]
    pub fn printer_note(&self) -> Option<String> {
        let p = self.printer()?;
        if !p.reads_pdf {
            return Some(
                "This printer's driver may not print PDF files directly. Use Export to PDF if nothing comes out."
                    .to_owned(),
            );
        }
        None
    }

    // ---- the job ------------------------------------------------------------------------------

    /// The finished job PDF: the chosen pages, copies counted, placed on sheets.
    ///
    /// # Errors
    /// The reason there is nothing to print.
    pub fn job_pdf(&self) -> Result<Vec<u8>, String> {
        let pdf = self.pdf.as_ref().ok_or_else(|| {
            self.pdf_error
                .clone()
                .unwrap_or_else(|| "Nothing to print.".to_owned())
        })?;
        let sequence = self
            .settings
            .sequence(pdf.page_count(), self.current_page)
            .map_err(|e| e.to_string())?;
        compose(pdf, &sequence, self.sheet_size(), self.settings.per_sheet)
            .map_err(|e| e.to_string())
    }

    #[must_use]
    pub fn job_options(&self) -> JobOptions {
        JobOptions {
            name: self.job_name.clone(),
            sides: self.settings.sides,
            media: paper_by_id(&self.settings.paper_id).map(|p| p.ipp_media),
        }
    }

    /// Sends the job. Nothing leaves the machine unless this is called.
    ///
    /// # Errors
    /// The message the user sees.
    pub fn print(&mut self, system: &mut dyn PrintSystem) -> Result<SubmitReport, String> {
        let result = self.send(system);
        self.message = Some(match &result {
            Ok(r) if r.ignored.is_empty() => format!("Sent to {}.", self.printer_name()),
            Ok(r) => format!(
                "Sent to {}. The system print path does not pass {} on; set them in the printer's own settings.",
                self.printer_name(),
                r.ignored.join(" and "),
            ),
            Err(e) => e.clone(),
        });
        result
    }

    fn send(&self, system: &mut dyn PrintSystem) -> Result<SubmitReport, String> {
        let printer = self
            .settings
            .printer
            .clone()
            .ok_or_else(|| open365_print::SubmitError::NoPrinter.to_string())?;
        let pdf = self.job_pdf()?;
        system
            .submit(&printer, &pdf, &self.job_options())
            .map_err(|e| e.to_string())
    }

    fn printer_name(&self) -> String {
        self.printer().map_or_else(String::new, |p| p.name.clone())
    }
}

fn close(m: &MarginPreset, (t, b, l, r): (f64, f64, f64, f64)) -> bool {
    [(m.top, t), (m.bottom, b), (m.left, l), (m.right, r)]
        .iter()
        .all(|(x, y)| (x - y).abs() < 0.6)
}
