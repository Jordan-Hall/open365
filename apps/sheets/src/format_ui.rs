//! The Home tab's formatting controls: the Font, Alignment, Number and Styles groups, Format
//! Painter and Clear Formats. Everything here maps a control id plus the state of the active cell
//! to `open365-commands` commands, so the logic runs (and is tested) without a window.

use open365_commands::{
    AlignmentChange, BorderChange, CellFormatChange, ColorSpec, Command, FillChange, FontChange,
    HAlign, LineSpec, MergeMode, NumberFormatSpec, RangeRef, RotationSpec, UnderlineKind, VAlign,
    merge_discards,
};
use open365_ui_shell::{CommandState, MenuEntry};
use open365_xlsx::{
    CellFormat, Color, ColorSource, HorizontalAlignment, Rgb, Underline, VerticalAlignment,
    Workbook,
};

/// What the formatting commands act on: the sheet, the active cell and the selected range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sel {
    pub sheet: usize,
    /// `(row, col)`; the cell whose format the buttons show.
    pub active: (u32, u32),
    /// `(row1, col1, row2, col2)`.
    pub range: (u32, u32, u32, u32),
}

impl Sel {
    fn range_ref(self) -> RangeRef {
        RangeRef {
            first_row: self.range.0,
            first_col: self.range.1,
            last_row: self.range.2,
            last_col: self.range.3,
        }
    }
}

/// Which colour a palette chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourSlot {
    Fill,
    Font,
}

/// What running a control produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Apply(Vec<Command>),
    /// Ask first (Excel's "Merging cells only keeps the upper-left value" prompt); run the command on OK.
    Confirm {
        text: &'static str,
        command: Command,
    },
}

/// The prompt Excel shows when a merge would discard values.
pub const MERGE_WARNING: &str =
    "Merging cells only keeps the upper-left value and discards other values.";

/// Whether a merged cell touches the selection.
fn touches_merge(wb: &Workbook, sel: Sel) -> bool {
    let (r1, c1, r2, c2) = sel.range;
    wb.sheet(sel.sheet).is_some_and(|ws| {
        ws.merged_cells()
            .iter()
            .any(|&(a, b, c, d)| a <= r2 && r1 <= c && b <= c2 && c1 <= d)
    })
}

/// The command for a Merge & Centre menu item, behind the prompt when it would discard values.
fn merge_action(wb: &Workbook, sel: Sel, mode: MergeMode) -> Action {
    let command = Command::MergeCells {
        sheet: sel.sheet,
        range: sel.range_ref(),
        mode,
    };
    let loses = mode != MergeMode::Unmerge
        && merge_discards(wb, sel.sheet, sel.range_ref(), mode).is_ok_and(|d| !d.is_empty());
    if loses {
        Action::Confirm {
            text: MERGE_WARNING,
            command,
        }
    } else {
        Action::Apply(vec![command])
    }
}

/// Format Painter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Painter {
    Off,
    /// Applies to the next range chosen, then switches off (single click).
    Once((u32, u32, u32, u32)),
    /// Stays on until Escape (double click).
    Locked((u32, u32, u32, u32)),
}

/// Two clicks on Format Painter within this many seconds are a double click.
const DOUBLE_CLICK: f64 = 0.5;

/// Memory of the formatting controls between clicks.
#[derive(Debug, Clone)]
pub struct FormatUi {
    pub painter: Painter,
    /// A press happened in the grid since the painter was armed.
    pub painter_pressed: bool,
    last_painter_click: Option<f64>,
    last_fill: ColorSpec,
    last_font: ColorSpec,
    last_border: &'static str,
}

impl Default for FormatUi {
    fn default() -> Self {
        Self {
            painter: Painter::Off,
            painter_pressed: false,
            last_painter_click: None,
            // Excel starts with yellow for Fill Colour, red for Font Colour and the bottom border.
            last_fill: ColorSpec::rgb("FFFF00"),
            last_font: ColorSpec::rgb("FF0000"),
            last_border: "bottom",
        }
    }
}

const FONT_SIZES: [f64; 16] = [
    8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0,
];

const FONT_NAMES: [&str; 12] = [
    "Aptos Narrow",
    "Aptos",
    "Arial",
    "Calibri",
    "Cambria",
    "Consolas",
    "Courier New",
    "Georgia",
    "Segoe UI",
    "Tahoma",
    "Times New Roman",
    "Verdana",
];

/// The number format list: (id suffix, label, format).
fn number_formats() -> [(&'static str, &'static str, NumberFormatSpec); 11] {
    [
        ("general", "General", NumberFormatSpec::Builtin(0)),
        ("number", "Number", NumberFormatSpec::Builtin(2)),
        (
            "currency",
            "Currency",
            NumberFormatSpec::Code("\"$\"#,##0.00".into()),
        ),
        ("accounting", "Accounting", NumberFormatSpec::Builtin(44)),
        ("short-date", "Short Date", NumberFormatSpec::Builtin(14)),
        (
            "long-date",
            "Long Date",
            NumberFormatSpec::Code("dddd\\,\\ mmmm\\ dd\\,\\ yyyy".into()),
        ),
        ("time", "Time", NumberFormatSpec::Builtin(19)),
        ("percentage", "Percentage", NumberFormatSpec::Builtin(10)),
        ("fraction", "Fraction", NumberFormatSpec::Builtin(12)),
        ("scientific", "Scientific", NumberFormatSpec::Builtin(11)),
        ("text", "Text", NumberFormatSpec::Builtin(49)),
    ]
}

/// The accounting button's currency choices: (id suffix, label, symbol).
const ACCOUNTING: [(&str, &str, &str); 4] = [
    ("usd", "$ English (United States)", "$"),
    ("gbp", "\u{a3} English (United Kingdom)", "\u{a3}"),
    ("eur", "\u{20ac} Euro (\u{20ac} 123)", "\u{20ac}"),
    ("jpy", "\u{a5} Japanese", "\u{a5}"),
];

fn accounting_code(symbol: &str) -> String {
    format!(
        "_(\"{symbol}\"* #,##0.00_);_(\"{symbol}\"* \\(#,##0.00\\);_(\"{symbol}\"* \"-\"??_);_(@_)"
    )
}

