//! The Format Cells planner: from what the active cell has now and what the dialog asks for to
//! the ribbon commands (`home.*`) that get there. Every command is the plain id or an `=value`
//! form the Home tab already handles, so each is one undo step and nothing here touches the
//! model. Two things have no command yet and are emitted under the id the Home tab should
//! accept: `home.number.format-code=<code>` and `home.cells.format.protection=...`.
//!
//! What the Home tab has no button for (strikethrough, shrink to fit, any text angle, the
//! style and colour of each border edge) is not a command id: [`format_change`] and
//! [`border_change`] build the model's own change records for `SheetModel::format_selection`.

use open365_commands::{AlignmentChange, BorderChange, CellFormatChange, ColorSpec, FontChange};
use open365_commands::{LineSpec, RotationSpec};

/// Horizontal alignment as the dialog offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    General,
    Left,
    Center,
    Right,
}

/// Vertical alignment as the dialog offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Center,
    Bottom,
}

/// Underline kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Under {
    None,
    Single,
    Double,
}

/// The number categories of the Number tab.
pub const CATEGORIES: [&str; 11] = [
    "General",
    "Number",
    "Currency",
    "Accounting",
    "Date",
    "Time",
    "Percentage",
    "Fraction",
    "Scientific",
    "Text",
    "Custom",
];

/// Decimal places a category starts with.
#[must_use]
pub fn default_decimals(category: usize) -> i32 {
    match category {
        1..=3 | 6 | 8 => 2,
        _ => 0,
    }
}

/// The category a number format code belongs to, and its decimal places.
#[must_use]
pub fn category_of(code: &str) -> (usize, i32) {
    let c = code.trim();
    let decimals = c.split_once('.').map_or(0, |(_, rest)| {
        rest.chars().take_while(|ch| *ch == '0').count() as i32
    });
    let lower = c.to_ascii_lowercase();
    let cat = if c.is_empty() || lower == "general" {
        0
    } else if c == "@" {
        9
    } else if lower.contains("e+") || lower.contains("e-") {
        8
    } else if c.contains('%') {
        6
    } else if c.contains("?/") || c.contains("#/") {
        7
    } else if c.contains("_(") || c.contains("* ") {
        3
    } else if lower.contains('y')
        || (lower.contains('d') && lower.contains('m') && !lower.contains('0'))
    {
        4
    } else if (lower.contains('h') || lower.contains("ss")) && !lower.contains('0') {
        5
    } else if c.contains('$') || c.contains('£') || c.contains('€') || c.contains('¥') {
        2
    } else if c == "0" || c.starts_with("0.") || c.starts_with("#,##0") || c == "0.00" {
        1
    } else {
        10
    };
    (cat, decimals)
}

/// The cell's state the planner compares against.
#[derive(Debug, Clone, PartialEq)]
pub struct Current {
    pub number_category: usize,
    pub decimals: i32,
    pub halign: HAlign,
    pub valign: VAlign,
    pub wrap: bool,
    pub indent: u32,
    /// Degrees counter-clockwise (-90 to 90), `None` for stacked text.
    pub rotation: Option<i32>,
    pub shrink: bool,
    pub merged: bool,
    pub font_name: String,
    pub font_size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: Under,
    pub strike: bool,
    /// `RRGGBB`, `None` for Automatic.
    pub font_rgb: Option<String>,
    pub fill_rgb: Option<String>,
    pub locked: bool,
    pub hidden: bool,
}

/// The line styles of the Border tab's list, in Excel's order: `(caption, OOXML name)`. The
/// first, None, removes a line.
pub const LINE_STYLES: [(&str, &str); 14] = [
    ("None", "none"),
    ("Hair", "hair"),
    ("Dotted", "dotted"),
    ("Dash-dot-dot", "dashDotDot"),
    ("Dash-dot", "dashDot"),
    ("Dashed", "dashed"),
    ("Thin", "thin"),
    ("Medium dash-dot-dot", "mediumDashDotDot"),
    ("Slanted dash-dot", "slantDashDot"),
    ("Medium dash-dot", "mediumDashDot"),
    ("Medium dashed", "mediumDashed"),
    ("Medium", "medium"),
    ("Thick", "thick"),
    ("Double", "double"),
];

