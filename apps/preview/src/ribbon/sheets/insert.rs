//! Sheets ribbon: the Insert tab (`insert.*`).
//!
//! Every edit is a `SheetObjects` command (see `sheet_ext::insert`), so each is one undo step.
//! A control that opens a dialog in Excel applies the dialog's defaults when it is sent without a
//! value; the dialog's answer is the value after `=`:
//!
//! - `insert.tables.table` makes a table of the selection (Excel's header guess);
//!   `=headers` / `=no-headers` is "My table has headers", `=name:Sales` names it
//! - `insert.tables.pivot-table` makes an empty PivotTable on a new sheet (`.from-table-range`);
//!   `=A1:C10` picks the source range
//! - `insert.charts.*` insert the chart of the menu item (`.clustered-column` ...); the plain
//!   control inserts its default chart, `recommended-charts` a clustered column
//! - `insert.links.link=address|text|screen tip` (`#Sheet2!A1` is a place, `mailto:` an e-mail);
//!   `.remove` removes the link of the active cell
//! - `insert.comments.comment=text` adds a threaded comment
//! - `insert.symbols.symbol=U+00A9` appends the symbol to the active cell
//! - `insert.text.text.header-footer` switches to Page Layout view; `insert.text.text.text-box` (or the
//!   bare control) inserts a text box at the active cell, selected, at the click size
//! - `insert.illustrations.pictures=path` inserts a PNG, JPEG, GIF, BMP or SVG file at the active cell
//!   at its natural size (`.this-device` is the menu item; with no path a file dialog asks);
//!   `insert.illustrations.shapes.<preset>` inserts a gallery shape (`.rect`, `.ellipse`,
//!   `.rightArrow` ...; bare is a rectangle); `insert.illustrations.icons=name` (or `.name`) inserts a
//!   bundled icon as an SVG picture
//! - the new object is selected; moving, sizing, rotating, filling, outlining, text, order and delete
//!   are `SheetModel::drawing_*` (see `sheet_ext::drawing`), each one undo step
//!
//! Not wired, with the missing subsystem: Suggested PivotTables (recommendation engine), Forms,
//! Stock and Online Pictures (no image services), 3D Models, Diagram, Screenshot (no capture or
//! SmartArt), Checkbox (cell controls), Hierarchy, Statistic, Combo, Maps and PivotChart (chart
//! engine has no such types), Sparklines (no sparkline engine), Slicer and Timeline, Equation,
//! WordArt, Signature Line and Object (no such objects yet), waterfall, funnel, surface and bubble.

use std::path::{Path, PathBuf};

use open365_drawing::catalog;
use open365_ui_core::MenuItem;

use crate::dialogs::{insert_chart, insert_link, insert_symbol};
use crate::ribbon::{Handled, RibbonCtx, base};
use crate::sheet::SheetModel;
use crate::sheet::sheet_ext::drawing::icons;
use crate::sheet::sheet_ext::view::ViewMode;

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "insert.tables.table",
    "insert.tables.pivot-table",
    "insert.charts.recommended-charts",
    "insert.charts.insert-column-or-bar-chart",
    "insert.charts.insert-line-or-area-chart",
    "insert.charts.insert-pie-or-doughnut-chart",
    "insert.charts.insert-scatter-x-y-or-bubble-chart",
    "insert.charts.insert-waterfall-funnel-stock-surface-or-radar-chart",
    "insert.links.link",
    "insert.comments.comment",
    "insert.symbols.symbol",
    "insert.text.text",
    "insert.illustrations.pictures",
    "insert.illustrations.shapes",
    "insert.illustrations.icons",
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

/// The drop-down content of an Insert control, `None` when it has none here.
#[must_use]
pub fn menu_items(id: &str) -> Option<Vec<MenuItem>> {
    let item = |label: &str, sub: &str, access: char| MenuItem {
        access: Some(access),
        ..MenuItem::new(&format!("{id}.{sub}"), label)
    };
    Some(match id {
        "insert.charts.insert-column-or-bar-chart" => vec![
            MenuItem::heading("2-D Column"),
            item("Clustered Column", "clustered-column", 'C'),
            item("Stacked Column", "stacked-column", 'S'),
            item("100% Stacked Column", "100-stacked-column", '1'),
            MenuItem::heading("2-D Bar"),
            item("Clustered Bar", "clustered-bar", 'B'),
            item("Stacked Bar", "stacked-bar", 'T'),
            item("100% Stacked Bar", "100-stacked-bar", '0'),
        ],
        "insert.charts.insert-line-or-area-chart" => vec![
            MenuItem::heading("2-D Line"),
            item("Line", "line", 'L'),
            item("Line with Markers", "line-with-markers", 'M'),
            MenuItem::heading("2-D Area"),
            item("Area", "area", 'A'),
            item("Stacked Area", "stacked-area", 'S'),
            item("100% Stacked Area", "100-stacked-area", '1'),
        ],
        "insert.charts.insert-pie-or-doughnut-chart" => vec![
            MenuItem::heading("2-D Pie"),
            item("Pie", "pie", 'P'),
            MenuItem::heading("Doughnut"),
            item("Doughnut", "doughnut", 'D'),
        ],
        "insert.charts.insert-scatter-x-y-or-bubble-chart" => vec![item("Scatter", "scatter", 'S')],
        "insert.charts.insert-waterfall-funnel-stock-surface-or-radar-chart" => {
            vec![item("Stock", "stock", 'S'), item("Radar", "radar", 'R')]
        }
        "insert.tables.pivot-table" => vec![item("From Table/Range...", "from-table-range", 'T')],
        "insert.links.link" => vec![
            item("Insert Link...", "run", 'I'),
            item("Remove Link", "remove", 'R'),
        ],
        "insert.text.text" => vec![
            item("Text Box", "text-box", 'T'),
            item("Header & Footer", "header-footer", 'H'),
        ],
        "insert.illustrations.pictures" => vec![item("This Device...", "this-device", 'D')],
        "insert.illustrations.shapes" => {
            let mut out = Vec::new();
            for c in catalog::CATEGORIES {
                out.push(MenuItem::heading(c.label));
                for (preset, label) in c.shapes {
                    out.push(MenuItem::new(&format!("{id}.{preset}"), label));
                }
            }
            out
        }
        "insert.illustrations.icons" => icons::ICONS
            .iter()
            .map(|(name, _)| MenuItem::new(&format!("{id}.{name}"), &icons::icon_label(name)))
            .collect(),
        _ => return None,
    })
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let Some((ctrl, sub, arg)) = split(id) else {
        return Handled::No;
    };
    let m = &mut *ctx.sheet;
    match run(m, ctrl, sub, arg) {
        Ok(()) => *ctx.refocus_doc = true,
        Err(e) => m.status = e,
    }
    Handled::Yes
}

