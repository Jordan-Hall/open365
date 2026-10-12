//! Sheets ribbon: the Page Layout tab (`page-layout.*`).
//!
//! Wired: Themes (the choice), Page Set-up (margins, orientation, size, print area, breaks,
//! background, print titles), Scale to Fit and Sheet Options. Dropdown items arrive as
//! `<dropdown id>.<item>` (`page-layout.page-set-up.size.a4`) or as `<dropdown id>#<n>`.
//! Left: the Arrange group (it needs drawing objects with a z-order).

use crate::ribbon::{Handled, RibbonCtx, base};
use crate::sheet::sheet_ext::page_layout::{Margins, Paper, parse_pages};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "page-layout.themes.themes",
    "page-layout.themes.colours",
    "page-layout.themes.fonts",
    "page-layout.themes.effects",
    "page-layout.page-set-up.margins",
    "page-layout.page-set-up.orientation",
    "page-layout.page-set-up.size",
    "page-layout.page-set-up.print-area",
    "page-layout.page-set-up.breaks",
    "page-layout.page-set-up.background",
    "page-layout.page-set-up.print-titles",
    "page-layout.scale-to-fit.width",
    "page-layout.scale-to-fit.height",
    "page-layout.scale-to-fit.scale",
    "page-layout.sheet-options.view",
    "page-layout.sheet-options.print",
    "page-layout.sheet-options.view-2",
    "page-layout.sheet-options.print-2",
];

const MARGIN_ITEMS: &[&str] = &["normal", "wide", "narrow", "custom"];
const ORIENTATION_ITEMS: &[&str] = &["portrait", "landscape"];
const SIZE_ITEMS: &[&str] = &[
    "letter",
    "tabloid",
    "legal",
    "statement",
    "executive",
    "a3",
    "a4",
    "a5",
    "b4",
    "b5",
];
const AREA_ITEMS: &[&str] = &["set", "add", "clear"];
const BREAK_ITEMS: &[&str] = &["insert", "remove", "reset"];

/// `(control id, menu item)` for `id`; the item comes from `.<item>`, `=<value>` or `#<n>`.
fn split(id: &str) -> Option<(&'static str, Option<String>)> {
    let b = base(id);
    let (control, tail) = IDS.iter().find_map(|c| {
        if b == *c {
            Some((*c, ""))
        } else {
            b.strip_prefix(c)
                .and_then(|r| r.strip_prefix('.'))
                .map(|t| (*c, t))
        }
    })?;
    let item = if !tail.is_empty() {
        // `custom=0.5,0.5,0.5,0.5` keeps its value: the suffix after `=` is part of the item.
        Some(match id.split_once('=') {
            Some((_, v)) => format!("{tail}={v}"),
            None => tail.to_owned(),
        })
    } else if let Some((_, v)) = id.split_once('=') {
        Some(v.to_owned())
    } else if let Some((_, n)) = id.rsplit_once('#') {
        let n = n.parse::<usize>().ok();
        match control {
            // Fit-to-pages lists: Automatic, then 1 to 9 pages.
            "page-layout.scale-to-fit.width" | "page-layout.scale-to-fit.height" => n.map(|i| {
                if i == 0 {
                    "Automatic".to_owned()
                } else {
                    format!("{i} page")
                }
            }),
            _ => {
                let items: &[&str] = match control {
                    "page-layout.page-set-up.margins" => MARGIN_ITEMS,
                    "page-layout.page-set-up.orientation" => ORIENTATION_ITEMS,
                    "page-layout.page-set-up.size" => SIZE_ITEMS,
                    "page-layout.page-set-up.print-area" => AREA_ITEMS,
                    "page-layout.page-set-up.breaks" => BREAK_ITEMS,
                    _ => &[],
                };
                n.and_then(|i| items.get(i)).map(|s| (*s).to_owned())
            }
        }
    } else {
        None
    };
    Some((control, item))
}

/// A checkbox value: the typed `=on`/`=off`, or the flipped current state.
fn checked(id: &str, now: bool) -> bool {
    match id.split_once('=').map(|(_, v)| v) {
        Some("on" | "true" | "1") => true,
        Some("off" | "false" | "0") => false,
        _ => !now,
    }
}

