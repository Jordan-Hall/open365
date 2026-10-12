//! Format Cells (Number, Alignment, Font, Border, Fill and Protection tabs), Row Height, Column
//! Width, the Merge confirmation and the data validation alerts.

use open365_ui_core::Rect;
use open365_ui_core::Ui;
use open365_ui_core::dialog::{ButtonRole, Dialog, FooterButton, Outcome, Spec, Values};
use open365_xlsx::{ColorSource, FillPattern, HorizontalAlignment, Underline, VerticalAlignment};

use super::{View, request_close, wrap};
use crate::dialogs::format_cells::{
    BorderWish, CATEGORIES, COLOURS, Current, HAlign, Under, VAlign, Wanted, category_of,
    default_decimals, plan,
};
use crate::dialogs::request::{DialogRequest, FormatTab};
use crate::shell::Shell;

pub(super) fn make(s: &mut Shell, req: &DialogRequest) -> Option<(Spec, Box<dyn View>)> {
    Some(match req {
        DialogRequest::FormatCells { tab } => format_cells(s, *tab),
        DialogRequest::RowHeight => row_height(s),
        DialogRequest::ColumnWidth => column_width(s),
        DialogRequest::MergeConfirm { command } => merge_confirm(command),
        DialogRequest::ValidationAlert {
            row,
            col,
            text,
            style,
            title,
            message,
        } => validation_alert(*row, *col, text, style, title, message),
        _ => return None,
    })
}

impl Shell {
    /// Opens the alert Excel shows for an entry the cell's validation rule does not accept
    /// (`style` `warning` or `information`; a `stop` alert only offers Retry and Cancel).
    /// OK or Yes writes `text` into the cell anyway.
    pub fn open_validation_alert(
        &mut self,
        ui: &mut Ui,
        (row, col): (u32, u32),
        text: &str,
        rejected: &crate::sheet_ext::data::Rejected,
    ) -> bool {
        self.open_dialog(
            ui,
            DialogRequest::ValidationAlert {
                row,
                col,
                text: text.to_owned(),
                style: rejected.style.to_owned(),
                title: rejected.title.clone(),
                message: rejected.message.clone(),
            },
        )
    }
}

/// Merge & Center and friends discard values below and right of the upper-left cell; Excel asks
/// first. `Some` when `id` is such a command and it would discard something.
pub(super) fn merge_request(s: &Shell, id: &str) -> Option<DialogRequest> {
    let sub = id.strip_prefix("home.alignment.merge-centre")?;
    if id.contains(['=', '#']) {
        return None;
    }
    let sub = sub.trim_start_matches('.');
    let across = sub == "across";
    if !matches!(sub, "" | "merge-centre" | "across" | "cells") {
        return None;
    }
    let (r1, c1, r2, c2) = s.sheet.range();
    if r1 == r2 && c1 == c2 {
        return None;
    }
    let (ar, ac) = s.sheet.active;
    let merged = s
        .sheet
        .home_merged()
        .iter()
        .any(|m| (m.0..=m.2).contains(&ar) && (m.1..=m.3).contains(&ac));
    if merged && matches!(sub, "" | "merge-centre") {
        return None;
    }
    let mut seen = 0usize;
    let mut discards = false;
    'rows: for r in r1..=r2 {
        for c in c1..=c2 {
            seen += 1;
            if seen > 200_000 {
                break 'rows;
            }
            let keep = if across { c == c1 } else { r == r1 && c == c1 };
            if !keep && !s.sheet.source(r, c).is_empty() {
                discards = true;
                break 'rows;
            }
        }
    }
    discards.then(|| DialogRequest::MergeConfirm {
        command: id.to_owned(),
    })
}

// ---- Format Cells --------------------------------------------------------------------------------

const FONTS: [&str; 12] = [
    "Aptos Narrow",
    "Arial",
    "Calibri",
    "Cambria",
    "Consolas",
    "Courier New",
    "Georgia",
    "Segoe UI",
    "Tahoma",
    "Times New Roman",
    "Trebuchet MS",
    "Verdana",
];
const SIZES: [&str; 16] = [
    "8", "9", "10", "11", "12", "14", "16", "18", "20", "22", "24", "26", "28", "36", "48", "72",
];
const SYMBOLS: [(&str, &str); 5] = [
    ("$ (USD)", "usd"),
    ("\u{a3} (GBP)", "gbp"),
    ("\u{20ac} (EUR)", "eur"),
    ("\u{a5} (JPY)", "jpy"),
    ("CHF", "chf"),
];

