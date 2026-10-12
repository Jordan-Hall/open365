//! Writer ribbon: the References tab (`references.*`): table of contents, footnotes and endnotes,
//! captions, index and table of authorities. The work is in the pane
//! (`doc/writer/ext_references.rs`) and the command layer.
//!
//! A control can carry a menu choice after its id (`references.footnotes.next-footnote.next-endnote`);
//! the plain id and `<id>.run` run the control's default action.
//!
//! Not wired: Insert Citation, Manage Sources, Style, Bibliography and Change Provider (the
//! sources list lives in a custom XML part the document model does not hold), Show Notes (no
//! notes pane), Cross-reference (needs the reference dialog).

use crate::ribbon::{Handled, RibbonCtx};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "references.table-of-contents.table-of-contents",
    "references.table-of-contents.add-text",
    "references.table-of-contents.update-table",
    "references.footnotes.insert-footnote",
    "references.footnotes.insert-endnote",
    "references.footnotes.next-footnote",
    "references.captions.insert-caption",
    "references.captions.insert-table-of-figures",
    "references.captions.update-table",
    "references.index.mark-entry",
    "references.index.insert-index",
    "references.index.update-index",
    "references.table-of-authorities.mark-citation",
    "references.table-of-authorities.insert-table-of-authorities",
    "references.table-of-authorities.update-table",
];

/// The control `id` belongs to and the menu choice after it (`""` for the control itself).
fn split(id: &str) -> Option<(&'static str, &str)> {
    let id = crate::ribbon::base(id);
    IDS.iter().find_map(|c| {
        if id == *c {
            Some((*c, ""))
        } else {
            id.strip_prefix(*c)
                .and_then(|r| r.strip_prefix('.'))
                .map(|v| (*c, v))
        }
    })
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let Some((control, choice)) = split(id) else {
        return Handled::No;
    };
    let choice = if choice == "run" { "" } else { choice };
    let Some(p) = ctx.writer.as_deref_mut() else {
        return Handled::Yes;
    };
    let done = match (control, choice) {
        ("references.table-of-contents.table-of-contents", "" | "automatic-1") => {
            p.insert_toc_automatic_1()
        }
        ("references.table-of-contents.table-of-contents", "automatic-2") => {
            p.insert_toc_automatic_2()
        }
        ("references.table-of-contents.table-of-contents", "custom") => p.insert_toc_custom(),
        ("references.table-of-contents.table-of-contents", "remove") => p.remove_toc(),
        ("references.table-of-contents.add-text", "do-not-show") => p.toc_add_text(0),
        ("references.table-of-contents.add-text", "level-1") => p.toc_add_text(1),
        ("references.table-of-contents.add-text", "level-2") => p.toc_add_text(2),
        ("references.table-of-contents.add-text", "level-3") => p.toc_add_text(3),
        ("references.table-of-contents.add-text", "") => true,
        ("references.table-of-contents.update-table", "") => p.update_toc(),
        ("references.footnotes.insert-footnote", "") => p.insert_note(false),
        ("references.footnotes.insert-endnote", "") => p.insert_note(true),
        ("references.footnotes.next-footnote", "" | "next-footnote") => {
            p.step_note(true, false).is_some()
        }
        ("references.footnotes.next-footnote", "previous-footnote") => {
            p.step_note(false, false).is_some()
        }
        ("references.footnotes.next-footnote", "next-endnote") => p.step_note(true, true).is_some(),
        ("references.footnotes.next-footnote", "previous-endnote") => {
            p.step_note(false, true).is_some()
        }
        ("references.captions.insert-caption", "") => p.insert_caption(None),
        ("references.captions.insert-table-of-figures", "") => p.insert_table_of_figures("Figure"),
        ("references.captions.update-table", "") => p.update_table_of_figures(),
        ("references.index.mark-entry", "") => p.mark_index_entry(),
        ("references.index.insert-index", "") => p.insert_index(),
        ("references.index.update-index", "") => p.update_index(),
        ("references.table-of-authorities.mark-citation", "") => p.mark_citation(),
        ("references.table-of-authorities.insert-table-of-authorities", "") => {
            p.insert_table_of_authorities()
        }
        ("references.table-of-authorities.update-table", "") => p.update_table_of_authorities(),
        _ => return Handled::No,
    };
    if done {
        *ctx.refocus_doc = true;
    }
    Handled::Yes
}