/// The category the Number Format box shows for a format.
pub fn number_category(id: u32, code: &str) -> &'static str {
    match id {
        0 => "General",
        1..=4 | 37..=40 => "Number",
        5..=8 => "Currency",
        9 | 10 => "Percentage",
        11 | 48 => "Scientific",
        12 | 13 => "Fraction",
        14 => "Short Date",
        15..=17 | 22 => "Date",
        18..=21 | 45..=47 => "Time",
        41..=44 => "Accounting",
        49 => "Text",
        _ => custom_category(code),
    }
}

fn custom_category(code: &str) -> &'static str {
    let section = code.split(';').next().unwrap_or_default();
    let mut bare = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in section.chars() {
        match (quoted, escaped, ch) {
            (_, true, _) => escaped = false,
            (false, false, '\\') => escaped = true,
            (_, false, '"') => quoted = !quoted,
            (false, false, c) => bare.push(c),
            _ => {}
        }
    }
    let lower = bare.to_ascii_lowercase();
    if lower == "general" {
        "General"
    } else if lower.contains('@') {
        "Text"
    } else if lower.contains('%') {
        "Percentage"
    } else if lower.contains("e+") || lower.contains("e-") {
        "Scientific"
    } else if lower.contains('/') && lower.contains('?') {
        "Fraction"
    } else if lower.contains(['d', 'y']) || (lower.contains('m') && !lower.contains(['h', 's'])) {
        "Date"
    } else if lower.contains(['h', 's']) {
        "Time"
    } else if lower.contains('*') && code.contains('"') {
        "Accounting"
    } else if code.contains('$') || code.contains('\u{a3}') || code.contains('\u{20ac}') {
        "Currency"
    } else if !section.contains('"')
        && lower.chars().all(|c| "0#?,. _()-".contains(c))
        && lower.contains('0')
    {
        "Number"
    } else {
        "Custom"
    }
}

fn font_change(font: FontChange) -> CellFormatChange {
    CellFormatChange {
        font,
        ..CellFormatChange::default()
    }
}

fn align_change(alignment: AlignmentChange) -> CellFormatChange {
    CellFormatChange {
        alignment,
        ..CellFormatChange::default()
    }
}

fn number_change(spec: NumberFormatSpec) -> CellFormatChange {
    CellFormatChange {
        number_format: Some(spec),
        ..CellFormatChange::default()
    }
}

fn format_cells(sel: Sel, change: CellFormatChange) -> Command {
    Command::FormatCells {
        sheet: sel.sheet,
        range: sel.range_ref(),
        change: Box::new(change),
    }
}

fn line(style: &str) -> Option<LineSpec> {
    Some(LineSpec {
        style: style.to_owned(),
        color: None,
    })
}

/// The border menu entries: (id suffix, label).
const BORDERS: [(&str, &str); 13] = [
    ("bottom", "Bottom Border"),
    ("top", "Top Border"),
    ("left", "Left Border"),
    ("right", "Right Border"),
    ("none", "No Border"),
    ("all", "All Borders"),
    ("outside", "Outside Borders"),
    ("thick-outside", "Thick Outside Borders"),
    ("bottom-double", "Bottom Double Border"),
    ("thick-bottom", "Thick Bottom Border"),
    ("top-bottom", "Top and Bottom Border"),
    ("top-thick-bottom", "Top and Thick Bottom Border"),
    ("top-double-bottom", "Top and Double Bottom Border"),
];

/// The borders a border menu entry draws.
pub fn border_change(which: &str) -> Option<BorderChange> {
    let mut b = BorderChange::default();
    match which {
        "bottom" => b.bottom = line("thin"),
        "top" => b.top = line("thin"),
        "left" => b.left = line("thin"),
        "right" => b.right = line("thin"),
        "none" => {
            for slot in [
                &mut b.left,
                &mut b.right,
                &mut b.top,
                &mut b.bottom,
                &mut b.inside_horizontal,
                &mut b.inside_vertical,
                &mut b.diagonal_down,
                &mut b.diagonal_up,
            ] {
                *slot = line("none");
            }
        }
        "all" => {
            for slot in [
                &mut b.left,
                &mut b.right,
                &mut b.top,
                &mut b.bottom,
                &mut b.inside_horizontal,
                &mut b.inside_vertical,
            ] {
                *slot = line("thin");
            }
        }
        "outside" => outline(&mut b, "thin"),
        "thick-outside" => outline(&mut b, "medium"),
        "bottom-double" => b.bottom = line("double"),
        "thick-bottom" => b.bottom = line("medium"),
        "top-bottom" => {
            b.top = line("thin");
            b.bottom = line("thin");
        }
        "top-thick-bottom" => {
            b.top = line("thin");
            b.bottom = line("medium");
        }
        "top-double-bottom" => {
            b.top = line("thin");
            b.bottom = line("double");
        }
        _ => return None,
    }
    Some(b)
}

fn outline(b: &mut BorderChange, style: &str) {
    b.left = line(style);
    b.right = line(style);
    b.top = line(style);
    b.bottom = line(style);
}

fn horizontal(h: HorizontalAlignment) -> HAlign {
    match h {
        HorizontalAlignment::General => HAlign::General,
        HorizontalAlignment::Left => HAlign::Left,
        HorizontalAlignment::Center => HAlign::Center,
        HorizontalAlignment::Right => HAlign::Right,
        HorizontalAlignment::Fill => HAlign::Fill,
        HorizontalAlignment::Justify => HAlign::Justify,
        HorizontalAlignment::CenterContinuous => HAlign::CenterContinuous,
        HorizontalAlignment::Distributed => HAlign::Distributed,
    }
}

/// The size Increase Font Size moves to from `size`.
pub fn next_font_size(size: f64) -> f64 {
    if size >= 72.0 {
        // Past the list Excel keeps growing in steps of eight, up to its limit.
        return ((size / 8.0).floor() * 8.0 + 8.0).min(409.0);
    }
    FONT_SIZES
        .iter()
        .copied()
        .find(|s| *s > size)
        .unwrap_or(72.0)
}

/// The size Decrease Font Size moves to from `size`.
pub fn previous_font_size(size: f64) -> f64 {
    if size > 72.0 {
        return (((size / 8.0).ceil() - 1.0) * 8.0).max(72.0);
    }
    if size <= 8.0 {
        return (size - 1.0).max(1.0);
    }
    FONT_SIZES
        .iter()
        .rev()
        .copied()
        .find(|s| *s < size)
        .unwrap_or(8.0)
}

