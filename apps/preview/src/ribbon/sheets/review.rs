//! Sheets ribbon: the Review tab (`review.*`).
//!
//! Every edit is a `ReviewOp` command (see `sheet_ext::review`), so protection and notes are one
//! undo step each. Dialog controls ask the shell for their dialog
//! (`SheetModel::ext_review.take_request()`) and take the dialog's answer as a value after `=`:
//!
//! - `review.protect.protect-sheet` protects (or, when protected, unprotects) the sheet;
//!   `=password=abc;sort;format-cells;no-select-locked` sets options, `=unprotect=abc` unprotects
//! - `review.protect.protect-workbook` toggles structure protection; `=password=abc`
//! - `review.comments.new-comment=text` adds a threaded comment (a reply when the cell has one)
//! - `review.notes.notes` edits the note of the active cell (new when it has none): `=text`;
//!   the items `.new`, `.edit`, `.delete`, `.previous`, `.next` are the Notes menu
//! - `review.notes.notes.convert-to-comments` turns every note of the workbook into a comment thread
//! - `review.protect.allow-edit-ranges=add:Title|A1:B3|password`, `=modify:Old|Title|A1:B3|password`
//!   and `=delete:Title` change the ranges users may edit on an unprotected sheet (the dialog that
//!   collects them is not built; a plain click only reports how many ranges there are)
//! - `review.proofing.spelling=en-GB` checks the sheet in that language
//!
//! Not wired (left to `Handled::No`): Thesaurus (no thesaurus data), Check Performance, Translate
//! (network service), Show Changes (legacy track changes) and Unshare Workbook (legacy shared
//! workbooks; neither has an engine, both are replaced by co-authoring), Hide Ink (no ink).

use open365_commands::review_ops::SheetProtection;

use crate::ribbon::{Handled, RibbonCtx, base};
use crate::sheet::SheetModel;
use crate::sheet::sheet_ext::review::{Request, Severity};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "review.proofing.spelling",
    "review.proofing.workbook-statistics",
    "review.accessibility.check-accessibility",
    "review.comments.new-comment",
    "review.comments.delete",
    "review.comments.previous-comment",
    "review.comments.next-comment",
    "review.comments.show-comments",
    "review.notes.notes",
    "review.protect.protect-sheet",
    "review.protect.protect-workbook",
    "review.protect.allow-edit-ranges",
];

fn split(id: &str) -> Option<(&'static str, &str, Option<&str>)> {
    let b = base(id);
    let ctrl = IDS
        .iter()
        .find(|c| b == **c || b.strip_prefix(**c).is_some_and(|r| r.starts_with('.')))?;
    let sub = b[ctrl.len()..].trim_start_matches('.');
    let arg = id.split_once('=').map(|(_, a)| a);
    Some((ctrl, sub, arg))
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let Some((ctrl, sub, arg)) = split(id) else {
        return Handled::No;
    };
    let m = &mut *ctx.sheet;
    if let Err(e) = run(m, ctrl, sub, arg) {
        m.status = e;
    }
    Handled::Yes
}

fn run(m: &mut SheetModel, ctrl: &str, sub: &str, arg: Option<&str>) -> Result<(), String> {
    match ctrl.rsplit('.').next().unwrap_or(ctrl) {
        "spelling" => {
            let issues = m.review_spell_check(arg.unwrap_or("en-US"));
            if !issues.is_empty() {
                m.ext_review.request(Request::Spelling);
            }
            Ok(())
        }
        "workbook-statistics" => {
            let s = m.review_statistics();
            m.status = s
                .lines()
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join(", ");
            m.ext_review.request(Request::Statistics);
            Ok(())
        }
        "check-accessibility" => {
            let issues = m.review_accessibility();
            let count = |s: Severity| issues.iter().filter(|i| i.severity == s).count();
            m.status = if issues.is_empty() {
                "No accessibility issues found.".to_owned()
            } else {
                format!(
                    "Accessibility: {} errors, {} warnings, {} tips",
                    count(Severity::Error),
                    count(Severity::Warning),
                    count(Severity::Tip)
                )
            };
            m.ext_review.request(Request::Accessibility);
            Ok(())
        }
        "new-comment" => m.review_add_comment(arg.unwrap_or_default()),
        "delete" => m.review_delete_comment(),
        "previous-comment" => m.review_goto_comment(false),
        "next-comment" => m.review_goto_comment(true),
        "show-comments" => {
            m.ext_review.show_comments = !m.ext_review.show_comments;
            Ok(())
        }
        "notes" => notes(m, sub, arg),
        "protect-sheet" => protect_sheet(m, arg),
        "protect-workbook" => protect_workbook(m, arg),
        "allow-edit-ranges" => edit_ranges(m, arg),
        _ => Ok(()),
    }
}

