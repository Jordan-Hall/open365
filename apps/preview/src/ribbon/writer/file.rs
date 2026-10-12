//! Writer ribbon: the File tab (`file.*`): New, Open, Save, Save As, Print, Export to PDF,
//! Share and Close.
//!
//! The spec has no `file.*` controls (Backstage items are `Act::Nav` in the shell), so `IDS`
//! stays empty and the `ribbon_dispatch` coverage test is unaffected; the shell can route a
//! Backstage item to `file.<item>` through `run_command`. `file.open=<path>` and
//! `file.save-as=<path>` take a typed path instead of showing the file chooser.

use std::path::PathBuf;

use crate::doc::WriterPane;
use crate::ribbon::{Handled, RibbonCtx, base};
use crate::shell::WinAct;

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[];

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let typed = id.split_once('=').map(|(_, v)| PathBuf::from(v.trim()));
    let id = base(id);
    if !matches!(
        id,
        "file.new"
            | "file.open"
            | "file.save"
            | "file.save-as"
            | "file.print"
            | "file.export"
            | "file.export-pdf"
            | "file.share"
            | "file.close"
    ) {
        return Handled::No;
    }
    let Some(p) = ctx.writer.as_deref_mut() else {
        return Handled::Yes;
    };
    match id {
        "file.new" | "file.open" => {
            if p.is_dirty() {
                p.status =
                    Some("Save or close the changes before opening another document".to_owned());
            } else if id == "file.new" {
                p.replace_document(ctx.ui, WriterPane::blank());
            } else if let Some(path) = typed.or_else(WriterPane::choose_open_path) {
                match WriterPane::open(&path) {
                    Ok(new) => p.replace_document(ctx.ui, new),
                    Err(e) => p.status = Some(format!("Open failed: {e}")),
                }
            }
        }
        "file.save" => {
            p.save();
        }
        "file.save-as" => {
            match typed {
                Some(path) => p.save_to(path),
                None => p.save_as(),
            };
        }
        "file.export" | "file.export-pdf" => {
            p.export_pdf(typed);
        }
        "file.print" => {
            p.status = Some("Printing is not available yet; use Export to PDF".to_owned())
        }
        "file.share" => p.status = Some("Use the Share button to send a copy or a PDF".to_owned()),
        "file.close" => {
            if p.is_dirty() {
                p.status = Some("Save the changes before closing".to_owned());
            } else {
                ctx.out.push(WinAct::Close);
            }
        }
        _ => return Handled::No,
    }
    *ctx.refocus_doc = true;
    Handled::Yes
}