fn size_label(size: f64) -> String {
    if size.fract().abs() < 1e-9 {
        format!("{size:.0}")
    } else {
        format!("{size}")
    }
}

/// A named colour in the palette.
#[derive(Debug, Clone, PartialEq)]
pub struct Swatch {
    pub name: String,
    pub color: ColorSpec,
}

/// The theme colours as Excel's picker lays them out: ten columns, a row of the base colours then
/// five tints of each.
pub fn theme_palette() -> Vec<Vec<Swatch>> {
    const COLUMNS: [(u32, &str, [f64; 6]); 10] = [
        (0, "Background 1", [0.0, -0.05, -0.15, -0.25, -0.35, -0.5]),
        (1, "Text 1", [0.0, 0.5, 0.35, 0.25, 0.15, 0.05]),
        (2, "Background 2", [0.0, -0.1, -0.25, -0.5, -0.75, -0.9]),
        (3, "Text 2", [0.0, 0.9, 0.75, 0.5, -0.25, -0.5]),
        (4, "Accent 1", [0.0, 0.8, 0.6, 0.4, -0.25, -0.5]),
        (5, "Accent 2", [0.0, 0.8, 0.6, 0.4, -0.25, -0.5]),
        (6, "Accent 3", [0.0, 0.8, 0.6, 0.4, -0.25, -0.5]),
        (7, "Accent 4", [0.0, 0.8, 0.6, 0.4, -0.25, -0.5]),
        (8, "Accent 5", [0.0, 0.8, 0.6, 0.4, -0.25, -0.5]),
        (9, "Accent 6", [0.0, 0.8, 0.6, 0.4, -0.25, -0.5]),
    ];
    (0..6)
        .map(|row| {
            COLUMNS
                .iter()
                .map(|(index, name, tints)| {
                    let tint = tints[row];
                    let name = if tint == 0.0 {
                        (*name).to_owned()
                    } else {
                        format!(
                            "{name}, {} {:.0}%",
                            if tint > 0.0 { "Lighter" } else { "Darker" },
                            tint.abs() * 100.0
                        )
                    };
                    Swatch {
                        name,
                        color: ColorSpec::theme(*index, tint),
                    }
                })
                .collect()
        })
        .collect()
}

/// The ten standard colours.
pub fn standard_colours() -> Vec<Swatch> {
    [
        ("Dark Red", "C00000"),
        ("Red", "FF0000"),
        ("Orange", "FFC000"),
        ("Yellow", "FFFF00"),
        ("Light Green", "92D050"),
        ("Green", "00B050"),
        ("Light Blue", "00B0F0"),
        ("Blue", "0070C0"),
        ("Dark Blue", "002060"),
        ("Purple", "7030A0"),
    ]
    .into_iter()
    .map(|(name, hex)| Swatch {
        name: name.to_owned(),
        color: ColorSpec::rgb(hex),
    })
    .collect()
}

/// `rgb:RRGGBB` or `theme:INDEX:TINT` (`auto` for the automatic colour), the form a colour takes
/// inside a menu entry id.
pub fn colour_token(c: &ColorSpec) -> String {
    match (&c.rgb, c.theme) {
        (Some(h), _) => format!("rgb:{h}"),
        (None, Some(t)) => format!("theme:{t}:{}", c.tint),
        _ => "auto".to_owned(),
    }
}

pub fn parse_colour_token(s: &str) -> Option<ColorSpec> {
    if s == "auto" {
        return Some(ColorSpec::default());
    }
    if let Some(h) = s.strip_prefix("rgb:") {
        return Some(ColorSpec::rgb(h));
    }
    let rest = s.strip_prefix("theme:")?;
    let (index, tint) = rest.split_once(':')?;
    Some(ColorSpec::theme(index.parse().ok()?, tint.parse().ok()?))
}

/// The screen colour of a palette entry for the workbook's theme.
pub fn resolve_spec(wb: &Workbook, c: &ColorSpec) -> Rgb {
    let source = if let Some(h) = &c.rgb {
        let v = u32::from_str_radix(h, 16).unwrap_or(0);
        ColorSource::Rgb(Rgb {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        })
    } else if let Some(t) = c.theme {
        ColorSource::Theme(t)
    } else {
        ColorSource::Auto
    };
    wb.styles().resolve_color(&Color {
        source,
        tint: c.tint,
    })
}