fn run(m: &mut SheetModel, ctrl: &str, sub: &str, arg: Option<&str>) -> Result<(), String> {
    let tail = ctrl.rsplit('.').next().unwrap_or(ctrl);
    let sub = if sub == "run" { "" } else { sub };
    match tail {
        "table" => {
            let (mut headers, mut name) = (None, None);
            for part in arg.unwrap_or_default().split(';').filter(|p| !p.is_empty()) {
                match part.split_once(':') {
                    Some(("name", n)) => name = Some(n.to_owned()),
                    _ if part == "headers" => headers = Some(true),
                    _ if part == "no-headers" => headers = Some(false),
                    _ => {}
                }
            }
            m.insert_table(headers, name)
        }
        "pivot-table" => {
            if !sub.is_empty() && sub != "from-table-range" {
                return Err("This PivotTable source is not available yet.".to_owned());
            }
            let source = match arg.filter(|a| !a.is_empty()) {
                Some(a) => Some(parse_range(a)?),
                None => None,
            };
            m.insert_pivot_table(source)
        }
        "link" => match sub {
            "remove" => m.insert_remove_link(),
            _ => {
                let mut parts = arg.unwrap_or_default().split('|');
                let target = insert_link::parse_target(parts.next().unwrap_or_default())?;
                let text = parts.next().filter(|t| !t.is_empty()).map(str::to_owned);
                let tip = parts.next().filter(|t| !t.is_empty()).map(str::to_owned);
                m.insert_link(target, text, tip)
            }
        },
        "comment" => match arg {
            Some(text) if !text.is_empty() => m.review_add_comment(text),
            _ => Err("Type the comment text.".to_owned()),
        },
        "symbol" => match arg.and_then(insert_symbol::parse_symbol) {
            Some(s) => m.insert_symbol(&s),
            None => Err("Choose a symbol to insert.".to_owned()),
        },
        "text" => match sub {
            "header-footer" => {
                m.set_view_mode(ViewMode::PageLayout);
                Ok(())
            }
            "" | "text-box" => m.insert_text_box().map(drop),
            _ => Err("This Text command is not available yet.".to_owned()),
        },
        "pictures" => match arg.filter(|a| !a.is_empty()) {
            Some(path) => m.insert_picture_file(Path::new(path)).map(drop),
            None => match choose_picture_path() {
                Some(path) => m.insert_picture_file(&path).map(drop),
                None => Ok(()),
            },
        },
        "shapes" => m
            .insert_shape(if sub.is_empty() { "rect" } else { sub })
            .map(drop),
        "icons" => {
            let name = if sub.is_empty() {
                arg.unwrap_or_default()
            } else {
                sub
            };
            if name.is_empty() {
                Err("Choose an icon: insert.illustrations.icons=NAME, for example home or chart-pie."
                    .to_owned())
            } else {
                m.insert_icon(name).map(drop)
            }
        }
        _ => match insert_chart::kind_of(tail, sub) {
            Some(kind) => m.insert_chart(kind),
            None => Err("This chart type is not available yet.".to_owned()),
        },
    }
}

fn drop<T>(_: T) {}

/// Insert > Pictures > This Device: asks for the file.
fn choose_picture_path() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Insert Picture")
        .add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "bmp", "svg"])
        .pick_file()
}

fn parse_range(text: &str) -> Result<(u32, u32, u32, u32), String> {
    let bad = || format!("{text} is not a valid range.");
    let (a, b) = text.split_once(':').unwrap_or((text, text));
    let a = crate::sheet::parse_a1(&a.replace('$', "")).ok_or_else(bad)?;
    let b = crate::sheet::parse_a1(&b.replace('$', "")).ok_or_else(bad)?;
    Ok((a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)))
}
