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

mod clipboard;
mod command;
mod conditional;
mod error;
mod eval;
mod ext;
mod find;
mod format;
mod merge;
mod pdf;
mod share;
mod structure;
mod target;
mod template;
mod xlsx_state;

pub use clipboard::{Clip, GridClip, docx_clip, grid_copy, parse_tsv, paste_rows};
pub use command::{CellEntry, CellRef, Command, CommandLog, EffectClass, Outcome};
pub use conditional::{CfEffect, ConditionalFormats, DataBarEffect, IconEffect};
pub use error::CommandError;
pub use eval::{
    EvalValue, Evaluated, FormulaEngine, MAX_FORMULA_CHARS, MAX_RESULT_DIM, MAX_RESULT_TEXT,
    evaluate_formula,
};
pub use ext::data::{self as data_ops, DataOp};
pub use ext::header_footer::{HeaderFooterRequest, header_footer};
pub use ext::insert_more::{CoverDesign, InsertMoreOp, XRefKind};
pub use ext::mailings::{self as mailings_ops, MailingsOp};
pub use ext::references_more::{self as references_more, EntrySeg, MoreOp};
pub use ext::review::{self as review_ops, ReviewOp};
pub use ext::sheet_objects::{self as objects_ops, ObjectsOp};
pub use ext::sheet_page::{self as sheet_page_ops, SheetPageOp};
pub use ext::sheets_home::{HomeOp, SortLevel};
pub use ext::writer_design::{self as design_ops, DesignOp};
pub use ext::writer_home::{BorderKind, CaseMode, ListKind, WriterHomeOp, char_format_xml};
pub use ext::writer_insert::{HfAlign, HfKind, LinkTarget, WriterInsertOp, to_hex};
pub use ext::writer_layout::{self as layout_ops, LayoutOp};
pub use ext::writer_margins::SectionMargins;
pub use ext::writer_references::{FieldScope, RefOp};
pub use ext::writer_review::WriterReviewOp;
pub use ext::writer_search::{self as search_ops, ReplaceOne};
pub use find::{
    DocMatch, FindError, FindOptions, FindScope, LookIn, PlannedEdit, XlsxFind, cell_display_text,
    cell_source_text, excel_matches, next_match, xlsx_find_all, xlsx_replace_plan,
};
pub use format::{
    AlignmentChange, BorderChange, CellFormatChange, ColorSpec, FillChange, FontChange, HAlign,
    LineSpec, NumberFormatSpec, RangeRef, RotationSpec, UnderlineKind, VAlign,
};
pub use merge::{MergeMode, merge_discards, merge_plan};
pub use open365_docx::{Alignment, DocPos, RunPropsPatch, TabAlign, VertAlign};
pub use open365_pdf::{Conformance, PdfMetadata};
pub use pdf::PdfOptions;
pub use share::{
    HiddenItem, HiddenKind, HiddenReport, SHARE_TEMP_PREFIX, Sanitized, inspect_bytes,
    remove_share_temp, sanitize_bytes, sweep_share_temp,
};
pub use target::{Group, Target, apply};
pub use template::template_output_extension;