/// Where a Cell Styles gallery heading puts each built-in style.
pub fn style_groups(names: &[String]) -> Vec<(&'static str, Vec<String>)> {
    const GROUPS: [(&str, &[&str]); 4] = [
        (
            "Good, Bad and Neutral",
            &["Normal", "Bad", "Good", "Neutral"],
        ),
        (
            "Data and Model",
            &[
                "Calculation",
                "Check Cell",
                "Explanatory Text",
                "Input",
                "Linked Cell",
                "Note",
                "Output",
                "Warning Text",
            ],
        ),
        (
            "Titles and Headings",
            &[
                "Heading 1",
                "Heading 2",
                "Heading 3",
                "Heading 4",
                "Title",
                "Total",
            ],
        ),
        (
            "Number Format",
            &["Comma", "Comma [0]", "Currency", "Currency [0]", "Percent"],
        ),
    ];
    let has = |n: &str| names.iter().any(|x| x == n);
    let mut out: Vec<(&'static str, Vec<String>)> = Vec::new();
    let mut used: Vec<&str> = Vec::new();
    for (i, (title, list)) in GROUPS.iter().enumerate() {
        if i == 3 {
            // Themed styles sit between the headings and the number formats.
            let themed: Vec<String> = names
                .iter()
                .filter(|n| n.contains("Accent"))
                .cloned()
                .collect();
            if !themed.is_empty() {
                out.push(("Themed Cell Styles", themed));
            }
        }
        let items: Vec<String> = list
            .iter()
            .filter(|n| has(n))
            .map(|n| (*n).to_owned())
            .collect();
        used.extend(list.iter());
        if !items.is_empty() {
            out.push((title, items));
        }
    }
    let custom: Vec<String> = names
        .iter()
        .filter(|n| !used.contains(&n.as_str()) && !n.contains("Accent"))
        .cloned()
        .collect();
    if !custom.is_empty() {
        out.push(("Custom", custom));
    }
    out
}

fn entry(id: String, label: &str) -> MenuEntry {
    MenuEntry::new(&id, label, true)
}

/// Every control id the formatting code answers for.
pub fn handles(id: &str) -> bool {
    let id = id.split('#').next().unwrap_or(id);
    id.starts_with("fmt.")
        || matches!(
            id,
            "home.clipboard.format-painter" | "home.styles.cell-styles" | "home.editing.clear"
        )
        || id.starts_with("home.font.")
        || id.starts_with("home.alignment.")
        || id.starts_with("home.number.")
}

impl FormatUi {
    pub fn painter_active(&self) -> bool {
        self.painter != Painter::Off
    }

    /// A different document was opened: drop what belonged to the old one, keep the colours.
    pub fn reset_document(&mut self) {
        self.cancel_painter();
    }

    pub fn cancel_painter(&mut self) {
        self.painter = Painter::Off;
        self.painter_pressed = false;
        self.last_painter_click = None;
    }

    /// The painter's source when it is armed.
    fn painter_source(&self) -> Option<(u32, u32, u32, u32)> {
        match self.painter {
            Painter::Off => None,
            Painter::Once(r) | Painter::Locked(r) => Some(r),
        }
    }

    /// Called when the user finished choosing a range with the painter armed: copies the formats.
    pub fn finish_paint(&mut self, sheet: usize, target: (u32, u32, u32, u32)) -> Option<Command> {
        let src = self.painter_source()?;
        self.painter_pressed = false;
        if matches!(self.painter, Painter::Once(_)) {
            self.painter = Painter::Off;
        }
        let range = |r: (u32, u32, u32, u32)| RangeRef {
            first_row: r.0,
            first_col: r.1,
            last_row: r.2,
            last_col: r.3,
        };
        Some(Command::CopyFormat {
            sheet,
            from: range(src),
            to: range(target),
        })
    }

    pub fn state(&self, id: &str, wb: Option<&Workbook>, sel: Sel) -> CommandState {
        let Some(wb) = wb else {
            return CommandState::DISABLED;
        };
        if !handles(id) {
            return CommandState::DISABLED;
        }
        let fmt = wb.cell_format(sel.sheet, sel.active.0, sel.active.1);
        let a = &fmt.alignment;
        match id {
            "home.font.bold" => CommandState::checked(fmt.font.bold),
            "home.font.italic" => CommandState::checked(fmt.font.italic),
            "home.font.underline" => CommandState::checked(fmt.font.underline != Underline::None),
            "home.alignment.align-left" => {
                CommandState::checked(a.horizontal == HorizontalAlignment::Left)
            }
            "home.alignment.centre" => {
                CommandState::checked(a.horizontal == HorizontalAlignment::Center)
            }
            "home.alignment.align-right" => {
                CommandState::checked(a.horizontal == HorizontalAlignment::Right)
            }
            "home.alignment.top-align" => {
                CommandState::checked(a.vertical == VerticalAlignment::Top)
            }
            "home.alignment.middle-align" => {
                CommandState::checked(a.vertical == VerticalAlignment::Center)
            }
            "home.alignment.bottom-align" => {
                CommandState::checked(a.vertical == VerticalAlignment::Bottom)
            }
            "home.alignment.wrap-text" => CommandState::checked(a.wrap_text),
            "home.alignment.decrease-indent" => {
                if a.indent > 0 {
                    CommandState::ENABLED
                } else {
                    CommandState::DISABLED
                }
            }
            "home.clipboard.format-painter" => CommandState::checked(self.painter != Painter::Off),
            // The button is lit while the selection holds a merged cell.
            "home.alignment.merge-centre" => CommandState::checked(touches_merge(wb, sel)),
            "fmt.merge#unmerge" => {
                if touches_merge(wb, sel) {
                    CommandState::ENABLED
                } else {
                    CommandState::DISABLED
                }
            }
            _ => CommandState::ENABLED,
        }
    }

    pub fn value(&self, id: &str, wb: Option<&Workbook>, sel: Sel) -> Option<String> {
        let wb = wb?;
        let fmt = wb.cell_format(sel.sheet, sel.active.0, sel.active.1);
        match id {
            "home.font.font" => Some(fmt.font.name.clone()),
            "home.font.font-size" => Some(size_label(fmt.font.size)),
            "home.number.number-format" => {
                let xf = wb.cell_xf(sel.sheet, sel.active.0, sel.active.1);
                let code = wb.styles().number_format(xf);
                Some(number_category(fmt.num_fmt_id, code).to_owned())
            }
            _ => None,
        }
    }

    /// Text typed into an editable combo and confirmed with Enter: the id of the command to run.
    pub fn combo_commit(&self, id: &str, text: &str) -> Option<String> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        match id {
            "home.font.font" => Some(format!("fmt.font-name#{text}")),
            "home.font.font-size" => text
                .parse::<f64>()
                .ok()
                .filter(|s| (1.0..=409.0).contains(s))
                .map(|s| format!("fmt.font-size#{s}")),
            "home.number.number-format" => Some(
                number_formats()
                    .iter()
                    .find(|(_, l, _)| l.eq_ignore_ascii_case(text))
                    .map_or_else(
                        || format!("fmt.numfmt-code#{text}"),
                        |(k, ..)| format!("fmt.numfmt#{k}"),
                    ),
            ),
            _ => None,
        }
    }

    pub fn menu(&self, id: &str, wb: Option<&Workbook>, sel: Sel) -> Option<Vec<MenuEntry>> {
        let wb = wb?;
        match id {
            "home.alignment.merge-centre" => Some(vec![
                entry("fmt.merge#center".into(), "Merge & Centre"),
                entry("fmt.merge#across".into(), "Merge Across"),
                entry("fmt.merge#cells".into(), "Merge Cells"),
                MenuEntry::new("fmt.merge#unmerge", "Unmerge Cells", touches_merge(wb, sel)),
            ]),
            "home.font.font" => {
                let current = &wb
                    .cell_format(sel.sheet, sel.active.0, sel.active.1)
                    .font
                    .name;
                let mut names: Vec<String> = FONT_NAMES.iter().map(|n| (*n).to_owned()).collect();
                if !names.contains(current) {
                    names.insert(0, current.clone());
                }
                Some(
                    names
                        .iter()
                        .map(|n| {
                            let mut e =
                                entry(format!("fmt.font-name#{n}"), n).checked(n == current);
                            e.height = Some(33);
                            e
                        })
                        .collect(),
                )
            }
            "home.font.font-size" => {
                let current = wb
                    .cell_format(sel.sheet, sel.active.0, sel.active.1)
                    .font
                    .size;
                Some(
                    FONT_SIZES
                        .iter()
                        .map(|s| {
                            let mut e = entry(format!("fmt.font-size#{s}"), &size_label(*s))
                                .checked((*s - current).abs() < 1e-9);
                            e.height = Some(30);
                            e.centred = true;
                            e
                        })
                        .collect(),
                )
            }
            "home.font.underline" => Some(vec![
                entry("fmt.underline#single".into(), "Underline"),
                entry("fmt.underline#double".into(), "Double Underline"),
            ]),
            "home.font.borders" => Some(
                BORDERS
                    .iter()
                    .map(|(k, l)| entry(format!("fmt.border#{k}"), l))
                    .collect(),
            ),
            "home.font.fill-colour" => Some(colour_menu(wb, ColourSlot::Fill, &self.last_fill)),
            "home.font.font-colour" => Some(colour_menu(wb, ColourSlot::Font, &self.last_font)),
            "home.styles.cell-styles" => Some(cell_styles_menu(wb, sel)),
            "home.alignment.orientation" => Some(vec![
                entry("fmt.rotate#45".into(), "Angle Counterclockwise"),
                entry("fmt.rotate#-45".into(), "Angle Clockwise"),
                entry("fmt.rotate#stacked".into(), "Vertical Text"),
                entry("fmt.rotate#90".into(), "Rotate Text Up"),
                entry("fmt.rotate#-90".into(), "Rotate Text Down"),
            ]),
            "home.number.number-format" => {
                let xf = wb.cell_xf(sel.sheet, sel.active.0, sel.active.1);
                let fmt = wb.cell_format(sel.sheet, sel.active.0, sel.active.1);
                let current = number_category(fmt.num_fmt_id, wb.styles().number_format(xf));
                let mut out: Vec<MenuEntry> = number_formats()
                    .iter()
                    .map(|(k, l, _)| {
                        let mut e = entry(format!("fmt.numfmt#{k}"), l)
                            .detail(number_sample(k))
                            .checked(*l == current);
                        e.height = Some(60);
                        e
                    })
                    .collect();
                out.push(MenuEntry::separator());
                out.push(entry("fmt.numfmt#more".into(), "More Number Formats..."));
                Some(out)
            }
            "home.number.accounting-number-format" => Some(
                ACCOUNTING
                    .iter()
                    .map(|(k, l, _)| entry(format!("fmt.accounting#{k}"), l))
                    .collect(),
            ),
            "home.editing.clear" => Some(vec![entry("fmt.clear-formats".into(), "Clear Formats")]),
            _ => None,
        }
    }

    /// Runs a control or menu entry. `now` is a clock in seconds (for the double click).
    pub fn run(&mut self, id: &str, wb: Option<&Workbook>, sel: Sel, now: f64) -> Action {
        let Some(wb) = wb else {
            return Action::None;
        };
        let fmt = wb
            .cell_format(sel.sheet, sel.active.0, sel.active.1)
            .clone();
        let (key, arg) = match id.split_once('#') {
            Some((k, a)) => (k, Some(a)),
            None => (id, None),
        };
        let apply = |c: Command| Action::Apply(vec![c]);
        match key {
            "home.clipboard.format-painter" => {
                self.format_painter_click(sel, now);
                Action::None
            }
            "home.font.bold" => apply(format_cells(
                sel,
                font_change(FontChange {
                    bold: Some(!fmt.font.bold),
                    ..FontChange::default()
                }),
            )),
            "home.font.italic" => apply(format_cells(
                sel,
                font_change(FontChange {
                    italic: Some(!fmt.font.italic),
                    ..FontChange::default()
                }),
            )),
            "home.font.underline" => {
                let to = if fmt.font.underline == Underline::None {
                    UnderlineKind::Single
                } else {
                    UnderlineKind::None
                };
                apply(underline(sel, to))
            }
            "fmt.underline" => {
                let want = if arg == Some("double") {
                    (Underline::Double, UnderlineKind::Double)
                } else {
                    (Underline::Single, UnderlineKind::Single)
                };
                let to = if fmt.font.underline == want.0 {
                    UnderlineKind::None
                } else {
                    want.1
                };
                apply(underline(sel, to))
            }
            "fmt.strikethrough" => apply(format_cells(
                sel,
                font_change(FontChange {
                    strike: Some(!fmt.font.strike),
                    ..FontChange::default()
                }),
            )),
            "fmt.font-name" => arg.map_or(Action::None, |n| {
                apply(format_cells(
                    sel,
                    font_change(FontChange {
                        name: Some(n.to_owned()),
                        ..FontChange::default()
                    }),
                ))
            }),
            "fmt.font-size" => arg
                .and_then(|s| s.parse::<f64>().ok())
                .map_or(Action::None, |size| apply(font_size(sel, size))),
            "home.font.increase-font-size" => apply(font_size(sel, next_font_size(fmt.font.size))),
            "home.font.decrease-font-size" => {
                apply(font_size(sel, previous_font_size(fmt.font.size)))
            }
            "home.font.borders" => self.border(sel, self.last_border),
            "fmt.border" => {
                let Some(which) = arg.and_then(|a| BORDERS.iter().find(|(k, _)| *k == a)) else {
                    return Action::None;
                };
                self.last_border = which.0;
                self.border(sel, which.0)
            }
            "fmt.outline-border" => self.border(sel, "outside"),
            "home.font.fill-colour" => apply(fill(sel, Some(self.last_fill.clone()))),
            "home.font.font-colour" => apply(font_colour(sel, self.last_font.clone())),
            "fmt.fill" => match arg {
                Some("none") => apply(fill(sel, None)),
                Some("more") => Action::None,
                Some(t) => parse_colour_token(t).map_or(Action::None, |c| {
                    self.last_fill = c.clone();
                    apply(fill(sel, Some(c)))
                }),
                None => Action::None,
            },
            "fmt.font-colour" => match arg {
                Some("more") => Action::None,
                Some(t) => parse_colour_token(t).map_or(Action::None, |c| {
                    self.last_font = c.clone();
                    apply(font_colour(sel, c))
                }),
                None => Action::None,
            },
            "home.alignment.align-left" => apply(h_toggle(sel, &fmt, HorizontalAlignment::Left)),
            "home.alignment.centre" => apply(h_toggle(sel, &fmt, HorizontalAlignment::Center)),
            "home.alignment.align-right" => apply(h_toggle(sel, &fmt, HorizontalAlignment::Right)),
            "home.alignment.top-align" => apply(v_toggle(sel, &fmt, VerticalAlignment::Top)),
            "home.alignment.middle-align" => apply(v_toggle(sel, &fmt, VerticalAlignment::Center)),
            "home.alignment.bottom-align" => apply(format_cells(
                sel,
                align_change(AlignmentChange {
                    vertical: Some(VAlign::Bottom),
                    ..AlignmentChange::default()
                }),
            )),
            // The button toggles: a merged selection splits, anything else merges and centres.
            "home.alignment.merge-centre" => merge_action(
                wb,
                sel,
                if touches_merge(wb, sel) {
                    MergeMode::Unmerge
                } else {
                    MergeMode::MergeCenter
                },
            ),
            "fmt.merge" => match arg {
                Some("center") => merge_action(wb, sel, MergeMode::MergeCenter),
                Some("across") => merge_action(wb, sel, MergeMode::Across),
                Some("cells") => merge_action(wb, sel, MergeMode::Cells),
                Some("unmerge") => merge_action(wb, sel, MergeMode::Unmerge),
                _ => Action::None,
            },
            "home.alignment.wrap-text" => apply(format_cells(
                sel,
                align_change(AlignmentChange {
                    wrap_text: Some(!fmt.alignment.wrap_text),
                    ..AlignmentChange::default()
                }),
            )),
            "home.alignment.increase-indent" => apply(indent(sel, fmt.alignment.indent + 1)),
            "home.alignment.decrease-indent" => {
                if fmt.alignment.indent == 0 {
                    Action::None
                } else {
                    apply(indent(sel, fmt.alignment.indent - 1))
                }
            }
            "fmt.rotate" => {
                let want = match arg {
                    Some("stacked") => Some(RotationSpec::Stacked),
                    Some(d) => d.parse::<i32>().ok().map(RotationSpec::Degrees),
                    None => None,
                };
                let Some(want) = want else {
                    return Action::None;
                };
                let current = match fmt.alignment.rotation_degrees() {
                    Some(d) => RotationSpec::Degrees(d),
                    None => RotationSpec::Stacked,
                };
                // Choosing the angle a cell already has turns the text horizontal again.
                let to = if current == want {
                    RotationSpec::Degrees(0)
                } else {
                    want
                };
                apply(format_cells(
                    sel,
                    align_change(AlignmentChange {
                        rotation: Some(to),
                        ..AlignmentChange::default()
                    }),
                ))
            }
            "fmt.numfmt" => number_formats()
                .into_iter()
                .find(|(k, ..)| Some(*k) == arg)
                .map_or(Action::None, |(_, _, spec)| {
                    apply(format_cells(sel, number_change(spec)))
                }),
            "fmt.numfmt-code" => {
                arg.filter(|c| !c.trim().is_empty())
                    .map_or(Action::None, |code| {
                        apply(format_cells(
                            sel,
                            number_change(NumberFormatSpec::Code(code.to_owned())),
                        ))
                    })
            }
            "fmt.accounting" => ACCOUNTING.iter().find(|(k, ..)| Some(*k) == arg).map_or(
                Action::None,
                |(_, _, symbol)| {
                    apply(format_cells(
                        sel,
                        number_change(NumberFormatSpec::Code(accounting_code(symbol))),
                    ))
                },
            ),
            "fmt.number-shortcut" => arg.map_or(Action::None, |a| shortcut_action(sel, a)),
            "home.number.accounting-number-format" => apply(style(sel, "Currency")),
            "home.number.percent-style" => apply(style(sel, "Percent")),
            "home.number.comma-style" => apply(style(sel, "Comma")),
            "home.number.increase-decimal" => apply(decimals(sel, 1)),
            "home.number.decrease-decimal" => apply(decimals(sel, -1)),
            "fmt.cell-style" => arg.map_or(Action::None, |n| apply(style(sel, n))),
            "home.styles.cell-styles" => arg.map_or(Action::None, |n| apply(style(sel, n))),
            "home.editing.clear" => Action::None,
            "fmt.clear-formats" => apply(Command::ClearFormats {
                sheet: sel.sheet,
                range: sel.range_ref(),
            }),
            _ => Action::None,
        }
    }

    fn border(&self, sel: Sel, which: &str) -> Action {
        border_change(which).map_or(Action::None, |b| {
            Action::Apply(vec![Command::SetBorders {
                sheet: sel.sheet,
                range: sel.range_ref(),
                borders: Box::new(b),
            }])
        })
    }

    /// A click on Format Painter: arms it, locks it on a second quick click, or switches it off.
    fn format_painter_click(&mut self, sel: Sel, now: f64) {
        let quick = self
            .last_painter_click
            .is_some_and(|t| now - t <= DOUBLE_CLICK);
        match self.painter {
            Painter::Off => {
                self.painter = Painter::Once(sel.range);
                self.painter_pressed = false;
                self.last_painter_click = Some(now);
            }
            Painter::Once(src) if quick => {
                self.painter = Painter::Locked(src);
                self.last_painter_click = None;
            }
            Painter::Once(_) | Painter::Locked(_) => self.cancel_painter(),
        }
    }
}

