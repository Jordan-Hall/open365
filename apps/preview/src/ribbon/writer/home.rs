//! Writer ribbon: the Home tab (`home.*`): clipboard, font, paragraph, styles and editing.
//!
//! A control can carry a menu choice after its id (`home.font.change-case.upper-case`) or a
//! typed value (`home.font.font-colour=FF0000`); the plain id and `<id>.run` run the control's
//! default action, the main part of a split button. The work is in the pane
//! (`doc/writer/ext_home.rs`) and the command layer (`WriterHomeOp`).
//!
//! Choices by control:
//! - `change-case`: `sentence-case`, `lowercase`, `uppercase`, `capitalize-each-word`,
//!   `toggle-case`.
//! - `underline`: `single`, `double`, `thick`, `dotted`, `dash`, `wave`... or `none`.
//! - `text-highlight-colour`, `font-colour`, `shading`: a colour name or `RRGGBB`; `auto` or
//!   `none` for no colour.
//! - `line-and-paragraph-spacing`: `1.0`, `1.15`, `1.5`, `2.0`... or `add-space-before`,
//!   `remove-space-before`, `add-space-after`, `remove-space-after`.
//! - `borders`: `bottom`, `top`, `left`, `right`, `none`, `all`, `outside`.
//! - `sort`: `descending`.
//! - `select`: `select-all`.
//!
//! Not wired: Text Effects and Typography, Dictate, Add-ins (need their own subsystems).

use open365_commands::{BorderKind, CaseMode, ListKind};

use crate::doc::{Format, WriterPane};
use crate::ribbon::{Handled, RibbonCtx, base};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
/// `home.styles.<name>` gallery ids are accepted too (see [`Format::from_ribbon`]).
pub const IDS: &[&str] = &[
    "home.clipboard.paste",
    "home.clipboard.cut",
    "home.clipboard.copy",
    "home.clipboard.format-painter",
    "home.font.font",
    "home.font.font-size",
    "home.font.bold",
    "home.font.italic",
    "home.font.underline",
    "home.font.strikethrough",
    "home.font.subscript",
    "home.font.superscript",
    "home.font.grow-font",
    "home.font.shrink-font",
    "home.font.change-case",
    "home.font.clear-formatting",
    "home.font.text-highlight-colour",
    "home.font.font-colour",
    "home.paragraph.bullets",
    "home.paragraph.numbering",
    "home.paragraph.multi-level-list",
    "home.paragraph.decrease-indent",
    "home.paragraph.increase-indent",
    "home.paragraph.sort",
    "home.paragraph.show-all",
    "home.paragraph.align-left",
    "home.paragraph.centre",
    "home.paragraph.align-right",
    "home.paragraph.justify",
    "home.paragraph.line-and-paragraph-spacing",
    "home.paragraph.shading",
    "home.paragraph.borders",
    "home.styles.quickstylesgallery",
    "home.editing.find",
    "home.editing.replace",
    "home.editing.select",
    "home.proofread.proofread",
];

/// The control `id` belongs to and the menu choice after it (`run` and empty are no choice).
fn split(id: &str) -> Option<(&'static str, String)> {
    IDS.iter().find_map(|c| {
        let rest = id.strip_prefix(c)?;
        if rest.is_empty() {
            return Some((*c, String::new()));
        }
        let choice = rest.strip_prefix('.')?;
        let choice = if choice == "run" { "" } else { choice };
        Some((*c, choice.to_ascii_lowercase().replace('_', "-")))
    })
}

fn case_mode(choice: &str) -> Option<CaseMode> {
    Some(match choice {
        "sentence-case" | "sentence" => CaseMode::Sentence,
        "lowercase" | "lower-case" | "lower" => CaseMode::Lower,
        "uppercase" | "upper-case" | "upper" => CaseMode::Upper,
        "capitalize-each-word" | "capitalise-each-word" | "capitalize" => CaseMode::CapitalizeEach,
        "toggle-case" | "toggle" => CaseMode::Toggle,
        _ => return None,
    })
}

fn border_kind(choice: &str) -> Option<BorderKind> {
    Some(match choice {
        "bottom" | "bottom-border" => BorderKind::Bottom,
        "top" | "top-border" => BorderKind::Top,
        "left" | "left-border" => BorderKind::Left,
        "right" | "right-border" => BorderKind::Right,
        "none" | "no-border" => BorderKind::None,
        "all" | "all-borders" | "outside" | "outside-borders" | "box" => BorderKind::Box,
        _ => return None,
    })
}