/// The index in [`LINE_STYLES`] of an OOXML style name (0, None, for an unknown one).
#[must_use]
pub fn line_style_index(ooxml: &str) -> usize {
    LINE_STYLES
        .iter()
        .position(|(_, n)| *n == ooxml)
        .unwrap_or(0)
}

/// One line of the border diagram: a style (an index into [`LINE_STYLES`], never 0) and a
/// colour (`RRGGBB`, `None` for Automatic).
pub type EdgeLine = (usize, Option<String>);

/// The eight edges of the diagram, in the order of `ui_core::dialog::BORDER_EDGES`: top, inside
/// horizontal, bottom, left, inside vertical, right, diagonal down, diagonal up. `None` is no
/// line.
pub type Edges = [Option<EdgeLine>; 8];

/// What the Border tab asks for: the edges as they were and as they are now. Only the edges
/// that differ are written.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BorderWish {
    pub before: Edges,
    pub after: Edges,
}

/// The border change for the edges the user changed; `None` when none changed.
#[must_use]
pub fn border_change(w: &BorderWish) -> Option<BorderChange> {
    let mut change = BorderChange::default();
    let mut any = false;
    for k in 0..8 {
        if w.before[k] == w.after[k] {
            continue;
        }
        any = true;
        let line = Some(match &w.after[k] {
            Some((style, rgb)) => LineSpec {
                style: LINE_STYLES[(*style).min(LINE_STYLES.len() - 1)]
                    .1
                    .to_owned(),
                color: rgb.as_deref().map(ColorSpec::rgb),
            },
            None => LineSpec {
                style: "none".to_owned(),
                color: None,
            },
        });
        match k {
            0 => change.top = line,
            1 => change.inside_horizontal = line,
            2 => change.bottom = line,
            3 => change.left = line,
            4 => change.inside_vertical = line,
            5 => change.right = line,
            6 => change.diagonal_down = line,
            _ => change.diagonal_up = line,
        }
    }
    any.then_some(change)
}

/// The change for strikethrough, shrink to fit and the text angle when they differ from what
/// the cell has; `None` when none does.
#[must_use]
pub fn format_change(cur: &Current, want: &Wanted) -> Option<CellFormatChange> {
    let mut change = CellFormatChange::default();
    if want.strike != cur.strike {
        change.font = FontChange {
            strike: Some(want.strike),
            ..FontChange::default()
        };
    }
    let mut alignment = AlignmentChange::default();
    if want.shrink != cur.shrink {
        alignment.shrink_to_fit = Some(want.shrink);
    }
    if want.rotation != cur.rotation {
        alignment.rotation = Some(match want.rotation {
            None => RotationSpec::Stacked,
            Some(d) => RotationSpec::Degrees(d.clamp(-90, 90)),
        });
    }
    change.alignment = alignment;
    (change != CellFormatChange::default()).then_some(change)
}

/// What the dialog asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Wanted {
    pub number_category: usize,
    pub decimals: i32,
    pub thousands: bool,
    /// `usd`, `gbp`, `eur`, `jpy`, `chf` for Accounting.
    pub symbol: String,
    /// `short` or `long` for Date.
    pub date_type: String,
    pub custom_code: String,
    pub halign: HAlign,
    pub valign: VAlign,
    pub wrap: bool,
    pub indent: u32,
    pub rotation: Option<i32>,
    pub shrink: bool,
    pub merged: bool,
    pub font_name: String,
    pub font_size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: Under,
    pub strike: bool,
    pub font_rgb: Option<String>,
    pub fill_rgb: Option<String>,
    pub locked: bool,
    pub hidden: bool,
    pub border: BorderWish,
}

fn halign_cmd(a: HAlign) -> Option<&'static str> {
    match a {
        HAlign::General => None,
        HAlign::Left => Some("home.alignment.align-left"),
        HAlign::Center => Some("home.alignment.centre"),
        HAlign::Right => Some("home.alignment.align-right"),
    }
}

