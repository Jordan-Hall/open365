//! Sheets ribbon: the Home tab (`home.*`).
//!
//! Every control is a command of the command layer applied to the selection (see
//! `sheet_ext::home`), so each one is a single undo step. Drop-down items arrive as
//! `<control>.<item>` (`home.editing.sort-filter.az`) and carry a value after `=` where Excel's
//! dialog would ask for one (`home.cells.format.row-height=30`). Dialogs the shell cannot draw yet
//! (Paste Special, Custom Sort, Find, Format Cells) are reachable through these value forms only.
//!
//! Not wired: Conditional Formatting and Format as Table (the workbook model cannot create rules
//! or tables yet) and Add-ins.

use open365_commands::{HAlign, MergeMode, RotationSpec, SortLevel, VAlign};

use crate::menus;
use crate::ribbon::{Handled, RibbonCtx, base};
use crate::sheet::sheet_ext::home::{FillDir, NUMBER_FORMAT_NAMES};
use crate::sheet::{PasteKind, SheetModel};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "home.clipboard.paste",
    "home.clipboard.cut",
    "home.clipboard.copy",
    "home.clipboard.format-painter",
    "home.font.font",
    "home.font.font-size",
    "home.font.increase-font-size",
    "home.font.decrease-font-size",
    "home.font.bold",
    "home.font.italic",
    "home.font.underline",
    "home.font.borders",
    "home.font.fill-colour",
    "home.font.font-colour",
    "home.alignment.top-align",
    "home.alignment.middle-align",
    "home.alignment.bottom-align",
    "home.alignment.align-left",
    "home.alignment.centre",
    "home.alignment.align-right",
    "home.alignment.orientation",
    "home.alignment.decrease-indent",
    "home.alignment.increase-indent",
    "home.alignment.wrap-text",
    "home.alignment.merge-centre",
    "home.number.number-format",
    "home.number.accounting-number-format",
    "home.number.percent-style",
    "home.number.comma-style",
    "home.number.increase-decimal",
    "home.number.decrease-decimal",
    "home.styles.cell-styles",
    "home.cells.insert",
    "home.cells.delete",
    "home.cells.format",
    "home.editing.auto-sum",
    "home.editing.fill",
    "home.editing.clear",
    "home.editing.sort-filter",
    "home.editing.find-select",
];

/// The part of `id` after the control id, and the typed value (`=text`) or list index (`#n`).
fn split(id: &str) -> Option<(&'static str, String, Option<String>)> {
    let b = base(id);
    let ctrl = IDS
        .iter()
        .copied()
        .find(|c| b == *c || b.strip_prefix(c).is_some_and(|r| r.starts_with('.')))?;
    let sub = b[ctrl.len()..].trim_start_matches('.').to_owned();
    let rest = &id[b.len()..];
    let arg = match rest.chars().next() {
        Some('=') => Some(rest[1..].to_owned()),
        Some('#') => rest[1..]
            .parse::<usize>()
            .ok()
            .and_then(|n| index_item(ctrl, n)),
        _ => None,
    };
    Some((ctrl, sub, arg))
}

/// The list entry behind `#n` for the combo `ctrl`.
fn index_item(ctrl: &str, n: usize) -> Option<String> {
    match ctrl {
        "home.number.number-format" => NUMBER_FORMAT_NAMES.get(n).map(|s| (*s).to_owned()),
        _ => menus::combo_items(ctrl).get(n).cloned(),
    }
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

fn hex(s: &str) -> Option<String> {
    let h = s.trim().trim_start_matches('#');
    (h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())).then(|| h.to_ascii_uppercase())
}

/// `B:desc,C` as sort levels.
fn levels(text: &str) -> Vec<SortLevel> {
    text.split(',')
        .filter_map(|part| {
            let (col, order) = part.split_once(':').unwrap_or((part, "asc"));
            let column = crate::sheet::parse_a1(&format!("{}1", col.trim()))?.1;
            Some(SortLevel {
                column,
                descending: order.trim().eq_ignore_ascii_case("desc"),
            })
        })
        .collect()
}

