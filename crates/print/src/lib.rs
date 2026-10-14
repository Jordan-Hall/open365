//! File > Print back end (ADR-018): everything a print page needs that is not drawing.
//!
//! The editors produce a PDF of the pages (Writer through the layout and `open365-pdf`, Sheets
//! through its page-layout code). This crate turns that PDF and a [`PrintSettings`] into the job
//! the printer gets:
//!
//! * [`range`] parses page-range text such as "1-3, 5, 8-".
//! * [`paper`] holds the paper sizes and the margin presets of both products.
//! * [`settings`] holds the choices of the print page and derives the page sequence (copies,
//!   collate) from them.
//! * [`compose`] cuts the pages out of the PDF, repeats them for copies, places them on sheets
//!   (N pages per sheet, fit to the paper) and draws a coarse "ink map" of a page for the live
//!   preview.
//! * [`system`] hands the finished PDF to the operating system (`printers`: winspool, CUPS)
//!   behind a trait, so the print page is tested without a printer.
//!
//! Nothing here prints on its own: [`system::OsPrintSystem::submit`] is the only call that sends
//! a job, and only the caller's Print button reaches it.

pub mod compose;
pub mod paper;
pub mod range;
pub mod settings;
pub mod system;

pub use compose::{Ink, InkItem, PdfError, PdfSource, Placement, compose, grid};
pub use paper::{
    MarginPreset, Paper, Product, margin_presets, paper_by_id, paper_matching, papers,
};
pub use range::{RangeError, parse_page_ranges};
pub use settings::{PER_SHEET, PrintSettings, RangeMode, Scaling, SequenceError, Sides};
pub use system::{
    JobOptions, MemoryPrintSystem, OsPrintSystem, PrintSystem, PrinterInfo, PrinterState,
    SubmitError, SubmitReport, Submitted,
};