fn spacing(p: &mut WriterPane, choice: &str) {
    match choice {
        "add-space-before" => p.set_paragraph_space(Some(12.0), None),
        "remove-space-before" => p.set_paragraph_space(Some(0.0), None),
        "add-space-after" => p.set_paragraph_space(None, Some(12.0)),
        "remove-space-after" => p.set_paragraph_space(None, Some(0.0)),
        other => match other.parse::<f64>() {
            Ok(m) if (0.06..=132.0).contains(&m) => p.set_line_spacing(m),
            _ => p.status = Some(format!("{other} is not a line spacing")),
        },
    }
}

fn colour(p: &mut WriterPane, ctl: &str, choice: &str) {
    match ctl {
        "home.font.font-colour" => {
            let c = if choice.is_empty() {
                p.home_state().font_colour
            } else {
                choice.to_owned()
            };
            p.set_font_colour(&c);
        }
        "home.font.text-highlight-colour" => match choice {
            "" => match p.home_state().highlight {
                Some(c) => p.set_highlight(&c),
                None => p.set_highlight("none"),
            },
            "auto" => p.set_highlight("none"),
            c => p.set_highlight(c),
        },
        _ => match choice {
            "" => {
                let c = p.home_state().shading;
                p.set_shading(&c);
            }
            c => p.set_shading(c),
        },
    }
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let typed = id.split_once('=').map(|(_, v)| v.trim().to_owned());
    let plain = base(id).to_owned();
    // Gallery cells (`home.styles.heading-1`) and a typed style name.
    if let Some(rest) = plain.strip_prefix("home.styles.")
        && (rest != "quickstylesgallery" || typed.is_some())
    {
        let f = Format::from_ribbon(&plain).or_else(|| {
            typed
                .as_deref()
                .and_then(Format::style_by_name)
                .map(|s| Format::Style(s.to_owned()))
        });
        if let Some(f) = f {
            if let Some(p) = ctx.writer.as_deref_mut() {
                p.format(f);
                *ctx.refocus_doc = true;
            }
            return Handled::Yes;
        }
        if rest != "quickstylesgallery" {
            return Handled::No;
        }
    }
    let Some((ctl, mut choice)) = split(&plain) else {
        return Handled::No;
    };
    if let Some(t) = &typed {
        choice = t.to_ascii_lowercase().replace('_', "-");
    }
    if ctl == "home.proofread.proofread" {
        return crate::ribbon::dispatch(ctx, "review.proofing.proofread");
    }
    let Some(p) = ctx.writer.as_deref_mut() else {
        return Handled::Yes;
    };
    match ctl {
        "home.clipboard.paste" => p.paste_plain(),
        "home.clipboard.cut" => p.cut(),
        "home.clipboard.copy" => p.copy(),
        "home.clipboard.format-painter" => p.format_painter(),
        "home.font.grow-font" => p.grow_font(true),
        "home.font.shrink-font" => p.grow_font(false),
        "home.font.change-case" => {
            if let Some(mode) = case_mode(&choice) {
                p.change_case(mode);
            }
        }
        "home.font.clear-formatting" => p.clear_formatting(),
        "home.font.underline" if !choice.is_empty() => p.set_underline_style(&choice),
        "home.font.text-highlight-colour" | "home.font.font-colour" | "home.paragraph.shading" => {
            // Menu entries that open a palette have no colour yet.
            if !matches!(
                choice.as_str(),
                "theme"
                    | "standard"
                    | "more"
                    | "theme-colours"
                    | "standard-colours"
                    | "more-colours"
            ) {
                colour(p, ctl, &choice);
            }
        }
        "home.paragraph.bullets" => p.toggle_list(ListKind::Bullet),
        "home.paragraph.numbering" => p.toggle_list(ListKind::Number),
        "home.paragraph.multi-level-list" => p.toggle_list(ListKind::Multilevel),
        "home.paragraph.decrease-indent" => p.change_indent(false),
        "home.paragraph.increase-indent" => p.change_indent(true),
        "home.paragraph.sort" => {
            p.sort_paragraphs(matches!(choice.as_str(), "descending" | "desc" | "z-to-a"))
        }
        "home.paragraph.show-all" => p.toggle_show_all(),
        "home.paragraph.line-and-paragraph-spacing" => {
            if !choice.is_empty() {
                spacing(p, &choice);
            }
        }
        "home.paragraph.borders" => {
            let kind = if choice.is_empty() {
                Some(p.home_state().border)
            } else {
                border_kind(&choice)
            };
            if let Some(k) = kind {
                p.set_border(k);
            }
        }
        "home.editing.find" => p.find_open(),
        "home.editing.replace" => p.replace_open(),
        "home.editing.select" => {
            if matches!(choice.as_str(), "select-all" | "all") {
                p.select_all();
            }
        }
        "home.styles.quickstylesgallery" => {}
        _ => {
            if let Some(f) = Format::from_ribbon(id) {
                p.format(f);
            }
        }
    }
    *ctx.refocus_doc = true;
    Handled::Yes
}