fn valign_cmd(a: VAlign) -> Option<&'static str> {
    match a {
        VAlign::Top => Some("home.alignment.top-align"),
        VAlign::Center => Some("home.alignment.middle-align"),
        VAlign::Bottom => None,
    }
}

fn rgb_cmd(base: &str, rgb: Option<&str>, none: &str) -> String {
    match rgb {
        Some(c) => format!(
            "{base}.rgb-{}",
            c.trim_start_matches('#').to_ascii_uppercase()
        ),
        None => format!("{base}.{none}"),
    }
}

/// The commands that turn `cur` into `want`, in an order that does not undo itself (indent
/// before alignment, merge last).
#[must_use]
pub fn plan(cur: &Current, want: &Wanted) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    // Number.
    let number_changed =
        want.number_category != cur.number_category || want.decimals != cur.decimals;
    if number_changed {
        match want.number_category {
            0 => out.push("home.number.number-format=General".to_owned()),
            1 if want.thousands => out.push("home.number.comma-style".to_owned()),
            1 => out.push("home.number.number-format=Number".to_owned()),
            2 => out.push("home.number.number-format=Currency".to_owned()),
            3 => out.push(format!(
                "home.number.accounting-number-format.{}",
                want.symbol
            )),
            4 => out.push(format!(
                "home.number.number-format={}",
                if want.date_type == "long" {
                    "Long Date"
                } else {
                    "Short Date"
                }
            )),
            5 => out.push("home.number.number-format=Time".to_owned()),
            6 => out.push("home.number.number-format=Percentage".to_owned()),
            7 => out.push("home.number.number-format=Fraction".to_owned()),
            8 => out.push("home.number.number-format=Scientific".to_owned()),
            9 => out.push("home.number.number-format=Text".to_owned()),
            _ => {
                if !want.custom_code.trim().is_empty() {
                    out.push(format!(
                        "home.number.format-code={}",
                        want.custom_code.trim()
                    ));
                }
            }
        }
        if matches!(want.number_category, 1 | 2 | 3 | 6 | 8) {
            let base = if want.number_category == cur.number_category
                && !number_changed_category(cur, want)
            {
                cur.decimals
            } else {
                default_decimals(want.number_category)
            };
            let steps = want.decimals - base;
            let cmd = if steps >= 0 {
                "home.number.increase-decimal"
            } else {
                "home.number.decrease-decimal"
            };
            for _ in 0..steps.unsigned_abs().min(30) {
                out.push(cmd.to_owned());
            }
        }
    }

    // Font.
    if want.font_name != cur.font_name && !want.font_name.trim().is_empty() {
        out.push(format!("home.font.font={}", want.font_name.trim()));
    }
    if (want.font_size - cur.font_size).abs() > 1e-9 {
        out.push(format!("home.font.font-size={}", fmt(want.font_size)));
    }
    if want.bold != cur.bold {
        out.push("home.font.bold".to_owned());
    }
    if want.italic != cur.italic {
        out.push("home.font.italic".to_owned());
    }
    if want.underline != cur.underline {
        match (cur.underline, want.underline) {
            (Under::Single, Under::None) | (_, Under::Single) => {
                out.push("home.font.underline.single".to_owned())
            }
            (Under::Double, Under::None) | (_, Under::Double) => {
                out.push("home.font.underline.double".to_owned())
            }
            _ => {}
        }
    }
    if want.font_rgb != cur.font_rgb {
        out.push(rgb_cmd(
            "home.font.font-colour",
            want.font_rgb.as_deref(),
            "auto",
        ));
    }

    // Alignment.
    if want.wrap != cur.wrap {
        out.push("home.alignment.wrap-text".to_owned());
    }
    if want.indent != cur.indent {
        let (cmd, n) = if want.indent > cur.indent {
            ("home.alignment.increase-indent", want.indent - cur.indent)
        } else {
            ("home.alignment.decrease-indent", cur.indent - want.indent)
        };
        for _ in 0..n.min(250) {
            out.push(cmd.to_owned());
        }
    }
    if want.halign != cur.halign || (want.indent != cur.indent && want.halign == HAlign::General) {
        // An indent makes a General cell left aligned; look at what that leaves.
        let after_indent = if want.indent > cur.indent && cur.halign == HAlign::General {
            HAlign::Left
        } else {
            cur.halign
        };
        if want.halign != after_indent {
            if want.halign == HAlign::General {
                if let Some(c) = halign_cmd(after_indent) {
                    out.push(c.to_owned());
                }
            } else if let Some(c) = halign_cmd(want.halign) {
                out.push(c.to_owned());
            }
        }
    }
    if want.valign != cur.valign {
        if want.valign == VAlign::Bottom {
            if let Some(c) = valign_cmd(cur.valign) {
                out.push(c.to_owned());
            }
        } else if let Some(c) = valign_cmd(want.valign) {
            out.push(c.to_owned());
        }
    }

    // Fill.
    if want.fill_rgb != cur.fill_rgb {
        out.push(rgb_cmd(
            "home.font.fill-colour",
            want.fill_rgb.as_deref(),
            "no-fill",
        ));
    }

    // Protection.
    if want.locked != cur.locked || want.hidden != cur.hidden {
        out.push(format!(
            "home.cells.format.protection=locked={};hidden={}",
            u8::from(want.locked),
            u8::from(want.hidden)
        ));
    }

    // Merge last: it changes which cell the formats above applied to.
    if want.merged != cur.merged {
        out.push(
            if want.merged {
                "home.alignment.merge-centre.cells"
            } else {
                "home.alignment.merge-centre.unmerge"
            }
            .to_owned(),
        );
    }
    out
}