/// Margins typed as `top,bottom,left,right[,header,footer]` in inches (Custom Margins...).
fn custom_margins(text: &str) -> Option<Margins> {
    let n: Vec<f64> = text
        .split(',')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    if !n.iter().all(|v| (0.0..=50.0).contains(v)) {
        return None;
    }
    match *n.as_slice() {
        [top, bottom, left, right] => Some(Margins {
            top,
            bottom,
            left,
            right,
            ..Margins::NORMAL
        }),
        [top, bottom, left, right, header, footer] => Some(Margins {
            top,
            bottom,
            left,
            right,
            header,
            footer,
        }),
        _ => None,
    }
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let Some((control, item)) = split(id) else {
        return Handled::No;
    };
    let item = item.as_deref().unwrap_or("");
    let sheet = ctx.sheet.sheet;
    match control {
        "page-layout.themes.themes"
        | "page-layout.themes.colours"
        | "page-layout.themes.fonts"
        | "page-layout.themes.effects"
            if !item.is_empty() =>
        {
            let t = &mut ctx.sheet.ext_page.theme;
            let name = item.to_owned();
            match control {
                "page-layout.themes.themes" => t.theme = name,
                "page-layout.themes.colours" => t.colours = name,
                "page-layout.themes.fonts" => t.fonts = name,
                _ => t.effects = name,
            }
        }
        "page-layout.page-set-up.margins" => match item {
            "normal" => ctx.sheet.set_margins(Margins::NORMAL),
            "wide" => ctx.sheet.set_margins(Margins::WIDE),
            "narrow" => ctx.sheet.set_margins(Margins::NARROW),
            other => {
                if let Some(m) = other.strip_prefix("custom=").and_then(custom_margins) {
                    ctx.sheet.set_margins(m);
                }
            }
        },
        "page-layout.page-set-up.orientation" => match item {
            "portrait" => ctx.sheet.set_landscape(false),
            "landscape" => ctx.sheet.set_landscape(true),
            _ => {}
        },
        "page-layout.page-set-up.size" => {
            if let Some(p) = Paper::from_key(item) {
                ctx.sheet.set_paper(p);
            }
        }
        "page-layout.page-set-up.print-area" => match item {
            "set" => ctx.sheet.set_print_area(),
            "add" => ctx.sheet.add_print_area(),
            "clear" => ctx.sheet.clear_print_area(),
            _ => {}
        },
        "page-layout.page-set-up.breaks" => match item {
            "insert" => ctx.sheet.insert_page_break(),
            "remove" => ctx.sheet.remove_page_break(),
            "reset" => ctx.sheet.reset_page_breaks(),
            _ => {}
        },
        "page-layout.page-set-up.background" => {
            // A typed picture sets it; with one set, the button is Delete Background.
            if !item.is_empty() {
                ctx.sheet.set_background(Some(item.into()));
            } else if ctx.sheet.page().background.is_some() {
                ctx.sheet.set_background(None);
            } else if let Some(choose) = ctx.sheet.ext_page.chooser {
                ctx.sheet.set_background(choose());
            }
        }
        "page-layout.page-set-up.print-titles" => {
            let r = if item.is_empty() {
                ctx.sheet.set_print_titles_from_selection()
            } else {
                ctx.sheet.set_print_titles_text(item)
            };
            if let Err(e) = r {
                ctx.sheet.status = e;
            }
        }
        "page-layout.scale-to-fit.width" => {
            if let Some(n) = parse_pages(item) {
                ctx.sheet.set_fit_width(n);
            }
        }
        "page-layout.scale-to-fit.height" => {
            if let Some(n) = parse_pages(item) {
                ctx.sheet.set_fit_height(n);
            }
        }
        "page-layout.scale-to-fit.scale" => {
            if let Ok(n) = item.trim().trim_end_matches('%').parse::<u32>() {
                ctx.sheet.set_print_scale(n);
            }
        }
        // Gridlines and Headings share the View tab's per-sheet flags.
        "page-layout.sheet-options.view" => {
            let on = checked(id, ctx.sheet.ext_view.gridlines(sheet));
            ctx.sheet.ext_view.set_gridlines(sheet, on);
        }
        "page-layout.sheet-options.view-2" => {
            let on = checked(id, ctx.sheet.ext_view.headings(sheet));
            ctx.sheet.ext_view.set_headings(sheet, on);
        }
        "page-layout.sheet-options.print" => {
            let on = checked(id, ctx.sheet.page().print_gridlines);
            ctx.sheet.set_print_gridlines(on);
        }
        "page-layout.sheet-options.print-2" => {
            let on = checked(id, ctx.sheet.page().print_headings);
            ctx.sheet.set_print_headings(on);
        }
        _ => {}
    }
    Handled::Yes
}
