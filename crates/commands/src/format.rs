//! Cell-formatting commands: the serialisable mirror of the xlsx crate's formatting types, so a
//! command can be logged, replayed and sent over the agent interface.

use open365_xlsx::{
    AlignmentPatch, BorderEdge, BorderPatch, BorderSpec, BorderStyle, Color, ColorSource,
    FillPattern, FillSpec, FontPatch, FormatPatch, FormatSnapshot, HorizontalAlignment,
    NumberFormat, ProtectionPatch, Rgb, Rotation, Underline, VerticalAlignment, Workbook,
};
use serde::{Deserialize, Serialize};

use crate::CommandError;
use crate::command::Command;

/// An inclusive cell rectangle, 1-based. A range spanning every column formats whole rows and
/// one spanning every row formats whole columns, as in Excel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeRef {
    pub first_row: u32,
    pub first_col: u32,
    pub last_row: u32,
    pub last_col: u32,
}

impl RangeRef {
    pub fn cell(row: u32, col: u32) -> Self {
        Self {
            first_row: row,
            first_col: col,
            last_row: row,
            last_col: col,
        }
    }

    pub(crate) fn rect(self) -> (u32, u32, u32, u32) {
        (self.first_row, self.first_col, self.last_row, self.last_col)
    }
}

/// A colour: `rgb` is `RRGGBB`, `theme` an OOXML theme index (0 = light 1, 1 = dark 1, 2 = light 2,
/// 3 = dark 2, 4 to 9 = accent 1 to 6, 10 = hyperlink, 11 = followed hyperlink), `indexed` a
/// legacy palette index. With none of them the colour is automatic.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ColorSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexed: Option<u32>,
    /// Lighter (positive) or darker (negative), -1.0 to 1.0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tint: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl ColorSpec {
    pub fn rgb(hex: &str) -> Self {
        Self {
            rgb: Some(hex.to_owned()),
            ..Self::default()
        }
    }

    pub fn theme(index: u32, tint: f64) -> Self {
        Self {
            theme: Some(index),
            tint,
            ..Self::default()
        }
    }

    fn to_color(&self) -> Result<Color, CommandError> {
        let source = if let Some(h) = &self.rgb {
            let v = (h.len() == 6)
                .then(|| u32::from_str_radix(h, 16).ok())
                .flatten()
                .ok_or_else(|| CommandError::Format(format!("{h:?} is not an RRGGBB colour")))?;
            ColorSource::Rgb(Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
        } else if let Some(t) = self.theme {
            ColorSource::Theme(t)
        } else if let Some(i) = self.indexed {
            ColorSource::Indexed(i)
        } else {
            ColorSource::Auto
        };
        Ok(Color {
            source,
            tint: self.tint,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnderlineKind {
    None,
    Single,
    Double,
    SingleAccounting,
    DoubleAccounting,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FontChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<UnderlineKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorSpec>,
}

/// A fill; `pattern` is the OOXML `patternType` name (`solid`, `darkGray`, `lightGrid`, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FillChange {
    None,
    Solid {
        color: ColorSpec,
    },
    Pattern {
        pattern: String,
        #[serde(default)]
        fg: Option<ColorSpec>,
        #[serde(default)]
        bg: Option<ColorSpec>,
    },
}

/// A border line: the OOXML style name (`thin`, `medium`, `dashed`, `dotted`, `thick`, `double`,
/// `hair`, `mediumDashed`, `dashDot`, `mediumDashDot`, `dashDotDot`, `mediumDashDotDot`,
/// `slantDashDot`) or `none`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineSpec {
    pub style: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BorderChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<LineSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<LineSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<LineSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<LineSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside_horizontal: Option<LineSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside_vertical: Option<LineSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagonal_down: Option<LineSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagonal_up: Option<LineSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HAlign {
    General,
    Left,
    Center,
    Right,
    Fill,
    Justify,
    CenterContinuous,
    Distributed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VAlign {
    Top,
    Center,
    Bottom,
    Justify,
    Distributed,
}

/// Text rotation: degrees from -90 to 90 (positive is counter-clockwise), or stacked letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "degrees", rename_all = "snake_case")]
pub enum RotationSpec {
    Degrees(i32),
    Stacked,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlignmentChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizontal: Option<HAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<VAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_text: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<RotationSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shrink_to_fit: Option<bool>,
}

/// A number format: a built-in id (0 to 49) or a format code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum NumberFormatSpec {
    Builtin(u32),
    Code(String),
}

/// Every property a formatting action can set; absent ones are left alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CellFormatChange {
    #[serde(default)]
    pub font: FontChange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<FillChange>,
    #[serde(default)]
    pub alignment: AlignmentChange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_format: Option<NumberFormatSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    /// Hide the formula in a protected sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula_hidden: Option<bool>,
}

fn border_style(name: &str) -> Result<BorderStyle, CommandError> {
    Ok(match name {
        "none" => BorderStyle::None,
        "thin" => BorderStyle::Thin,
        "medium" => BorderStyle::Medium,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "thick" => BorderStyle::Thick,
        "double" => BorderStyle::Double,
        "hair" => BorderStyle::Hair,
        "mediumDashed" => BorderStyle::MediumDashed,
        "dashDot" => BorderStyle::DashDot,
        "mediumDashDot" => BorderStyle::MediumDashDot,
        "dashDotDot" => BorderStyle::DashDotDot,
        "mediumDashDotDot" => BorderStyle::MediumDashDotDot,
        "slantDashDot" => BorderStyle::SlantDashDot,
        other => {
            return Err(CommandError::Format(format!(
                "unknown border style {other:?}"
            )));
        }
    })
}

fn fill_pattern(name: &str) -> Result<FillPattern, CommandError> {
    Ok(match name {
        "none" => FillPattern::None,
        "solid" => FillPattern::Solid,
        "mediumGray" => FillPattern::MediumGray,
        "darkGray" => FillPattern::DarkGray,
        "lightGray" => FillPattern::LightGray,
        "darkHorizontal" => FillPattern::DarkHorizontal,
        "darkVertical" => FillPattern::DarkVertical,
        "darkDown" => FillPattern::DarkDown,
        "darkUp" => FillPattern::DarkUp,
        "darkGrid" => FillPattern::DarkGrid,
        "darkTrellis" => FillPattern::DarkTrellis,
        "lightHorizontal" => FillPattern::LightHorizontal,
        "lightVertical" => FillPattern::LightVertical,
        "lightDown" => FillPattern::LightDown,
        "lightUp" => FillPattern::LightUp,
        "lightGrid" => FillPattern::LightGrid,
        "lightTrellis" => FillPattern::LightTrellis,
        "gray125" => FillPattern::Gray125,
        "gray0625" => FillPattern::Gray0625,
        other => {
            return Err(CommandError::Format(format!(
                "unknown fill pattern {other:?}"
            )));
        }
    })
}

fn color_opt(c: Option<&ColorSpec>) -> Result<Option<Color>, CommandError> {
    c.map(ColorSpec::to_color).transpose()
}

fn edge(line: Option<&LineSpec>) -> Result<Option<BorderEdge>, CommandError> {
    line.map(|l| {
        Ok(BorderEdge {
            style: border_style(&l.style)?,
            color: color_opt(l.color.as_ref())?.unwrap_or(Color::AUTO),
        })
    })
    .transpose()
}

impl BorderChange {
    pub(crate) fn to_spec(&self) -> Result<BorderSpec, CommandError> {
        Ok(BorderSpec {
            left: edge(self.left.as_ref())?,
            right: edge(self.right.as_ref())?,
            top: edge(self.top.as_ref())?,
            bottom: edge(self.bottom.as_ref())?,
            inside_horizontal: edge(self.inside_horizontal.as_ref())?,
            inside_vertical: edge(self.inside_vertical.as_ref())?,
            diagonal_down: edge(self.diagonal_down.as_ref())?,
            diagonal_up: edge(self.diagonal_up.as_ref())?,
        })
    }
}

impl CellFormatChange {
    pub(crate) fn to_patch(&self) -> Result<FormatPatch, CommandError> {
        let f = &self.font;
        let font = FontPatch {
            name: f.name.clone(),
            size: f.size,
            bold: f.bold,
            italic: f.italic,
            underline: f.underline.map(|u| match u {
                UnderlineKind::None => Underline::None,
                UnderlineKind::Single => Underline::Single,
                UnderlineKind::Double => Underline::Double,
                UnderlineKind::SingleAccounting => Underline::SingleAccounting,
                UnderlineKind::DoubleAccounting => Underline::DoubleAccounting,
            }),
            strike: f.strike,
            color: color_opt(f.color.as_ref())?,
        };
        let fill = match &self.fill {
            None => None,
            Some(FillChange::None) => Some(FillSpec::None),
            Some(FillChange::Solid { color }) => Some(FillSpec::Solid(color.to_color()?)),
            Some(FillChange::Pattern { pattern, fg, bg }) => Some(FillSpec::Pattern {
                pattern: fill_pattern(pattern)?,
                fg: color_opt(fg.as_ref())?,
                bg: color_opt(bg.as_ref())?,
            }),
        };
        let a = &self.alignment;
        let alignment = AlignmentPatch {
            horizontal: a.horizontal.map(|h| match h {
                HAlign::General => HorizontalAlignment::General,
                HAlign::Left => HorizontalAlignment::Left,
                HAlign::Center => HorizontalAlignment::Center,
                HAlign::Right => HorizontalAlignment::Right,
                HAlign::Fill => HorizontalAlignment::Fill,
                HAlign::Justify => HorizontalAlignment::Justify,
                HAlign::CenterContinuous => HorizontalAlignment::CenterContinuous,
                HAlign::Distributed => HorizontalAlignment::Distributed,
            }),
            vertical: a.vertical.map(|v| match v {
                VAlign::Top => VerticalAlignment::Top,
                VAlign::Center => VerticalAlignment::Center,
                VAlign::Bottom => VerticalAlignment::Bottom,
                VAlign::Justify => VerticalAlignment::Justify,
                VAlign::Distributed => VerticalAlignment::Distributed,
            }),
            wrap_text: a.wrap_text,
            indent: a.indent,
            rotation: a.rotation.map(|r| match r {
                RotationSpec::Degrees(d) => Rotation::Degrees(d),
                RotationSpec::Stacked => Rotation::Stacked,
            }),
            shrink_to_fit: a.shrink_to_fit,
        };
        Ok(FormatPatch {
            font,
            fill,
            border: BorderPatch::default(),
            alignment,
            number_format: self.number_format.as_ref().map(|n| match n {
                NumberFormatSpec::Builtin(id) => NumberFormat::Builtin(*id),
                NumberFormatSpec::Code(c) => NumberFormat::Code(c.clone()),
            }),
            protection: ProtectionPatch {
                locked: self.locked,
                hidden: self.formula_hidden,
            },
        })
    }
}

/// True for the cell-formatting commands.
pub(crate) fn is_format_command(command: &Command) -> bool {
    matches!(
        command,
        Command::FormatCells { .. }
            | Command::SetBorders { .. }
            | Command::ApplyCellStyle { .. }
            | Command::ClearFormats { .. }
            | Command::CopyFormat { .. }
            | Command::ChangeDecimals { .. }
    )
}

/// Runs a formatting command; gives the state to put back on undo, `None` when nothing changed.
pub(crate) fn run(
    wb: &mut Workbook,
    command: &Command,
) -> Result<Option<FormatSnapshot>, CommandError> {
    Ok(match command {
        Command::FormatCells {
            sheet,
            range,
            change,
        } => Some(wb.format_range(*sheet, range.rect(), &change.to_patch()?)?),
        Command::SetBorders {
            sheet,
            range,
            borders,
        } => Some(wb.set_borders(*sheet, range.rect(), &borders.to_spec()?)?),
        Command::ApplyCellStyle {
            sheet,
            range,
            style,
        } => Some(wb.apply_cell_style(*sheet, range.rect(), style)?),
        Command::ClearFormats { sheet, range } => Some(wb.clear_formats(*sheet, range.rect())?),
        Command::CopyFormat { sheet, from, to } => {
            Some(wb.copy_formats(*sheet, from.rect(), to.rect())?)
        }
        Command::ChangeDecimals {
            sheet,
            range,
            delta,
        } => wb.change_decimals(*sheet, range.rect(), *delta)?,
        _ => None,
    })
}