fn notes(m: &mut SheetModel, sub: &str, arg: Option<&str>) -> Result<(), String> {
    let (row, col) = m.active;
    match sub {
        "delete" => m.review_delete_note(),
        "previous" => m.review_goto_note(false),
        "next" => m.review_goto_note(true),
        "convert-to-comments" | "convert" => m.review_convert_notes_to_comments(),
        "" | "run" | "new" | "edit" => {
            let has = m.review_note_at(row, col).is_some();
            if sub == "new" && has {
                return Err("The cell already has a note.".to_owned());
            }
            if sub == "edit" && !has {
                return Err("The cell has no note.".to_owned());
            }
            m.review_set_note(arg.unwrap_or_default())
        }
        _ => Err("This Notes command is not available yet.".to_owned()),
    }
}

fn protect_sheet(m: &mut SheetModel, arg: Option<&str>) -> Result<(), String> {
    let mut options = SheetProtection::default();
    let mut password = None;
    let mut unprotect = None;
    for part in arg.unwrap_or_default().split(';').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        match k {
            "password" => password = Some(v.to_owned()),
            "unprotect" => unprotect = Some(v.to_owned()),
            "no-select-locked" => options.select_locked_cells = false,
            "no-select-unlocked" => options.select_unlocked_cells = false,
            "format-cells" => options.format_cells = true,
            "format-columns" => options.format_columns = true,
            "format-rows" => options.format_rows = true,
            "insert-columns" => options.insert_columns = true,
            "insert-rows" => options.insert_rows = true,
            "insert-hyperlinks" => options.insert_hyperlinks = true,
            "delete-columns" => options.delete_columns = true,
            "delete-rows" => options.delete_rows = true,
            "sort" => options.sort = true,
            "auto-filter" => options.use_auto_filter = true,
            "pivot-tables" => options.use_pivot_tables = true,
            "edit-objects" => options.edit_objects = true,
            "edit-scenarios" => options.edit_scenarios = true,
            other => return Err(format!("Unknown option '{other}'.")),
        }
    }
    if m.review_sheet_protection().is_some() {
        return m.review_unprotect_sheet(unprotect.as_deref());
    }
    if arg.is_none() {
        m.ext_review.request(Request::ProtectSheet);
    }
    m.review_protect_sheet(options, password.as_deref())
}

fn protect_workbook(m: &mut SheetModel, arg: Option<&str>) -> Result<(), String> {
    let password = arg
        .and_then(|a| a.strip_prefix("password="))
        .map(str::to_owned);
    if m.review_workbook_protected() {
        let given = arg.and_then(|a| a.strip_prefix("unprotect="));
        return m.review_unprotect_workbook(given);
    }
    if arg.is_none() {
        m.ext_review.request(Request::ProtectWorkbook);
    }
    m.review_protect_workbook(password.as_deref())
}

/// Allow Edit Ranges: `add:Title|A1:B3|password`, `modify:Old|Title|A1:B3|password`,
/// `delete:Title`. Without an argument it only reports the ranges.
fn edit_ranges(m: &mut SheetModel, arg: Option<&str>) -> Result<(), String> {
    let Some(arg) = arg else {
        let n = m.review_edit_ranges().len();
        m.status = format!("{n} ranges can be edited on this sheet.");
        return Ok(());
    };
    let (verb, rest) = arg.split_once(':').unwrap_or((arg, ""));
    let f: Vec<&str> = rest.split('|').collect();
    let field = |i: usize| f.get(i).copied().unwrap_or_default();
    match verb {
        "add" => m.review_add_edit_range(field(0), field(1), Some(field(2))),
        "modify" => m.review_modify_edit_range(field(0), field(1), field(2), Some(field(3))),
        "delete" => m.review_delete_edit_range(field(0)),
        other => Err(format!("Unknown Allow Edit Ranges action '{other}'.")),
    }
}
