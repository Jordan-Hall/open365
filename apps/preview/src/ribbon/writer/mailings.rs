//! Writer ribbon: the Mailings tab (`mailings.*`). No control is wired yet.
//!
//! What the engines have: `MERGEFIELD` is evaluated by `Document::update_fields` when the
//! `FieldContext` carries a `MergeData` source, and the field insertion primitives of
//! `open365_docx` (`insert_inline_xml`) can place a merge field.
//!
//! What is missing, in the order a merge needs it:
//! 1. A recipient list: reading a CSV, an XLSX sheet (through `open365_xlsx`) or a typed list into
//!    a table, plus the Edit Recipient List, Select Recipients and Match Fields dialogs.
//! 2. Merge state on the Writer pane (main document type, list, record number, preview on/off).
//!    `WriterPane` has no place to keep it and `Target` has no merge command, so Preview Results,
//!    First/Previous/Next/Last Record, Find Recipient and Check for Errors have nothing to act on.
//! 3. Address Block and Greeting Line: Word writes them as `ADDRESSBLOCK` / `GREETINGLINE` fields
//!    with a format string, which the field engine does not evaluate yet.
//! 4. Finish & Merge: one document per record (Edit Individual Documents), print and e-mail; this
//!    needs the merge state above and a way to open a new document from a `Target`.
//! 5. Envelopes and Labels: the dialogs, the vendor label tables (page and cell sizes) and the
//!    section/table builder that lays a label sheet out.
//! 6. Highlight Merge Fields, Rules (`IF`, `ASK`, `NEXT`...) and Update Labels.
//!
//! Every control of the tab except Start Mail Merge is disabled in the spec until a merge has
//! started, so none of them is reachable before item 2 exists.

use crate::ribbon::{Handled, RibbonCtx};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[];

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(_ctx: &mut RibbonCtx<'_>, _id: &str) -> Handled {
    Handled::No
}
