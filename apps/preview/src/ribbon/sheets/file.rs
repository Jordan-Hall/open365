//! Sheets ribbon: the File tab. Backstage entries are not ribbon controls (the spec has no
//! `file.*` ids), so this module claims no id in `IDS`; the shell runs `file.new`, `file.open`,
//! `file.save`, `file.save-as`, `file.print`, `file.export-pdf` and `file.close` for the
//! Backstage list. A typed `=path` replaces the dialog (`file.open=C:\a.xlsx`).

use std::path::Path;

use crate::ribbon::{Handled, RibbonCtx, base};
use crate::shell::WinAct;

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[];

fn arg(id: &str) -> Option<&Path> {
    id.split_once('=').map(|(_, p)| Path::new(p))
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let s = &mut *ctx.sheet;
    let result: Result<(), String> = match base(id) {
        "file.new" => s.file_new(),
        "file.open" => s.file_open(arg(id)).map(|_| ()),
        "file.save" => s.file_save().map(|_| ()),
        "file.save-as" => s.file_save_as(arg(id)).map(|_| ()),
        "file.print" => s.file_print().map(|_| ()),
        "file.export-pdf" => s.file_export_pdf(arg(id)).map(|_| ()),
        "file.close" => {
            if s.is_dirty() {
                Err("Save the changes to this workbook first".to_owned())
            } else {
                ctx.out.push(WinAct::Close);
                Ok(())
            }
        }
        _ => return Handled::No,
    };
    if let Err(e) = result {
        ctx.sheet.status = e;
    }
    Handled::Yes
}