/// A sample value under each Number Format list entry, as the list shows them.
fn number_sample(key: &str) -> &'static str {
    match key {
        "general" => "No specific format",
        "number" => "1.00",
        "currency" => "$1.00",
        "accounting" => " $ 1.00 ",
        "short-date" => "01/01/1900",
        "long-date" => "Sunday, 01 January 1900",
        "time" => "00:00:00",
        "percentage" => "100.00%",
        "fraction" => "1",
        "scientific" => "1.00E+00",
        _ => "",
    }
}

fn swatch_cell(
    wb: &Workbook,
    s: &Swatch,
    prefix: &str,
    current: &ColorSpec,
) -> open365_ui_shell::Swatch {
    let rgb = resolve_spec(wb, &s.color);
    let mut cell = open365_ui_shell::Swatch::new(
        &format!("{prefix}#{}", colour_token(&s.color)),
        &s.name,
        [rgb.r, rgb.g, rgb.b],
    );
    cell.checked = colour_token(&s.color) == colour_token(current);
    cell
}

/// The colour picker as Office lays it out: Automatic / No Fill, theme colours with their tints,
/// standard colours, More Colours.
fn colour_menu(wb: &Workbook, slot: ColourSlot, current: &ColorSpec) -> Vec<MenuEntry> {
    let (prefix, none, none_id) = match slot {
        ColourSlot::Fill => ("fmt.fill", "No Fill", "none"),
        ColourSlot::Font => ("fmt.font-colour", "Automatic", "auto"),
    };
    let mut palette = theme_palette().into_iter();
    let base: Vec<_> = palette
        .next()
        .unwrap_or_default()
        .iter()
        .map(|s| swatch_cell(wb, s, prefix, current))
        .collect();
    let mut tint_rows: Vec<Vec<_>> = palette
        .map(|row| {
            row.iter()
                .map(|s| swatch_cell(wb, s, prefix, current))
                .collect()
        })
        .collect();
    tint_rows.truncate(5);
    let standard: Vec<_> = standard_colours()
        .iter()
        .map(|s| swatch_cell(wb, s, prefix, current))
        .collect();
    let mut none_entry = entry(format!("{prefix}#{none_id}"), none);
    none_entry.height = Some(34);
    let mut out = vec![MenuEntry::filter("High-contrast only")];
    if slot == ColourSlot::Font {
        out.push(none_entry.clone());
        out.push(MenuEntry::separator());
    }
    out.push(MenuEntry::heading("Theme Colours"));
    out.push(MenuEntry::swatches(vec![base], false));
    out.push(MenuEntry::separator());
    out.push(MenuEntry::swatches(tint_rows, true));
    out.push(MenuEntry::separator());
    out.push(MenuEntry::heading("Standard Colours"));
    out.push(MenuEntry::swatches(vec![standard], false));
    out.push(MenuEntry::separator());
    if slot == ColourSlot::Fill {
        out.push(none_entry);
        out.push(MenuEntry::separator());
    }
    out.push(entry(format!("{prefix}#more"), "More Colours..."));
    out
}