fn number_changed_category(cur: &Current, want: &Wanted) -> bool {
    cur.number_category != want.number_category
}

fn fmt(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v}")
    }
}

/// Named colours of the Font Colour and Fill boxes: `(name, RRGGBB)`; `None` is Automatic or
/// No Fill.
pub const COLOURS: [(&str, Option<&str>); 10] = [
    ("Automatic", None),
    ("Black", Some("000000")),
    ("White", Some("FFFFFF")),
    ("Red", Some("FF0000")),
    ("Orange", Some("FFC000")),
    ("Yellow", Some("FFFF00")),
    ("Green", Some("00B050")),
    ("Blue", Some("0070C0")),
    ("Purple", Some("7030A0")),
    ("Gray", Some("808080")),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn cur() -> Current {
        Current {
            number_category: 0,
            decimals: 0,
            halign: HAlign::General,
            valign: VAlign::Bottom,
            wrap: false,
            indent: 0,
            rotation: Some(0),
            shrink: false,
            merged: false,
            font_name: "Calibri".into(),
            font_size: 11.0,
            bold: false,
            italic: false,
            underline: Under::None,
            strike: false,
            font_rgb: None,
            fill_rgb: None,
            locked: true,
            hidden: false,
        }
    }

    fn same(c: &Current) -> Wanted {
        Wanted {
            number_category: c.number_category,
            decimals: c.decimals,
            thousands: false,
            symbol: "usd".into(),
            date_type: "short".into(),
            custom_code: String::new(),
            halign: c.halign,
            valign: c.valign,
            wrap: c.wrap,
            indent: c.indent,
            rotation: c.rotation,
            shrink: c.shrink,
            merged: c.merged,
            font_name: c.font_name.clone(),
            font_size: c.font_size,
            bold: c.bold,
            italic: c.italic,
            underline: c.underline,
            strike: c.strike,
            font_rgb: c.font_rgb.clone(),
            fill_rgb: c.fill_rgb.clone(),
            locked: c.locked,
            hidden: c.hidden,
            border: BorderWish::default(),
        }
    }

    #[test]
    fn an_untouched_dialog_plans_nothing() {
        let c = cur();
        assert!(plan(&c, &same(&c)).is_empty());
    }

    #[test]
    fn categories_come_from_codes() {
        assert_eq!(category_of("General"), (0, 0));
        assert_eq!(category_of("0.00"), (1, 2));
        assert_eq!(category_of("0.0%"), (6, 1));
        assert_eq!(category_of("dd/mm/yyyy"), (4, 0));
        assert_eq!(category_of("h:mm:ss"), (5, 0));
        assert_eq!(category_of("@"), (9, 0));
        assert_eq!(category_of("0.00E+00"), (8, 2));
    }

    #[test]
    fn number_decimals_and_font_changes_become_commands() {
        let c = cur();
        let mut w = same(&c);
        w.number_category = 1;
        w.decimals = 4;
        w.bold = true;
        w.font_size = 14.0;
        w.font_rgb = Some("FF0000".into());
        assert_eq!(
            plan(&c, &w),
            vec![
                "home.number.number-format=Number",
                "home.number.increase-decimal",
                "home.number.increase-decimal",
                "home.font.font-size=14",
                "home.font.bold",
                "home.font.font-colour.rgb-FF0000",
            ]
        );
    }

    #[test]
    fn alignment_toggles_reach_the_target_from_the_current_state() {
        let mut c = cur();
        c.halign = HAlign::Left;
        let mut w = same(&c);
        w.halign = HAlign::General;
        assert_eq!(plan(&c, &w), vec!["home.alignment.align-left"]);
        w.halign = HAlign::Right;
        assert_eq!(plan(&c, &w), vec!["home.alignment.align-right"]);
        let mut c2 = cur();
        c2.valign = VAlign::Top;
        let mut w2 = same(&c2);
        w2.valign = VAlign::Bottom;
        assert_eq!(plan(&c2, &w2), vec!["home.alignment.top-align"]);
    }

    #[test]
    fn indent_comes_before_alignment_and_accounts_for_the_left_it_implies() {
        let c = cur();
        let mut w = same(&c);
        w.indent = 2;
        w.halign = HAlign::Left;
        let p = plan(&c, &w);
        assert_eq!(
            p,
            vec![
                "home.alignment.increase-indent",
                "home.alignment.increase-indent"
            ]
        );
    }

    #[test]
    fn fill_protection_and_merge() {
        let c = cur();
        let mut w = same(&c);
        w.fill_rgb = Some("FFFF00".into());
        w.locked = false;
        w.merged = true;
        assert_eq!(
            plan(&c, &w),
            vec![
                "home.font.fill-colour.rgb-FFFF00",
                "home.cells.format.protection=locked=0;hidden=0",
                "home.alignment.merge-centre.cells",
            ]
        );
    }

    #[test]
    fn strikethrough_shrink_and_any_angle_become_one_change() {
        let c = cur();
        assert_eq!(format_change(&c, &same(&c)), None);
        let mut w = same(&c);
        w.strike = true;
        w.shrink = true;
        w.rotation = Some(-37);
        let change = format_change(&c, &w).expect("a change");
        assert_eq!(change.font.strike, Some(true));
        assert_eq!(change.alignment.shrink_to_fit, Some(true));
        assert_eq!(change.alignment.rotation, Some(RotationSpec::Degrees(-37)));
        w.rotation = None;
        assert_eq!(
            format_change(&c, &w).and_then(|c| c.alignment.rotation),
            Some(RotationSpec::Stacked)
        );
        // The plan itself leaves all three alone.
        assert!(plan(&c, &w).is_empty());
    }

    #[test]
    fn only_the_edges_that_changed_are_written_with_their_style_and_colour() {
        let mut w = BorderWish::default();
        assert_eq!(border_change(&w), None);
        w.after[0] = Some((line_style_index("thick"), Some("FF0000".into())));
        w.after[7] = Some((line_style_index("dashed"), None));
        w.before[2] = Some((line_style_index("thin"), None));
        w.after[2] = Some((line_style_index("thin"), None));
        w.before[3] = Some((line_style_index("double"), None));
        let b = border_change(&w).expect("changes");
        assert_eq!(b.top.as_ref().map(|l| l.style.as_str()), Some("thick"));
        assert_eq!(
            b.top.and_then(|l| l.color).and_then(|c| c.rgb),
            Some("FF0000".to_owned())
        );
        assert_eq!(b.diagonal_up.map(|l| l.style), Some("dashed".to_owned()));
        assert_eq!(b.bottom, None, "unchanged");
        assert_eq!(b.left.map(|l| l.style), Some("none".to_owned()), "removed");
    }
}
