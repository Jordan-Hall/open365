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
//! - `date-time`: a date picture (`dd/MM/yyyy` by default); `auto:picture` inserts a `DATE`
//!   field that updates. `symbol`: a character, `U+00A9` or a special character name
//!   (`nbsp`, `em-dash`, `copyright`...).
//! - `cover-page`: `classic` (default), `banner`, `stripe`.
//! - `table`: also `quick-list`, `quick-matrix`, `quick-schedule` (the Quick Tables gallery).
//! - `cross-reference`: `[text|page|number|above:]target`, a bookmark or heading text.
//! - `text-box`: `simple` (sample text, the default) or `draw` (empty box).
//! - `quick-parts`: `field:CODE` (the Field dialog) or `property:title|author|...`.
//! - `object`: `text-from-file` (the file dialog) or `text-from-file:path`.
//! - `page-number`: also `current` (a `PAGE` field at the caret).
//!
//! Not wired, each needs a subsystem of its own: Shapes, Icons, 3D Models, Diagram, Chart,
//! Screenshot, Online Videos (drawing crate work or media), Styled Text, Drop Cap, Signature
//! Line, Equation, eSignature fields, and the building-block half of Quick Parts.

use std::path::PathBuf;

use crate::doc::WriterPane;
use crate::ribbon::{Handled, RibbonCtx, base};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "insert.pages.cover-page",
    "insert.pages.blank-page",
    "insert.pages.page-break",
    "insert.tables.table",
    "insert.illustrations.pictures",
    "insert.links.link",
    "insert.links.bookmark",
    "insert.links.cross-reference",
    "insert.comments.comment",
    "insert.header-footer.header",
    "insert.header-footer.footer",
    "insert.header-footer.page-number",
    "insert.text.text-box",
    "insert.text.quick-parts",
    "insert.text.date-time",
    "insert.text.object",
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

/// The character of a special character name, if it is one.
fn special(name: &str) -> Option<&'static str> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "nbsp" | "nonbreaking-space" => "\u{a0}",
        "nb-hyphen" | "nonbreaking-hyphen" => "\u{2011}",
        "opt-hyphen" | "optional-hyphen" => "\u{ad}",
        "em-dash" => "\u{2014}",
        "en-dash" => "\u{2013}",
        "em-space" => "\u{2003}",
        "en-space" => "\u{2002}",
        "copyright" => "\u{a9}",
        "registered" => "\u{ae}",
        "trademark" => "\u{2122}",
        "section" => "\u{a7}",
        "paragraph" => "\u{b6}",
        "ellipsis" => "\u{2026}",
        "open-quote" => "\u{2018}",
        "close-quote" => "\u{2019}",
        "open-double-quote" => "\u{201c}",
        "close-double-quote" => "\u{201d}",
        _ => return None,
    })
}

/// The field code of a Quick Parts document property.
fn property_field(name: &str) -> Option<&'static str> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "title" => "TITLE",
        "author" => "AUTHOR",
        "subject" => "SUBJECT",
        "comments" => "COMMENTS",
        "keywords" => "KEYWORDS",
        "filename" => "FILENAME",
        "company" => "DOCPROPERTY Company",
        _ => return None,
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
        "insert.pages.cover-page" => {
            p.insert_cover_page(&arg);
        }
        "insert.pages.blank-page" => p.insert_blank_page(),
        "insert.pages.page-break" => p.insert_page_break(),
        "insert.tables.table" if arg.starts_with("quick-") => {
            p.insert_quick_table(&arg["quick-".len()..]);
        }
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
        "insert.links.cross-reference" => {
            p.insert_cross_reference(&arg);
        }
        "insert.text.text-box" => match arg.as_str() {
            "" | "simple" => {
                p.insert_text_box(true);
            }
            "draw" => {
                p.insert_text_box(false);
            }
            other => p.status = Some(format!("{other} is not a text box")),
        },
        "insert.text.quick-parts" => {
            if let Some(code) = arg.strip_prefix("field:") {
                p.insert_field(code);
            } else if let Some(code) = arg.strip_prefix("property:").and_then(property_field) {
                p.insert_field(code);
            } else {
                p.status = Some("Quick Parts: type field:CODE or property:title after the control id, for example insert.text.quick-parts=field:PAGE".to_owned());
            }
        }
        "insert.text.object" => {
            let file = arg
                .strip_prefix("text-from-file")
                .map(|r| r.trim_start_matches(':'));
            match file {
                Some("") => {
                    if let Some(path) = WriterPane::choose_text_file_path() {
                        p.insert_text_from_file(&path);
                    }
                }
                Some(path) => {
                    p.insert_text_from_file(&PathBuf::from(path));
                }
                None if arg.is_empty() => {
                    if let Some(path) = WriterPane::choose_text_file_path() {
                        p.insert_text_from_file(&path);
                    }
                }
                None => {
                    p.status = Some("Object: only Text from File is available".to_owned());
                }
            }
        }
        "insert.comments.comment" => {
            p.insert_comment(&arg);
        }
        "insert.header-footer.header" => header_footer(p, false, &arg),
        "insert.header-footer.footer" => header_footer(p, true, &arg),
        "insert.header-footer.page-number" if arg == "current" => {
            p.insert_field("PAGE");
        }
        "insert.header-footer.page-number" => {
            p.insert_page_number_at(if arg.is_empty() {
                "bottom-centre"
            } else {
                &arg
            });
        }
        "insert.text.date-time" if arg.starts_with("auto:") => {
            let picture = arg["auto:".len()..].trim();
            let picture = if picture.is_empty() {
                "dd/MM/yyyy"
            } else {
                picture
            };
            p.insert_field(&format!("DATE \\@ \"{picture}\""));
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
                p.insert_symbol(special(&arg).unwrap_or(&arg));
            }
        }
        _ => return Handled::No,
    }
    *ctx.refocus_doc = true;
    Handled::Yes
}