#[derive(Debug)]
struct FormatCells {
    cur: Current,
}

fn current_state(s: &Shell) -> Current {
    let (r, c) = s.sheet.active;
    let code = s.sheet.home_format_code(r, c);
    let (number_category, decimals) = category_of(&code);
    let mut cur = Current {
        number_category,
        decimals,
        halign: HAlign::General,
        valign: VAlign::Bottom,
        wrap: false,
        indent: 0,
        rotation: Some(0),
        merged: false,
        font_name: "Calibri".to_owned(),
        font_size: 11.0,
        bold: false,
        italic: false,
        underline: Under::None,
        font_rgb: None,
        fill_rgb: None,
        locked: true,
        hidden: false,
    };
    if let Some(f) = s.sheet.home_cell_format(r, c) {
        cur.halign = match f.alignment.horizontal {
            HorizontalAlignment::Left => HAlign::Left,
            HorizontalAlignment::Center => HAlign::Center,
            HorizontalAlignment::Right => HAlign::Right,
            _ => HAlign::General,
        };
        cur.valign = match f.alignment.vertical {
            VerticalAlignment::Top => VAlign::Top,
            VerticalAlignment::Center => VAlign::Center,
            _ => VAlign::Bottom,
        };
        cur.wrap = f.alignment.wrap_text;
        cur.indent = f.alignment.indent;
        cur.rotation = match f.alignment.text_rotation {
            255 => None,
            r @ 0..=90 => Some(r as i32),
            r @ 91..=180 => Some(-(r as i32 - 90)),
            _ => Some(0),
        };
        cur.font_name = f.font.name.clone();
        cur.font_size = f.font.size;
        cur.bold = f.font.bold;
        cur.italic = f.font.italic;
        cur.underline = match f.font.underline {
            Underline::None => Under::None,
            Underline::Single | Underline::SingleAccounting => Under::Single,
            _ => Under::Double,
        };
        if let ColorSource::Rgb(c) = f.font.color.source {
            cur.font_rgb = Some(format!("{:02X}{:02X}{:02X}", c.r, c.g, c.b));
        }
        if f.fill.pattern == FillPattern::Solid
            && let ColorSource::Rgb(c) = f.fill.fg.source
        {
            cur.fill_rgb = Some(format!("{:02X}{:02X}{:02X}", c.r, c.g, c.b));
        }
    }
    cur.merged = s
        .sheet
        .home_merged()
        .iter()
        .any(|m| (m.0..=m.2).contains(&r) && (m.1..=m.3).contains(&c));
    cur
}

/// The colour box items: the named colours, plus the cell's own colour when it is none of them.
fn colour_items(current: Option<&str>, none_label: &str) -> (Vec<String>, usize) {
    let mut items: Vec<String> = COLOURS
        .iter()
        .map(|(n, c)| {
            if c.is_none() {
                none_label.to_owned()
            } else {
                (*n).to_owned()
            }
        })
        .collect();
    let mut sel = 0;
    match current {
        None => {}
        Some(rgb) => match COLOURS
            .iter()
            .position(|(_, c)| c.is_some_and(|c| c.eq_ignore_ascii_case(rgb)))
        {
            Some(i) => sel = i,
            None => {
                items.push(format!("#{rgb}"));
                sel = items.len() - 1;
            }
        },
    }
    (items, sel)
}

