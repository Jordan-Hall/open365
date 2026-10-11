//! Typed command layer: every mutation is a scoped, audited command. See ADR-002.
//!
//! The UI, the CLI and any future agent interface build a [`Command`] and hand it to
//! [`apply`]. The command declares its [`EffectClass`], so a permission layer can decide
//! before anything runs. Applied commands are appended to a JSON-serialisable log and
//! recorded on an undo/redo stack.
//!
//! Undo strategy: spreadsheet edits capture the previous cell (cheap) and recalculate on
//! undo. DOCX edits capture the affected paragraphs' XML before the edit and put them back on
//! undo (cost proportional to the edit). When the paragraphs cannot be captured, undo falls
//! back to reopening the original bytes and replaying the remaining commands.

mod command;
mod conditional;
mod error;
mod eval;
mod pdf;
mod target;
mod template;
mod xlsx_state;

pub use command::{CellEntry, CellRef, Command, CommandLog, EffectClass, Outcome};
pub use conditional::{CfEffect, ConditionalFormats, DataBarEffect, IconEffect};
pub use error::CommandError;
pub use eval::{
    EvalValue, Evaluated, FormulaEngine, MAX_FORMULA_CHARS, MAX_RESULT_DIM, MAX_RESULT_TEXT,
    evaluate_formula,
};
pub use open365_docx::{Alignment, DocPos, RunPropsPatch};
pub use open365_pdf::PdfMetadata;
pub use pdf::PdfOptions;
pub use target::{Target, apply};
pub use template::template_output_extension;