fn note(m: &mut SheetModel, text: &str) {
    m.status = text.to_owned();
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    // Format Cells > Number > Custom is not a control of the ribbon, only a value form.
    if let Some(code) = id.strip_prefix("home.number.format-code=") {
        let m = &mut *ctx.sheet;
        let result = if code.trim().is_empty() {
            m.home_number_format("general")
        } else {
            m.home_number_code(code.trim())
        };
        if let Err(e) = result {
            m.status = e;
        }
        return Handled::Yes;
    }
    let Some((ctrl, sub, arg)) = split(id) else {
        return Handled::No;
    };
    let sub = if sub == "run" { String::new() } else { sub };
    let scale = ctx.ui.scale.0;
    let ui = &*ctx.ui;
    let measure = move |t: &str| ui.measure(t) / scale;
    let m = &mut *ctx.sheet;
    let result = run(m, ctrl, &sub, arg.as_deref(), &measure);
    if let Err(e) = result {
        m.status = e;
    }
    Handled::Yes
}

fn run(
    m: &mut SheetModel,
    ctrl: &str,
    sub: &str,
    arg: Option<&str>,
    measure: &dyn Fn(&str) -> f32,
) -> Result<(), String> {
    let tail = ctrl.rsplit('.').next().unwrap_or(ctrl);
    match tail {
        "paste" => match sub {
            "" | "paste" => m.home_paste(None),
            "values" => m.home_paste(Some(PasteKind::Values)),
            "formatting" => m.home_paste(Some(PasteKind::Formats)),
            "formulas" => m.home_paste(Some(PasteKind::Formulas)),
            "transpose" => m.home_paste(Some(PasteKind::Transpose)),
            _ => {
                note(m, "This Paste option is not available yet.");
                Ok(())
            }
        },
        "cut" => m.copy(true),
        "copy" => m.copy(false),
        "format-painter" => {
            match m.ext_home.painter {
                Some((_, src)) if src != m.range() => {
                    m.painter_apply()?;
                }
                Some(_) => m.ext_home.painter = None,
                None => m.painter_pick_up(),
            }
            Ok(())
        }
        "font" => match arg {
            Some(name) => m.home_font_name(name),
            None => Ok(()),
        },
        "font-size" => match arg.and_then(|a| a.trim().parse::<f64>().ok()) {
            Some(size) => m.home_font_size(size),
            None if arg.is_some() => Err("The number must be between 1 and 409.".to_owned()),
            None => Ok(()),
        },
        "increase-font-size" => m.home_font_step(true),
        "decrease-font-size" => m.home_font_step(false),
        "bold" => m.home_bold(),
        "italic" => m.home_italic(),
        "underline" => match sub {
            "" | "single" => m.home_underline(false),
            "double" => m.home_underline(true),
            _ => Ok(()),
        },
        "borders" => {
            let kind = if sub.is_empty() {
                m.ext_home.border.clone()
            } else {
                sub.to_owned()
            };
            match kind.as_str() {
                "bottom" | "top" | "left" | "right" | "none" | "all" | "outside"
                | "thick-outside" | "bottom-double" | "bottom-thick" | "top-bottom"
                | "top-thick-bottom" | "top-double-bottom" => m.home_border(&kind),
                _ => {
                    note(m, "This Borders option is not available yet.");
                    Ok(())
                }
            }
        }
        "fill-colour" => match colour(sub, arg) {
            Pick::Keep => {
                let c = m.ext_home.fill.clone();
                m.home_fill(c.as_deref())
            }
            Pick::Auto => m.home_fill(None),
            Pick::Rgb(c) => m.home_fill(Some(&c)),
            Pick::Other => {
                note(m, "Pick a colour with .rgb-RRGGBB.");
                Ok(())
            }
        },
        "font-colour" => match colour(sub, arg) {
            Pick::Keep => {
                let c = m.ext_home.font_colour.clone();
                m.home_font_colour(c.as_deref())
            }
            Pick::Auto => m.home_font_colour(None),
            Pick::Rgb(c) => m.home_font_colour(Some(&c)),
            Pick::Other => {
                note(m, "Pick a colour with .rgb-RRGGBB.");
                Ok(())
            }
        },
        "top-align" => m.home_valign(VAlign::Top),
        "middle-align" => m.home_valign(VAlign::Center),
        "bottom-align" => m.home_valign(VAlign::Bottom),
        "align-left" => m.home_halign(HAlign::Left),
        "centre" => m.home_halign(HAlign::Center),
        "align-right" => m.home_halign(HAlign::Right),
        "orientation" => match sub {
            "angle-counterclockwise" => m.home_rotation(RotationSpec::Degrees(45)),
            "angle-clockwise" => m.home_rotation(RotationSpec::Degrees(-45)),
            "vertical-text" => m.home_rotation(RotationSpec::Stacked),
            "rotate-up" => m.home_rotation(RotationSpec::Degrees(90)),
            "rotate-down" => m.home_rotation(RotationSpec::Degrees(-90)),
            "horizontal" => m.home_rotation(RotationSpec::Degrees(0)),
            _ => Ok(()),
        },
        "decrease-indent" => m.home_indent(-1),
        "increase-indent" => m.home_indent(1),
        "wrap-text" => m.home_wrap(),
        "merge-centre" => match sub {
            "" | "merge-centre" => m.home_merge(MergeMode::MergeCenter, true),
            "across" => m.home_merge(MergeMode::Across, false),
            "cells" => m.home_merge(MergeMode::Cells, false),
            "unmerge" => m.home_merge(MergeMode::Unmerge, false),
            _ => Ok(()),
        },
        "number-format" => match arg {
            Some(name) => m.home_number_format(name),
            None => Ok(()),
        },
        "accounting-number-format" => match sub {
            "" => m.home_accounting(None),
            "usd" => m.home_accounting(Some("$")),
            "gbp" => m.home_accounting(Some("£")),
            "eur" => m.home_accounting(Some("€")),
            "jpy" => m.home_accounting(Some("¥")),
            "chf" => m.home_accounting(Some("CHF")),
            _ => {
                note(m, "This currency is not available yet.");
                Ok(())
            }
        },
        "percent-style" => m.home_percent(),
        "comma-style" => m.home_comma(),
        "increase-decimal" => m.home_decimals(1),
        "decrease-decimal" => m.home_decimals(-1),
        "cell-styles" => {
            let wanted = arg
                .map(str::to_owned)
                .or_else(|| (!sub.is_empty()).then(|| sub.to_owned()));
            let Some(wanted) = wanted else { return Ok(()) };
            let names = m.home_style_names();
            match names
                .iter()
                .find(|n| n.eq_ignore_ascii_case(&wanted) || slug(n) == slug(&wanted))
            {
                Some(n) => m.home_style(&n.clone()),
                None => Err(format!("There is no cell style named {wanted}.")),
            }
        }
        "insert" => match sub {
            "" | "cells" => m.home_insert().map(|_| ()),
            "rows" => m.insert_selected(true),
            "columns" => m.insert_selected(false),
            _ => {
                note(m, "This Insert option is not available yet.");
                Ok(())
            }
        },
        "delete" => match sub {
            "" | "cells" => m.home_delete().map(|_| ()),
            "rows" => m.delete_selected(true),
            "columns" => m.delete_selected(false),
            _ => {
                note(m, "This Delete option is not available yet.");
                Ok(())
            }
        },
        "format" => format_menu(m, sub, arg, measure),
        "auto-sum" => match sub {
            "" | "sum" => m.home_auto_sum("SUM"),
            "average" => m.home_auto_sum("AVERAGE"),
            "count-numbers" => m.home_auto_sum("COUNT"),
            "max" => m.home_auto_sum("MAX"),
            "min" => m.home_auto_sum("MIN"),
            _ => Ok(()),
        },
        "fill" => match sub {
            "down" => m.home_fill_dir(FillDir::Down).map(|_| ()),
            "right" => m.home_fill_dir(FillDir::Right).map(|_| ()),
            "up" => m.home_fill_dir(FillDir::Up).map(|_| ()),
            "left" => m.home_fill_dir(FillDir::Left).map(|_| ()),
            "" => Ok(()),
            _ => {
                note(m, "This Fill option is not available yet.");
                Ok(())
            }
        },
        "clear" => match sub {
            "all" => m.home_clear_all(),
            "formats" => m.home_clear_formats(),
            "contents" => m.home_clear_contents(),
            "" => Ok(()),
            _ => {
                note(m, "This Clear option is not available yet.");
                Ok(())
            }
        },
        "sort-filter" => match sub {
            "az" => m.home_sort(false),
            "za" => m.home_sort(true),
            "custom" => match arg.map(levels).filter(|l| !l.is_empty()) {
                Some(l) => {
                    let range = if m.is_range() {
                        m.range()
                    } else {
                        m.home_current_region(m.active)
                    };
                    m.home_sort_by(range, &l)
                }
                None => {
                    note(m, "Custom Sort needs levels, for example custom=B:desc,C.");
                    Ok(())
                }
            },
            "filter" => m.home_toggle_filter(),
            "clear" => m.home_refilter(true),
            "reapply" => m.home_refilter(false),
            _ => Ok(()),
        },
        "find-select" => match (sub, arg) {
            ("find", Some(text)) => {
                if !m.home_find(text) {
                    note(m, "We couldn't find what you were looking for.");
                }
                Ok(())
            }
            ("goto", Some(r)) => {
                if !m.home_goto(r) {
                    note(m, "Reference isn't valid.");
                }
                Ok(())
            }
            _ => {
                note(
                    m,
                    "This Find & Select option needs a dialog and is not available yet.",
                );
                Ok(())
            }
        },
        _ => Ok(()),
    }
}