/// Gives a gallery tile the look of a cell style: its real fill (white where the style has none, as
/// the grid's cells are), font colour and size, and the four border edges.
fn style_tile(wb: &Workbook, f: &open365_xlsx::CellFormat, tile: &mut open365_ui_shell::Tile) {
    use open365_xlsx::{BorderStyle as B, ColorSource, FillPattern};
    let rgb = |c: &open365_xlsx::Color| {
        let v = wb.styles().resolve_color(c);
        [v.r, v.g, v.b]
    };
    let solid = f.fill.pattern != FillPattern::None;
    tile.fill = Some(if solid { rgb(&f.fill.fg) } else { [255; 3] });
    tile.ink = Some(if f.font.color.source == ColorSource::Auto {
        [0; 3]
    } else {
        rgb(&f.font.color)
    });
    let edge = |e: &open365_xlsx::BorderEdge| {
        let w = match e.style {
            B::None => return None,
            B::Medium | B::Dashed => 2,
            B::Thick | B::Double => 3,
            _ => 1,
        };
        let c = if e.color.source == ColorSource::Auto {
            [0; 3]
        } else {
            rgb(&e.color)
        };
        Some((c, w))
    };
    tile.borders = [
        edge(&f.border.left),
        edge(&f.border.right),
        edge(&f.border.top),
        edge(&f.border.bottom),
    ];
    tile.text_pct = (f.font.size / 11.0 * 100.0).round() as u32;
    tile.bold = f.font.bold;
    tile.italic = f.font.italic;
}