fn items_ref(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn format_cells(s: &mut Shell, tab: FormatTab) -> (Spec, Box<dyn View>) {
    let cur = current_state(s);
    let mut spec = Spec::new("Format Cells", 440.0, 300.0);
    spec.tabs = vec![
        "&Number".into(),
        "&Alignment".into(),
        "&Font".into(),
        "&Border".into(),
        "F&ill".into(),
        "&Protection".into(),
    ];

    // Number.
    spec.on_page(Some(0));
    spec.label(12.0, 40.0, 150.0, "&Category:");
    spec.list(
        "cat",
        Rect::new(12.0, 60.0, 150.0, 214.0),
        CATEGORIES.iter().map(|c| (*c).to_owned()).collect(),
        Some(cur.number_category),
    );
    spec.label(176.0, 64.0, 110.0, "&Decimal places:");
    spec.spin(
        "dec",
        290.0,
        60.0,
        80.0,
        (f64::from(cur.decimals), 0.0, 30.0, 1.0),
    );
    spec.check("sep", 176.0, 94.0, 240.0, "Use 1000 &Separator (,)", false);
    spec.label(176.0, 126.0, 80.0, "S&ymbol:");
    let symbols: Vec<&str> = SYMBOLS.iter().map(|s| s.0).collect();
    spec.dropdown("symbol", 260.0, 122.0, 160.0, &symbols, 0);
    spec.label(176.0, 158.0, 80.0, "Typ&e:");
    spec.dropdown(
        "datetype",
        260.0,
        154.0,
        160.0,
        &["Short Date", "Long Date"],
        0,
    );
    spec.label(176.0, 190.0, 80.0, "C&ustom code:");
    spec.text("code", 260.0, 186.0, 160.0, "");
    spec.on_page(Some(1));
    spec.label(12.0, 44.0, 90.0, "&Horizontal:");
    let h = match cur.halign {
        HAlign::General => 0,
        HAlign::Left => 1,
        HAlign::Center => 2,
        HAlign::Right => 3,
    };
    spec.dropdown(
        "halign",
        104.0,
        40.0,
        150.0,
        &["General", "Left (Indent)", "Center", "Right (Indent)"],
        h,
    );
    spec.label(280.0, 44.0, 50.0, "I&ndent:");
    spec.spin(
        "indent",
        336.0,
        40.0,
        70.0,
        (f64::from(cur.indent), 0.0, 250.0, 1.0),
    );
    spec.label(12.0, 78.0, 90.0, "&Vertical:");
    let v = match cur.valign {
        VAlign::Top => 0,
        VAlign::Center => 1,
        VAlign::Bottom => 2,
    };
    spec.dropdown(
        "valign",
        104.0,
        74.0,
        150.0,
        &["Top", "Center", "Bottom"],
        v,
    );
    spec.group(12.0, 108.0, 416.0, 100.0, "Text control");
    spec.check("wrap", 24.0, 130.0, 300.0, "&Wrap text", cur.wrap);
    spec.check("shrink", 24.0, 152.0, 300.0, "Shrin&k to fit", false)
        .disabled();
    spec.check("merge", 24.0, 174.0, 300.0, "&Merge cells", cur.merged);
    spec.group(12.0, 216.0, 416.0, 66.0, "Orientation");
    spec.label(24.0, 242.0, 70.0, "Degr&ees:");
    spec.spin(
        "deg",
        96.0,
        238.0,
        70.0,
        (f64::from(cur.rotation.unwrap_or(0)), -90.0, 90.0, 1.0),
    );
    spec.check(
        "stacked",
        190.0,
        240.0,
        200.0,
        "Vertical te&xt",
        cur.rotation.is_none(),
    );
    spec.on_page(Some(2));
    spec.label(12.0, 44.0, 200.0, "&Font:");
    let mut fonts: Vec<&str> = FONTS.to_vec();
    if !fonts.contains(&cur.font_name.as_str()) {
        fonts.insert(0, cur.font_name.as_str());
    }
    spec.combo("fname", 12.0, 64.0, 200.0, &fonts, &cur.font_name);
    spec.label(224.0, 44.0, 100.0, "Font st&yle:");
    let style = usize::from(cur.italic) + 2 * usize::from(cur.bold);
    spec.dropdown(
        "fstyle",
        224.0,
        64.0,
        100.0,
        &["Regular", "Italic", "Bold", "Bold Italic"],
        style,
    );
    spec.label(336.0, 44.0, 80.0, "&Size:");
    let size_text = if (cur.font_size - cur.font_size.round()).abs() < 1e-9 {
        format!("{}", cur.font_size.round() as i64)
    } else {
        format!("{}", cur.font_size)
    };
    spec.combo("fsize", 336.0, 64.0, 90.0, &SIZES, &size_text);
    spec.label(12.0, 104.0, 100.0, "&Underline:");
    let u = match cur.underline {
        Under::None => 0,
        Under::Single => 1,
        Under::Double => 2,
    };
    spec.dropdown(
        "under",
        12.0,
        124.0,
        200.0,
        &["None", "Single", "Double"],
        u,
    );
    spec.label(224.0, 104.0, 100.0, "&Color:");
    let (fc, fsel) = colour_items(cur.font_rgb.as_deref(), "Automatic");
    spec.dropdown("fcolour", 224.0, 124.0, 200.0, &items_ref(&fc), fsel);
    spec.label(12.0, 164.0, 100.0, "Effects");
    spec.check("strike", 24.0, 186.0, 200.0, "Stri&kethrough", false)
        .disabled();
    spec.on_page(Some(3));
    spec.group(12.0, 40.0, 416.0, 66.0, "Presets");
    spec.radio("pre_none", "preset", 24.0, 64.0, 100.0, "&None", false);
    spec.radio(
        "pre_outline",
        "preset",
        130.0,
        64.0,
        100.0,
        "&Outline",
        false,
    );
    spec.radio(
        "pre_all",
        "preset",
        236.0,
        64.0,
        180.0,
        "&All borders",
        false,
    );
    spec.group(12.0, 114.0, 416.0, 66.0, "Border");
    spec.check("b_top", 24.0, 138.0, 90.0, "&Top", false);
    spec.check("b_bottom", 130.0, 138.0, 90.0, "&Bottom", false);
    spec.check("b_left", 236.0, 138.0, 90.0, "&Left", false);
    spec.check("b_right", 330.0, 138.0, 90.0, "&Right", false);
    spec.label(12.0, 198.0, 50.0, "St&yle:");
    spec.dropdown(
        "bstyle",
        66.0,
        194.0,
        150.0,
        &["Thin", "Thick", "Double"],
        0,
    );
    spec.on_page(Some(4));
    spec.label(12.0, 44.0, 200.0, "Background &Color:");
    let (bc, bsel) = colour_items(cur.fill_rgb.as_deref(), "No Color");
    spec.dropdown("fill", 12.0, 64.0, 200.0, &items_ref(&bc), bsel);
    spec.label(12.0, 104.0, 140.0, "Custom (&RRGGBB):");
    spec.text("fillhex", 150.0, 100.0, 100.0, "");
    spec.on_page(Some(5));
    spec.check("locked", 12.0, 44.0, 300.0, "&Locked", cur.locked);
    spec.check("hidden", 12.0, 70.0, 300.0, "Hi&dden", cur.hidden);
    spec.note(
        12.0,
        104.0,
        410.0,
        32.0,
        "Locking cells or hiding formulas has no effect until you protect the worksheet.\nTo do this, on the Review tab, select Protect Sheet.",
    );
    spec.on_page(None);
    spec.start_page = tab.index();
    // The dialog opens on its tab strip, as Excel's does.
    let cat = cur.number_category;
    for c in &mut spec.controls {
        match c.id.as_str() {
            "dec" => c.enabled = matches!(cat, 1 | 2 | 3 | 6 | 8),
            "sep" => c.enabled = cat == 1,
            "symbol" => c.enabled = cat == 3,
            "datetype" => c.enabled = cat == 4,
            "code" => c.enabled = cat == 10,
            _ => {}
        }
    }
    (spec, Box::new(FormatCells { cur }))
}

impl FormatCells {
    fn sync_number(ui: &mut Ui, d: &mut Dialog) {
        let v = d.values(ui);
        let cat = v.index("cat").unwrap_or(0);
        d.set_enabled(ui, "dec", matches!(cat, 1 | 2 | 3 | 6 | 8));
        d.set_enabled(ui, "sep", cat == 1);
        d.set_enabled(ui, "symbol", cat == 3);
        d.set_enabled(ui, "datetype", cat == 4);
        d.set_enabled(ui, "code", cat == 10);
    }

    fn wanted(&self, v: &Values) -> Result<Wanted, String> {
        let cat = v.index("cat").unwrap_or(self.cur.number_category);
        let size: f64 = v.text("fsize").trim().parse().unwrap_or(f64::NAN);
        if !(1.0..=409.0).contains(&size) {
            return Err("The number must be between 1 and 409.".to_owned());
        }
        let style = v.index("fstyle").unwrap_or(0);
        let fill_hex = v.text("fillhex").trim().trim_start_matches('#').to_owned();
        let fill_rgb = if fill_hex.is_empty() {
            None
        } else if fill_hex.len() == 6 && fill_hex.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(fill_hex.to_ascii_uppercase())
        } else {
            return Err(
                "Type the custom colour as six hexadecimal digits, for example FFC000.".to_owned(),
            );
        };
        let picked = |ix: usize, cur: &Option<String>| -> Option<String> {
            match COLOURS.get(ix) {
                Some((_, c)) => c.map(str::to_owned),
                None => cur.clone(),
            }
        };
        let font_rgb = picked(v.index("fcolour").unwrap_or(0), &self.cur.font_rgb);
        let fill_pick = picked(v.index("fill").unwrap_or(0), &self.cur.fill_rgb);
        let preset = match v.radio("preset") {
            "pre_none" => Some("none".to_owned()),
            "pre_outline" => Some("outline".to_owned()),
            "pre_all" => Some("all".to_owned()),
            _ => None,
        };
        Ok(Wanted {
            number_category: cat,
            decimals: v.num("dec") as i32,
            thousands: v.flag("sep"),
            symbol: SYMBOLS[v.index("symbol").unwrap_or(0).min(SYMBOLS.len() - 1)]
                .1
                .to_owned(),
            date_type: if v.index("datetype") == Some(1) {
                "long"
            } else {
                "short"
            }
            .to_owned(),
            custom_code: v.text("code").to_owned(),
            halign: [HAlign::General, HAlign::Left, HAlign::Center, HAlign::Right]
                [v.index("halign").unwrap_or(0).min(3)],
            valign: [VAlign::Top, VAlign::Center, VAlign::Bottom]
                [v.index("valign").unwrap_or(2).min(2)],
            wrap: v.flag("wrap"),
            indent: v.num("indent").max(0.0) as u32,
            rotation: if v.flag("stacked") {
                None
            } else {
                Some(v.num("deg") as i32)
            },
            merged: v.flag("merge"),
            font_name: v.text("fname").trim().to_owned(),
            font_size: size,
            bold: style >= 2,
            italic: style % 2 == 1,
            underline: [Under::None, Under::Single, Under::Double]
                [v.index("under").unwrap_or(0).min(2)],
            font_rgb,
            fill_rgb: fill_rgb.or(fill_pick),
            locked: v.flag("locked"),
            hidden: v.flag("hidden"),
            border: BorderWish {
                preset,
                top: v.flag("b_top"),
                bottom: v.flag("b_bottom"),
                left: v.flag("b_left"),
                right: v.flag("b_right"),
                style: ["thin", "thick", "double"][v.index("bstyle").unwrap_or(0).min(2)]
                    .to_owned(),
            },
        })
    }
}