/// The Format drop-down of the Cells group.
fn format_menu(
    m: &mut SheetModel,
    sub: &str,
    arg: Option<&str>,
    measure: &dyn Fn(&str) -> f32,
) -> Result<(), String> {
    let number = || arg.and_then(|a| a.trim().parse::<f64>().ok());
    match sub {
        "row-height" => match number() {
            Some(p) => m.home_row_height(p),
            None => Err("Enter a number between 0 and 409.5.".to_owned()),
        },
        "column-width" => match number() {
            Some(w) => m.home_col_width(w),
            None => Err("Enter a number between 0 and 255.".to_owned()),
        },
        "autofit-row" | "autofit-row-height" => m.home_autofit_rows(),
        "autofit-column" | "autofit-column-width" => m.home_autofit_cols(measure),
        "hide-rows" => m.home_hide(true, true),
        "hide-columns" => m.home_hide(false, true),
        "unhide-rows" => m.home_hide(true, false),
        "unhide-columns" => m.home_hide(false, false),
        "protection" => {
            let flag = |name: &str| {
                arg.and_then(|a| {
                    a.split(';')
                        .find_map(|p| p.trim().strip_prefix(name)?.strip_prefix('='))
                })
                .map(|v| v.trim() == "1")
            };
            match (flag("locked"), flag("hidden")) {
                (Some(locked), Some(hidden)) => m.home_protection(locked, hidden),
                _ => Err("Protection needs locked=0|1;hidden=0|1.".to_owned()),
            }
        }
        "" => Ok(()),
        _ => {
            note(m, "This Format option is not available yet.");
            Ok(())
        }
    }
}

enum Pick {
    Keep,
    Auto,
    Rgb(String),
    Other,
}

/// Which colour a Fill/Font Colour item stands for.
fn colour(sub: &str, arg: Option<&str>) -> Pick {
    if let Some(c) = arg.and_then(hex) {
        return Pick::Rgb(c);
    }
    match sub {
        "" => Pick::Keep,
        "auto" | "none" | "no-fill" | "automatic" => Pick::Auto,
        s => s
            .strip_prefix("rgb-")
            .and_then(hex)
            .map_or(Pick::Other, Pick::Rgb),
    }
}