/// The Cell Styles gallery: one heading and a grid of 133 x 36 u tiles per group.
fn cell_styles_menu(wb: &Workbook, sel: Sel) -> Vec<MenuEntry> {
    let names = wb.styles().cell_style_names();
    let current = wb
        .styles()
        .cell_style_name(wb.cell_xf(sel.sheet, sel.active.0, sel.active.1));
    let mut out = Vec::new();
    for (i, (title, styles)) in style_groups(&names).into_iter().enumerate() {
        if i > 0 {
            out.push(MenuEntry::separator());
        }
        out.push(MenuEntry::heading(title));
        let previews = wb.styles().cell_style_previews(&styles);
        let tiles = previews
            .into_iter()
            .map(|(n, format)| {
                let mut tile = open365_ui_shell::Tile::plain(
                    &format!("home.styles.cell-styles#{n}"),
                    &n,
                    Some(&n) == current.as_ref(),
                );
                if let Some(f) = format {
                    style_tile(wb, &f, &mut tile);
                }
                tile
            })
            .collect();
        out.push(MenuEntry::tiles(6, (133, 36), tiles));
    }
    out
}

fn underline(sel: Sel, kind: UnderlineKind) -> Command {
    format_cells(
        sel,
        font_change(FontChange {
            underline: Some(kind),
            ..FontChange::default()
        }),
    )
}

fn font_size(sel: Sel, size: f64) -> Command {
    format_cells(
        sel,
        font_change(FontChange {
            size: Some(size),
            ..FontChange::default()
        }),
    )
}

fn font_colour(sel: Sel, c: ColorSpec) -> Command {
    format_cells(
        sel,
        font_change(FontChange {
            color: Some(c),
            ..FontChange::default()
        }),
    )
}

fn fill(sel: Sel, c: Option<ColorSpec>) -> Command {
    format_cells(
        sel,
        CellFormatChange {
            fill: Some(match c {
                Some(color) => FillChange::Solid { color },
                None => FillChange::None,
            }),
            ..CellFormatChange::default()
        },
    )
}

/// Align Left, Centre and Align Right switch back to General when the cell already has them.
fn h_toggle(sel: Sel, fmt: &CellFormat, to: HorizontalAlignment) -> Command {
    let h = if fmt.alignment.horizontal == to {
        HorizontalAlignment::General
    } else {
        to
    };
    format_cells(
        sel,
        align_change(AlignmentChange {
            horizontal: Some(horizontal(h)),
            ..AlignmentChange::default()
        }),
    )
}

/// Top and Middle Align fall back to the default, bottom, when the cell already has them.
fn v_toggle(sel: Sel, fmt: &CellFormat, to: VerticalAlignment) -> Command {
    let v = if fmt.alignment.vertical == to {
        VAlign::Bottom
    } else if to == VerticalAlignment::Top {
        VAlign::Top
    } else {
        VAlign::Center
    };
    format_cells(
        sel,
        align_change(AlignmentChange {
            vertical: Some(v),
            ..AlignmentChange::default()
        }),
    )
}