impl View for FormatCells {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        if let Outcome::Changed(id) = o
            && id == "cat"
        {
            let cat = d.values(ui).index("cat").unwrap_or(0);
            d.set_number(ui, "dec", f64::from(default_decimals(cat)));
            Self::sync_number(ui, d);
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let want = self.wanted(v)?;
        let commands = plan(&self.cur, &want);
        s.run_ids(ui, &commands);
        // Apply keeps the dialog open: what it did is the new starting point.
        self.cur = current_state(s);
        Ok(())
    }
}

// ---- Row Height and Column Width -------------------------------------------------------------------

#[derive(Debug)]
struct Dimension {
    rows: bool,
}

fn row_height(s: &mut Shell) -> (Spec, Box<dyn View>) {
    let h = s.sheet.home_row_state(s.sheet.active.0).0.unwrap_or(15.0);
    let mut spec = Spec::new("Row Height", 260.0, 56.0);
    spec.label(12.0, 12.0, 90.0, "Row &height:");
    spec.text("value", 110.0, 8.0, 130.0, &trim_num(h));
    spec.focus = Some("value".to_owned());
    (spec, Box::new(Dimension { rows: true }))
}

fn column_width(s: &mut Shell) -> (Spec, Box<dyn View>) {
    // The stored width includes the cell padding; the box shows the characters.
    let w = s
        .sheet
        .home_col_state(s.sheet.active.1)
        .map_or(8.43, |(w, _)| (w - 0.7109375).max(0.0));
    let mut spec = Spec::new("Column Width", 260.0, 56.0);
    spec.label(12.0, 12.0, 90.0, "Column &width:");
    spec.text(
        "value",
        110.0,
        8.0,
        130.0,
        &trim_num((w * 100.0).round() / 100.0),
    );
    spec.focus = Some("value".to_owned());
    (spec, Box::new(Dimension { rows: false }))
}

