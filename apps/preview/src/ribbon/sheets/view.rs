//! Sheets ribbon: the View tab (`view.*`).
//!
//! Wired: workbook views, Show options, Zoom, Zoom to Selection, Freeze Panes, Split, Hide and
//! Unhide (window state only: the shell has no hide request), and Custom Views
//! (`view.workbook-views.custom-views=add:Name`, `=show:Name`, `=delete:Name`; the dialog that
//! lists them is not built, so a plain click does nothing).
//!
//! Left to `Handled::No`:
//! - New Window, Arrange All, Switch Windows, Reset Window Position, View Side by Side and
//!   Synchronous Scrolling: the shell hosts one document per window and has no second view of a
//!   live workbook; `WinAct::OpenDocument` would start another instance on the saved file, which
//!   shares no edits.
//! - Macros: there is no VBA runtime.
//! - Sheet View (named filter views) and Switch Theme.

use crate::ribbon::{Handled, RibbonCtx, base};
use crate::sheet::sheet_ext::view::{Freeze, ViewMode};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "view.workbook-views.normal",
    "view.workbook-views.page-break-preview",
    "view.workbook-views.page-layout",
    "view.workbook-views.custom-views",
    "view.show.navigation",
    "view.show.ruler",
    "view.show.gridlines",
    "view.show.formula-bar",
    "view.show.headings",
    "view.show.data-type-icons",
    "view.show.focus-cell",
    "view.zoom.zoom",
    "view.zoom.100",
    "view.zoom.zoom-to-selection",
    "view.window.freeze-panes",
    "view.window.split",
    "view.window.hide",
    "view.window.unhide",
];

/// The window the grid is shown in, for Zoom to Selection (logical pixels).
const VIEW_W: f32 = 1200.0;
const VIEW_H: f32 = 600.0;

/// `(control id, tail)` for `id`, where the tail is the menu item (`freeze-top-row`) or empty.
fn split(id: &str) -> Option<(&'static str, &str)> {
    let b = base(id);
    IDS.iter().find_map(|c| {
        if b == *c {
            Some((*c, ""))
        } else {
            b.strip_prefix(c)
                .and_then(|r| r.strip_prefix('.'))
                .map(|t| (*c, t))
        }
    })
}

/// A checkbox value: the typed `=on`/`=off`, or the flipped current state.
fn checked(id: &str, now: bool) -> bool {
    match id.split_once('=').map(|(_, v)| v) {
        Some("on" | "true" | "1") => true,
        Some("off" | "false" | "0") => false,
        _ => !now,
    }
}

/// `150` and `150%` as a zoom value.
fn percent(id: &str) -> Option<u32> {
    id.split_once('=')?
        .1
        .trim()
        .trim_end_matches('%')
        .parse()
        .ok()
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let Some((control, tail)) = split(id) else {
        return Handled::No;
    };
    let sheet = ctx.sheet.sheet;
    let v = &mut ctx.sheet.ext_view;
    match control {
        "view.workbook-views.normal" => ctx.sheet.set_view_mode(ViewMode::Normal),
        "view.workbook-views.page-break-preview" => {
            ctx.sheet.set_view_mode(ViewMode::PageBreakPreview)
        }
        "view.workbook-views.page-layout" => ctx.sheet.set_view_mode(ViewMode::PageLayout),
        "view.show.navigation" => v.navigation = !v.navigation,
        "view.show.ruler" => v.ruler = checked(id, v.ruler),
        "view.show.gridlines" => {
            let on = checked(id, v.gridlines(sheet));
            v.set_gridlines(sheet, on);
        }
        "view.show.formula-bar" => v.formula_bar = checked(id, v.formula_bar),
        "view.show.headings" => {
            let on = checked(id, v.headings(sheet));
            v.set_headings(sheet, on);
        }
        "view.show.data-type-icons" => v.data_type_icons = checked(id, v.data_type_icons),
        "view.show.focus-cell" => v.focus_cell = checked(id, v.focus_cell),
        "view.zoom.zoom" => {
            // The Zoom dialog: a typed percentage sets it, the plain click has no dialog yet.
            if let Some(p) = percent(id) {
                ctx.sheet.set_zoom_percent(p);
            }
        }
        "view.zoom.100" => ctx.sheet.set_zoom_percent(100),
        "view.zoom.zoom-to-selection" => ctx.sheet.zoom_to_selection(VIEW_W, VIEW_H),
        "view.window.freeze-panes" => {
            let kind = match tail {
                "freeze-panes" | "freeze" | "unfreeze" => Some(if tail == "unfreeze" {
                    Freeze::Unfreeze
                } else {
                    Freeze::Panes
                }),
                "freeze-top-row" => Some(Freeze::TopRow),
                "freeze-first-column" => Some(Freeze::FirstColumn),
                _ => match id.rsplit_once('#').map(|(_, n)| n) {
                    Some("0") => Some(Freeze::Panes),
                    Some("1") => Some(Freeze::TopRow),
                    Some("2") => Some(Freeze::FirstColumn),
                    _ => None,
                },
            };
            if let Some(k) = kind {
                ctx.sheet.freeze(k);
            }
        }
        "view.window.split" => ctx.sheet.toggle_split(),
        "view.window.hide" => v.hidden = true,
        "view.window.unhide" => v.hidden = false,
        "view.workbook-views.custom-views" => {
            if let Some((verb, name)) = id
                .split_once('=')
                .map(|(_, a)| a)
                .and_then(|a| a.split_once(':'))
            {
                let r = match verb {
                    "add" => ctx.sheet.view_add_custom_view(name),
                    "show" => ctx.sheet.view_show_custom_view(name),
                    "delete" => ctx.sheet.view_delete_custom_view(name),
                    other => Err(format!("Unknown Custom Views action '{other}'.")),
                };
                if let Err(e) = r {
                    ctx.sheet.status = e;
                }
            }
        }
        _ => {}
    }
    Handled::Yes
}
