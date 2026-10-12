//! Writer ribbon: the Insert tab (`insert.*`): page breaks, tables, pictures, links, comments,
//! header and footer, text and symbols. The work is in the pane (`doc/writer/ext_insert.rs`)
//! and the command layer (`WriterInsertOp`, `DocxInsert`).
//!
//! A control that needs a value takes it typed (`insert.tables.table=3x4`) or as a menu choice
//! after its id; there is no dialog host yet, so the plain id uses Word's default or says what
//! to type:
//! - `table`: `RxC` rows by columns (default 2 rows by 5 columns, the dialog's default).
//! - `pictures`: a file path (the plain id asks with the file dialog).
//! - `link`: an address; the selection becomes the link (or `address|text` with no selection).
//!   `bookmark:Name` links to a bookmark.
//! - `bookmark`: the name. `comment`: the text (empty by default).
//! - `header`, `footer`: `blank` (default), `remove`, `text:Some text`, or the text itself.
//! - `page-number`: `top-left`, `top-centre`, `top-right`, `bottom-left`, `bottom-centre` (the
//!   default), `bottom-right`.
//! - `date-time`: a date picture (`dd/MM/yyyy` by default). `symbol`: a character or `U+00A9`.
//!
//! Not wired, each needs a subsystem of its own: Cover Page, Shapes, Icons, 3D Models, Diagram,
//! Chart, Screenshot, Online Videos, Cross-reference, Text Box, Quick Parts, Styled Text, Drop
//! Cap, Signature Line, Object, Equation, eSignature fields.

use std::path::PathBuf;

use crate::doc::WriterPane;
use crate::ribbon::{Handled, RibbonCtx, base};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "insert.pages.blank-page",
    "insert.pages.page-break",
    "insert.tables.table",
    "insert.illustrations.pictures",
    "insert.links.link",
    "insert.links.bookmark",
    "insert.comments.comment",
    "insert.header-footer.header",
    "insert.header-footer.footer",
    "insert.header-footer.page-number",
    "insert.text.date-time",
    "insert.symbols.symbol",
];

/// The control `id` belongs to and the menu choice after it (`run` is no choice).
fn split(id: &str) -> Option<(&'static str, String)> {
    IDS.iter().find_map(|c| {
        let rest = id.strip_prefix(c)?;
        if rest.is_empty() {
            return Some((*c, String::new()));
        }
        let choice = rest.strip_prefix('.')?;
        Some((
            *c,
            if choice == "run" {
                String::new()
            } else {
                choice.to_owned()
            },
        ))
    })
}

/// `3x4` or `3 x 4`: rows by columns.
fn grid(text: &str) -> Option<(usize, usize)> {
    let (r, c) = text
        .to_ascii_lowercase()
        .split_once('x')
        .map(|(r, c)| (r.trim().to_owned(), c.trim().to_owned()))?;
    let (r, c) = (r.parse::<usize>().ok()?, c.parse::<usize>().ok()?);
    ((1..=32_767).contains(&r) && (1..=63).contains(&c)).then_some((r, c))
}

fn header_footer(p: &mut WriterPane, footer: bool, arg: &str) {
    match arg {
        "" | "blank" => {
            p.insert_header_footer(footer, Some(""));
        }
        "remove" => {
            p.insert_header_footer(footer, None);
        }
        other => {
            let text = other.strip_prefix("text:").unwrap_or(other);
            p.insert_header_footer(footer, Some(text));
        }
    }
}

fn link(p: &mut WriterPane, arg: &str) {
    if let Some(name) = arg.strip_prefix("bookmark:") {
        p.insert_bookmark_link(name.trim());
        return;
    }
    if arg.is_empty() {
        // Ctrl+K on selected text that is an address links it.
        let sel = p.selected_text();
        let t = sel.trim();
        if ["http://", "https://", "mailto:", "www."]
            .iter()
            .any(|s| t.starts_with(s))
        {
            p.insert_link(t, None);
        } else {
            p.status = Some("Link: type the address after the control id, for example insert.links.link=https://example.org".to_owned());
        }
        return;
    }
    match arg.split_once('|') {
        Some((url, text)) => p.insert_link(url, Some(text)),
        None => p.insert_link(arg, None),
    };
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let typed = id.split_once('=').map(|(_, v)| v.trim().to_owned());
    let Some((ctl, choice)) = split(base(id)) else {
        return Handled::No;
    };
    let arg = typed.unwrap_or(choice);
    let Some(p) = ctx.writer.as_deref_mut() else {
        return Handled::Yes;
    };
    match ctl {
        "insert.pages.blank-page" => p.insert_blank_page(),
        "insert.pages.page-break" => p.insert_page_break(),
        "insert.tables.table" => match grid(&arg).or_else(|| arg.is_empty().then_some((2, 5))) {
            Some((rows, cols)) => p.insert_table(rows, cols),
            None => p.status = Some(format!("{arg} is not a table size; use rows x columns")),
        },
        "insert.illustrations.pictures" => {
            let path = if arg.is_empty() {
                WriterPane::choose_picture_path()
            } else {
                Some(PathBuf::from(&arg))
            };
            if let Some(path) = path {
                p.insert_picture(&path);
            }
        }
        "insert.links.link" => link(p, &arg),
        "insert.links.bookmark" => {
            if arg.is_empty() {
                p.status = Some("Bookmark: type the name after the control id, for example insert.links.bookmark=Intro".to_owned());
            } else if !p.insert_bookmark(&arg) && p.status.is_none() {
                p.status = Some("The bookmark name is not valid".to_owned());
            }
        }
        "insert.comments.comment" => {
            p.insert_comment(&arg);
        }
        "insert.header-footer.header" => header_footer(p, false, &arg),
        "insert.header-footer.footer" => header_footer(p, true, &arg),
        "insert.header-footer.page-number" => {
            p.insert_page_number_at(if arg.is_empty() {
                "bottom-centre"
            } else {
                &arg
            });
        }
        "insert.text.date-time" => {
            let picture = if arg.is_empty() {
                "dd/MM/yyyy"
            } else {
                arg.as_str()
            };
            p.insert_date_time_now(picture);
        }
        "insert.symbols.symbol" => {
            if !arg.is_empty() {
                p.insert_symbol(&arg);
            }
        }
        _ => return Handled::No,
    }
    *ctx.refocus_doc = true;
    Handled::Yes
}