fn trim_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v}")
    }
}

impl View for Dimension {
    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let (limit, id, msg) = if self.rows {
            (
                409.5,
                "home.cells.format.row-height",
                "Enter a number between 0 and 409.5.",
            )
        } else {
            (
                255.0,
                "home.cells.format.column-width",
                "Enter a number between 0 and 255.",
            )
        };
        let n: f64 = v.text("value").trim().parse().map_err(|_| msg.to_owned())?;
        if !(0.0..=limit).contains(&n) {
            return Err(msg.to_owned());
        }
        s.run_id(ui, &format!("{id}={}", trim_num(n)));
        Ok(())
    }
}

// ---- Merge confirmation and validation alerts ----------------------------------------------------

#[derive(Debug)]
struct Confirm {
    command: String,
}

fn merge_confirm(command: &str) -> (Spec, Box<dyn View>) {
    let mut spec = Spec::new("Open365", 360.0, 60.0);
    for (i, line) in wrap(
        "Merging cells only keeps the upper-left value, and discards other values.",
        56,
        2,
    )
    .iter()
    .enumerate()
    {
        spec.label(12.0, 12.0 + 18.0 * i as f32, 336.0, line);
    }
    (
        spec,
        Box::new(Confirm {
            command: command.to_owned(),
        }),
    )
}