fn indent(sel: Sel, n: u32) -> Command {
    format_cells(
        sel,
        align_change(AlignmentChange {
            indent: Some(n),
            ..AlignmentChange::default()
        }),
    )
}

fn style(sel: Sel, name: &str) -> Command {
    Command::ApplyCellStyle {
        sheet: sel.sheet,
        range: sel.range_ref(),
        style: name.to_owned(),
    }
}

fn decimals(sel: Sel, delta: i32) -> Command {
    Command::ChangeDecimals {
        sheet: sel.sheet,
        range: sel.range_ref(),
        delta,
    }
}

/// The Ctrl+Shift+symbol number format shortcuts. `which` names the symbol.
fn shortcut_action(sel: Sel, which: &str) -> Action {
    let spec = match which {
        "general" => NumberFormatSpec::Builtin(0),
        "number" => NumberFormatSpec::Builtin(4),
        "time" => NumberFormatSpec::Builtin(18),
        "date" => NumberFormatSpec::Builtin(15),
        "currency" => NumberFormatSpec::Builtin(7),
        "percent" => NumberFormatSpec::Builtin(9),
        "scientific" => NumberFormatSpec::Builtin(11),
        "outline" => {
            return Action::Apply(vec![Command::SetBorders {
                sheet: sel.sheet,
                range: sel.range_ref(),
                borders: Box::new(border_change("outside").unwrap_or_default()),
            }]);
        }
        "no-outline" => {
            let mut b = BorderChange::default();
            outline(&mut b, "none");
            return Action::Apply(vec![Command::SetBorders {
                sheet: sel.sheet,
                range: sel.range_ref(),
                borders: Box::new(b),
            }]);
        }
        _ => return Action::None,
    };
    Action::Apply(vec![format_cells(sel, number_change(spec))])
}

/// The key combination of a Ctrl+Shift number-format shortcut, from the key as the keyboard
/// layout reports it and the key's position.
pub fn number_shortcut(key: egui::Key, physical: Option<egui::Key>) -> Option<&'static str> {
    use egui::Key;
    let k = physical.unwrap_or(key);
    Some(match k {
        Key::Backtick => "general",
        Key::Num1 | Key::Exclamationmark => "number",
        Key::Num2 => "time",
        Key::Num3 => "date",
        Key::Num4 => "currency",
        Key::Num5 => "percent",
        Key::Num6 => "scientific",
        Key::Num7 => "outline",
        Key::Minus => "no-outline",
        _ => return None,
    })
}

/// The control id a Ctrl shortcut runs: Ctrl+B, I, U and 5, and Ctrl+Shift with a number-format
/// symbol (`~ ! @ # $ % ^ & _`).
pub fn shortcut_id(
    key: egui::Key,
    physical: Option<egui::Key>,
    m: egui::Modifiers,
) -> Option<String> {
    use egui::Key;
    if !m.command || m.alt {
        return None;
    }
    if m.shift {
        return number_shortcut(key, physical).map(|k| format!("fmt.number-shortcut#{k}"));
    }
    Some(
        match key {
            Key::B => "home.font.bold",
            Key::I => "home.font.italic",
            Key::U => "home.font.underline",
            Key::Num5 => "fmt.strikethrough",
            _ => return None,
        }
        .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_size_steps_follow_the_size_list() {
        assert_eq!(next_font_size(11.0), 12.0);
        assert_eq!(next_font_size(12.0), 14.0);
        assert_eq!(next_font_size(10.5), 11.0);
        assert_eq!(previous_font_size(11.0), 10.0);
        assert_eq!(previous_font_size(10.5), 10.0);
        assert_eq!(previous_font_size(8.0), 7.0);
        assert_eq!(previous_font_size(1.0), 1.0);
        assert_eq!(next_font_size(72.0), 80.0);
        assert_eq!(next_font_size(408.0), 409.0);
        assert_eq!(previous_font_size(80.0), 72.0);
    }

    #[test]
    fn categories_name_the_common_formats() {
        assert_eq!(number_category(0, "General"), "General");
        assert_eq!(number_category(2, "0.00"), "Number");
        assert_eq!(number_category(10, "0.00%"), "Percentage");
        assert_eq!(number_category(14, "m/d/yyyy"), "Short Date");
        assert_eq!(number_category(49, "@"), "Text");
        assert_eq!(number_category(164, "0.000"), "Number");
        assert_eq!(number_category(165, "0.0%"), "Percentage");
        assert_eq!(number_category(166, "yyyy-mm-dd"), "Date");
        assert_eq!(number_category(167, "hh:mm"), "Time");
        assert_eq!(number_category(168, "\"x\"0 \"y\""), "Custom");
    }

    #[test]
    fn colour_tokens_round_trip() {
        for c in [
            ColorSpec::rgb("FFC000"),
            ColorSpec::theme(4, 0.8),
            ColorSpec::theme(1, -0.25),
            ColorSpec::default(),
        ] {
            assert_eq!(parse_colour_token(&colour_token(&c)), Some(c));
        }
    }

    #[test]
    fn palette_has_the_pickers_shape() {
        let p = theme_palette();
        assert_eq!((p.len(), p[0].len()), (6, 10));
        assert_eq!(p[0][0].color, ColorSpec::theme(0, 0.0));
        assert_eq!(standard_colours().len(), 10);
    }

    #[test]
    fn style_groups_cover_every_name_once() {
        let names: Vec<String> = [
            "Normal",
            "Good",
            "Heading 1",
            "20% - Accent1",
            "Percent",
            "Mine",
        ]
        .map(String::from)
        .to_vec();
        let groups = style_groups(&names);
        let total: usize = groups.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total, names.len());
        assert!(groups.iter().any(|(t, _)| *t == "Custom"));
    }

    #[test]
    fn number_shortcuts_use_the_key_position() {
        use egui::Key;
        assert_eq!(
            number_shortcut(Key::Exclamationmark, Some(Key::Num1)),
            Some("number")
        );
        assert_eq!(number_shortcut(Key::Num1, None), Some("number"));
        assert_eq!(number_shortcut(Key::A, Some(Key::A)), None);
        assert_eq!(
            number_shortcut(Key::Minus, Some(Key::Minus)),
            Some("no-outline")
        );
    }
}
