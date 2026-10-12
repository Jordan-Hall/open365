//! The Format Cells planner: from what the active cell has now and what the dialog asks for to
//! the ribbon commands (`home.*`) that get there. Every command is the plain id or an `=value`
//! form the Home tab already handles, so each is one undo step and nothing here touches the
//! model. Two things have no command yet and are emitted under the id the Home tab should
//! accept: `home.number.format-code=<code>` and `home.cells.format.protection=...`.

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
    pub merged: bool,
    pub font_name: String,
    pub font_size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: Under,
    /// `RRGGBB`, `None` for Automatic.
    pub font_rgb: Option<String>,
    pub fill_rgb: Option<String>,
    pub locked: bool,
    pub hidden: bool,
}

/// How the Border tab asks to change the borders.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BorderWish {
    /// `none`, `outline` or `all` from the presets; `None` leaves the preset alone.
    pub preset: Option<String>,
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
    /// `thin`, `thick` or `double`.
    pub style: String,
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
    pub merged: bool,
    pub font_name: String,
    pub font_size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: Under,
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

/// The orientation command nearest to `degrees`.
fn rotation_cmd(degrees: Option<i32>) -> &'static str {
    match degrees {
        None => "home.alignment.orientation.vertical-text",
        Some(d) => match d {
            i32::MIN..=-68 => "home.alignment.orientation.rotate-down",
            -67..=-23 => "home.alignment.orientation.angle-clockwise",
            -22..=22 => "home.alignment.orientation.horizontal",
            23..=67 => "home.alignment.orientation.angle-counterclockwise",
            _ => "home.alignment.orientation.rotate-up",
        },
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
    if want.rotation != cur.rotation {
        out.push(rotation_cmd(want.rotation).to_owned());
    }

    // Border.
    let b = &want.border;
    if let Some(p) = &b.preset {
        out.push(match p.as_str() {
            "none" => "home.font.borders.none".to_owned(),
            "all" => "home.font.borders.all".to_owned(),
            _ if b.style == "thick" => "home.font.borders.thick-outside".to_owned(),
            _ => "home.font.borders.outside".to_owned(),
        });
    }
    for (on, edge) in [
        (b.top, "top"),
        (b.bottom, "bottom"),
        (b.left, "left"),
        (b.right, "right"),
    ] {
        if !on {
            continue;
        }
        out.push(match (edge, b.style.as_str()) {
            ("bottom", "double") => "home.font.borders.bottom-double".to_owned(),
            ("bottom", "thick") => "home.font.borders.bottom-thick".to_owned(),
            _ => format!("home.font.borders.{edge}"),
        });
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
            merged: false,
            font_name: "Calibri".into(),
            font_size: 11.0,
            bold: false,
            italic: false,
            underline: Under::None,
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
            merged: c.merged,
            font_name: c.font_name.clone(),
            font_size: c.font_size,
            bold: c.bold,
            italic: c.italic,
            underline: c.underline,
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
    fn border_fill_protection_and_merge() {
        let c = cur();
        let mut w = same(&c);
        w.border.preset = Some("outline".into());
        w.border.style = "thick".into();
        w.border.bottom = true;
        w.fill_rgb = Some("FFFF00".into());
        w.locked = false;
        w.merged = true;
        assert_eq!(
            plan(&c, &w),
            vec![
                "home.font.borders.thick-outside",
                "home.font.borders.bottom-thick",
                "home.font.fill-colour.rgb-FFFF00",
                "home.cells.format.protection=locked=0;hidden=0",
                "home.alignment.merge-centre.cells",
            ]
        );
    }
}