impl View for Confirm {
    fn commit(
        &mut self,
        s: &mut Shell,
        ui: &mut Ui,
        _d: &mut Dialog,
        _v: &Values,
    ) -> Result<(), String> {
        s.run_id(ui, &self.command);
        Ok(())
    }
}

#[derive(Debug)]
struct Alert {
    cell: (u32, u32),
    text: String,
    stop: bool,
}

fn validation_alert(
    row: u32,
    col: u32,
    text: &str,
    style: &str,
    title: &str,
    message: &str,
) -> (Spec, Box<dyn View>) {
    let stop = style == "stop";
    let warning = style == "warning";
    let title = if title.is_empty() { "Open365" } else { title };
    let footer = if stop {
        vec![FooterButton::new("cancel", "Cancel", ButtonRole::Cancel)]
    } else if warning {
        vec![
            FooterButton::new("ok", "&Yes", ButtonRole::Default),
            FooterButton::new("no", "&No", ButtonRole::Normal),
            FooterButton::new("cancel", "Cancel", ButtonRole::Cancel),
        ]
    } else {
        vec![
            FooterButton::new("ok", "OK", ButtonRole::Default),
            FooterButton::new("cancel", "Cancel", ButtonRole::Cancel),
        ]
    };
    let mut lines = wrap(message, 52, 4);
    if warning {
        lines.push("Do you want to continue?".to_owned());
    }
    let mut spec = Spec::new(title, 360.0, 20.0 + 18.0 * lines.len() as f32).footer(footer);
    if stop {
        // Retry returns to editing the cell, which Cancel also does here.
        spec.footer
            .insert(0, FooterButton::new("retry", "&Retry", ButtonRole::Default));
    }
    for (i, l) in lines.iter().enumerate() {
        spec.label(12.0, 12.0 + 18.0 * i as f32, 336.0, l);
    }
    (
        spec,
        Box::new(Alert {
            cell: (row, col),
            text: text.to_owned(),
            stop,
        }),
    )
}

impl View for Alert {
    fn event(&mut self, _s: &mut Shell, _ui: &mut Ui, _d: &mut Dialog, o: &Outcome) {
        if let Outcome::Pressed(id) = o
            && (id == "no" || id == "retry")
        {
            request_close();
        }
    }

    fn commit(
        &mut self,
        s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        _v: &Values,
    ) -> Result<(), String> {
        if self.stop {
            return Ok(());
        }
        s.sheet.enter(self.cell.0, self.cell.1, &self.text)
    }
}
