//! File > Print for Writer and Sheets (ADR-018, `docs/architecture/printing.md`).
//!
//! * [`source`]: what the page needs from a document (page setup, a PDF of the pages).
//! * [`model`]: the settings, the printers and the derived page sequence, without drawing.
//! * [`page`]: the Backstage Print page: settings, live preview and the Print button.
//!
//! The job is the PDF of the open document. `open365-print` selects and arranges its pages and
//! hands the result to the operating system's print service.

pub mod model;
pub mod page;
pub mod source;

pub use model::PrintModel;
pub use page::{Ctl, PrintPage};
pub use source::{PageSetup, PrintSource, SheetsSource, WriterSource};
