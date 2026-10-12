//! Home tab operations of the sheets model: clipboard, font, alignment, number formats, styles,
//! cells and editing. Each one is a command of the command layer (so it is one undo step) built
//! from the selection; the ribbon module `ribbon::sheets::home` only maps control ids to these.
//!
//! The toggles read the active cell, as Excel does: Bold on a bold active cell clears bold from the
//! whole selection, otherwise sets it.

use std::collections::HashSet;

use open365_commands::{
    AlignmentChange, BorderChange, CellFormatChange, ColorSpec, Command, FillChange, FontChange,
    HAlign, HomeOp, LineSpec, MergeMode, NumberFormatSpec, RangeRef, RotationSpec, SortLevel,
    UnderlineKind, VAlign,
};
use open365_xlsx::{CellValue, HorizontalAlignment, Underline, VerticalAlignment};

use super::super::{MAX_COL, MAX_ROW, Paste, PasteKind, Rect4, SheetModel};

/// State the Home tab keeps between clicks (none of it is in the undo history or the file).
#[derive(Debug, Clone)]
pub struct HomeState {
    /// Format Painter: the sheet and range whose formats were picked up.
    pub painter: Option<(usize, Rect4)>,
    /// The border the Borders button applies (the last one chosen).
    pub border: String,
    /// The colour the Fill Colour button applies (`None` is No Fill).
    pub fill: Option<String>,
    /// The colour the Font Colour button applies (`None` is Automatic).
    pub font_colour: Option<String>,
}

impl Default for HomeState {
    fn default() -> Self {
        Self {
            painter: None,
            border: "bottom".to_owned(),
            fill: Some("FFFF00".to_owned()),
            font_colour: Some("FF0000".to_owned()),
        }
    }
}

/// The entries of the Number Format list, in Excel's order.
pub const NUMBER_FORMAT_NAMES: &[&str] = &[
    "General",
    "Number",
    "Currency",
    "Accounting",
    "Short Date",
    "Long Date",
    "Time",
    "Percentage",
    "Fraction",
    "Scientific",
    "Text",
];

/// The font sizes Increase and Decrease Font Size step through (Excel's list).
const SIZE_STEPS: &[f64] = &[
    8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0,
];

fn rr(r: Rect4) -> RangeRef {
    RangeRef {
        first_row: r.0,
        first_col: r.1,
        last_row: r.2,
        last_col: r.3,
    }
}

fn line(style: &str) -> Option<LineSpec> {
    Some(LineSpec {
        style: style.to_owned(),
        color: None,
    })
}

impl SheetModel {
    fn sel(&self) -> RangeRef {
        rr(self.range())
    }

    fn change(&mut self, change: CellFormatChange) -> Result<(), String> {
        let range = self.sel();
        self.apply(Command::FormatCells {
            sheet: self.sheet,
            range,
            change: Box::new(change),
        })
    }

    fn active_format(&self) -> Option<open365_xlsx::CellFormat> {
        let wb = self.target.xlsx()?;
        Some(
            wb.cell_format(self.sheet, self.active.0, self.active.1)
                .clone(),
        )
    }

    // ---- read-backs (the status of the ribbon buttons, and the tests) ------------------------------

    /// The resolved format of a cell.
    pub fn home_cell_format(&self, row: u32, col: u32) -> Option<open365_xlsx::CellFormat> {
        let wb = self.target.xlsx()?;
        Some(wb.cell_format(self.sheet, row, col).clone())
    }

    /// The number format code of a cell (`General` when none).
    pub fn home_format_code(&self, row: u32, col: u32) -> String {
        self.format_code(row, col)
    }

    /// The merged areas of the sheet.
    pub fn home_merged(&self) -> Vec<open365_xlsx::Rect> {
        self.target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .map(|ws| ws.merged_cells())
            .unwrap_or_default()
    }

    /// The height in points of a row (`None` for the default height) and whether it is hidden.
    pub fn home_row_state(&self, row: u32) -> (Option<f64>, bool) {
        let ws = self.target.xlsx().and_then(|wb| wb.sheet(self.sheet));
        match ws.and_then(|ws| ws.rows().iter().find(|r| r.r == row)) {
            Some(r) => (r.height(), r.hidden()),
            None => (None, false),
        }
    }

    /// The stored width (characters) of a column, `None` for the default.
    pub fn home_col_state(&self, col: u32) -> Option<(f64, bool)> {
        let ws = self.target.xlsx().and_then(|wb| wb.sheet(self.sheet))?;
        ws.columns()
            .into_iter()
            .find(|c| c.min <= col && col <= c.max)
            .and_then(|c| c.width.map(|w| (w, c.hidden)))
    }

    // ---- clipboard ---------------------------------------------------------------------------

    /// Paste (`None`) or one of the Paste menu items. Excel does not ask before it overwrites.
    pub fn home_paste(&mut self, kind: Option<PasteKind>) -> Result<(), String> {
        match kind {
            None => {
                if self.paste()? == Paste::NeedsConfirm {
                    self.confirm_paste()?;
                }
                Ok(())
            }
            Some(k) => self.paste_special(k).map(|_| ()),
        }
    }

    /// Format Painter, first click: picks up the formats of the selection.
    pub fn painter_pick_up(&mut self) {
        self.ext_home.painter = Some((self.sheet, self.range()));
    }

    /// Format Painter, second click: paints the picked-up formats over the selection (tiled).
    /// Returns whether anything was picked up. The painter is released afterwards.
    pub fn painter_apply(&mut self) -> Result<bool, String> {
        let Some((sheet, src)) = self.ext_home.painter.take() else {
            return Ok(false);
        };
        let (r1, c1, r2, c2) = self.range();
        let (h, w) = (src.2 - src.0 + 1, src.3 - src.1 + 1);
        // A single cell target gets the whole source block, as in Excel.
        let to = if (r1, c1) == (r2, c2) {
            (r1, c1, (r1 + h - 1).min(MAX_ROW), (c1 + w - 1).min(MAX_COL))
        } else {
            (r1, c1, r2, c2)
        };
        self.apply(Command::CopyFormat {
            sheet: self.sheet,
            from: rr(src),
            to: rr(to),
        })?;
        let _ = sheet;
        self.anchor = (to.0, to.1);
        self.active = (to.0, to.1);
        self.focus = (to.2, to.3);
        Ok(true)
    }

    // ---- font -----------------------------------------------------------------------------------

    pub fn home_font_name(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(());
        }
        self.change(CellFormatChange {
            font: FontChange {
                name: Some(name.to_owned()),
                ..FontChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    pub fn home_font_size(&mut self, size: f64) -> Result<(), String> {
        if !(1.0..=409.0).contains(&size) {
            return Err("The number must be between 1 and 409.".to_owned());
        }
        self.change(CellFormatChange {
            font: FontChange {
                size: Some(size),
                ..FontChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    /// Increase (`up`) or Decrease Font Size: the next size of Excel's list, taken from the
    /// active cell; below 8 pt it steps by one.
    pub fn home_font_step(&mut self, up: bool) -> Result<(), String> {
        let cur = self.active_format().map_or(11.0, |f| f.font.size);
        let next = if up {
            SIZE_STEPS
                .iter()
                .copied()
                .find(|s| *s > cur + 1e-9)
                .unwrap_or(cur)
        } else {
            SIZE_STEPS
                .iter()
                .rev()
                .copied()
                .find(|s| *s < cur - 1e-9)
                .unwrap_or_else(|| (cur - 1.0).max(1.0))
        };
        if (next - cur).abs() < 1e-9 {
            return Ok(());
        }
        self.home_font_size(next)
    }

    pub fn home_bold(&mut self) -> Result<(), String> {
        let now = self.active_format().is_some_and(|f| f.font.bold);
        self.change(CellFormatChange {
            font: FontChange {
                bold: Some(!now),
                ..FontChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    pub fn home_italic(&mut self) -> Result<(), String> {
        let now = self.active_format().is_some_and(|f| f.font.italic);
        self.change(CellFormatChange {
            font: FontChange {
                italic: Some(!now),
                ..FontChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    /// Underline (`double` false: single) toggles against the active cell.
    pub fn home_underline(&mut self, double: bool) -> Result<(), String> {
        let now = self
            .active_format()
            .map_or(Underline::None, |f| f.font.underline);
        let wanted = if double {
            Underline::Double
        } else {
            Underline::Single
        };
        let kind = if now == wanted {
            UnderlineKind::None
        } else if double {
            UnderlineKind::Double
        } else {
            UnderlineKind::Single
        };
        self.change(CellFormatChange {
            font: FontChange {
                underline: Some(kind),
                ..FontChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    /// Fill Colour: `Some("RRGGBB")` fills, `None` is No Fill. Remembered for the button.
    pub fn home_fill(&mut self, rgb: Option<&str>) -> Result<(), String> {
        self.ext_home.fill = rgb.map(str::to_owned);
        self.change(CellFormatChange {
            fill: Some(match rgb {
                Some(c) => FillChange::Solid {
                    color: ColorSpec::rgb(c),
                },
                None => FillChange::None,
            }),
            ..CellFormatChange::default()
        })
    }

    /// Font Colour: `Some("RRGGBB")` or `None` for Automatic. Remembered for the button.
    pub fn home_font_colour(&mut self, rgb: Option<&str>) -> Result<(), String> {
        self.ext_home.font_colour = rgb.map(str::to_owned);
        self.change(CellFormatChange {
            font: FontChange {
                color: Some(rgb.map_or_else(ColorSpec::default, ColorSpec::rgb)),
                ..FontChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    /// The Borders menu. `kind` is one of `bottom`, `top`, `left`, `right`, `none`, `all`,
    /// `outside`, `thick-outside`, `bottom-double`, `bottom-thick`, `top-bottom`,
    /// `top-thick-bottom`, `top-double-bottom`. Remembered for the button.
    pub fn home_border(&mut self, kind: &str) -> Result<(), String> {
        let mut b = BorderChange::default();
        match kind {
            "bottom" => b.bottom = line("thin"),
            "top" => b.top = line("thin"),
            "left" => b.left = line("thin"),
            "right" => b.right = line("thin"),
            "bottom-double" => b.bottom = line("double"),
            "bottom-thick" => b.bottom = line("thick"),
            "top-bottom" => {
                b.top = line("thin");
                b.bottom = line("thin");
            }
            "top-thick-bottom" => {
                b.top = line("thin");
                b.bottom = line("thick");
            }
            "top-double-bottom" => {
                b.top = line("thin");
                b.bottom = line("double");
            }
            "outside" | "thick-outside" | "all" => {
                let s = if kind == "thick-outside" {
                    "thick"
                } else {
                    "thin"
                };
                b.left = line(s);
                b.right = line(s);
                b.top = line(s);
                b.bottom = line(s);
                if kind == "all" {
                    b.inside_horizontal = line(s);
                    b.inside_vertical = line(s);
                }
            }
            "none" => {
                for e in [
                    &mut b.left,
                    &mut b.right,
                    &mut b.top,
                    &mut b.bottom,
                    &mut b.inside_horizontal,
                    &mut b.inside_vertical,
                    &mut b.diagonal_down,
                    &mut b.diagonal_up,
                ] {
                    *e = line("none");
                }
            }
            other => return Err(format!("unknown border {other}")),
        }
        self.ext_home.border = kind.to_owned();
        let range = self.sel();
        self.apply(Command::SetBorders {
            sheet: self.sheet,
            range,
            borders: Box::new(b),
        })
    }

    // ---- alignment ------------------------------------------------------------------------------

    /// Align Left, Centre or Right; pressing the button of the active cell's alignment again goes
    /// back to General.
    pub fn home_halign(&mut self, a: HAlign) -> Result<(), String> {
        let now = self
            .active_format()
            .map_or(HorizontalAlignment::General, |f| f.alignment.horizontal);
        let same = matches!(
            (a, now),
            (HAlign::Left, HorizontalAlignment::Left)
                | (HAlign::Center, HorizontalAlignment::Center)
                | (HAlign::Right, HorizontalAlignment::Right)
        );
        self.change(CellFormatChange {
            alignment: AlignmentChange {
                horizontal: Some(if same { HAlign::General } else { a }),
                ..AlignmentChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    /// Top, Middle or Bottom Align; pressing the active one again goes back to Bottom.
    pub fn home_valign(&mut self, a: VAlign) -> Result<(), String> {
        let now = self
            .active_format()
            .map_or(VerticalAlignment::Bottom, |f| f.alignment.vertical);
        let same = matches!(
            (a, now),
            (VAlign::Top, VerticalAlignment::Top) | (VAlign::Center, VerticalAlignment::Center)
        );
        self.change(CellFormatChange {
            alignment: AlignmentChange {
                vertical: Some(if same { VAlign::Bottom } else { a }),
                ..AlignmentChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    /// Increase (`delta` 1) or Decrease Indent. An indent needs a left or right alignment: a General
    /// cell becomes left aligned (right aligned for a number), as in Excel.
    pub fn home_indent(&mut self, delta: i32) -> Result<(), String> {
        let (cur, horiz) = self
            .active_format()
            .map_or((0, HorizontalAlignment::General), |f| {
                (f.alignment.indent, f.alignment.horizontal)
            });
        let new = (i64::from(cur) + i64::from(delta)).clamp(0, 250) as u32;
        let numeric = self
            .target
            .xlsx()
            .and_then(|wb| wb.cell(self.sheet, self.active.0, self.active.1))
            .is_some_and(|c| matches!(c.value, CellValue::Number(_)));
        let horizontal = match horiz {
            HorizontalAlignment::General | HorizontalAlignment::Center if new > 0 => {
                Some(if numeric && horiz == HorizontalAlignment::General {
                    HAlign::Right
                } else {
                    HAlign::Left
                })
            }
            HorizontalAlignment::Left | HorizontalAlignment::Right if new == 0 => {
                // Excel keeps the explicit alignment.
                None
            }
            _ => None,
        };
        self.change(CellFormatChange {
            alignment: AlignmentChange {
                horizontal,
                indent: Some(new),
                ..AlignmentChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    pub fn home_wrap(&mut self) -> Result<(), String> {
        let now = self.active_format().is_some_and(|f| f.alignment.wrap_text);
        self.change(CellFormatChange {
            alignment: AlignmentChange {
                wrap_text: Some(!now),
                ..AlignmentChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    /// Orientation menu: degrees counter-clockwise (-90 to 90) or stacked letters.
    pub fn home_rotation(&mut self, rotation: RotationSpec) -> Result<(), String> {
        self.change(CellFormatChange {
            alignment: AlignmentChange {
                rotation: Some(rotation),
                ..AlignmentChange::default()
            },
            ..CellFormatChange::default()
        })
    }

    /// Whether the active cell lies in a merged area.
    fn active_is_merged(&self) -> bool {
        let (r, c) = self.active;
        self.target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .is_some_and(|ws| {
                ws.merged_cells()
                    .iter()
                    .any(|m| (m.0..=m.2).contains(&r) && (m.1..=m.3).contains(&c))
            })
    }

    /// The Merge & Center menu. `toggle` is the button itself: merge and centre, or unmerge when
    /// the active cell is already merged. Excel asks before it discards values; the choice is
    /// undoable, so the model merges at once and says what it dropped in the status text.
    pub fn home_merge(&mut self, mode: MergeMode, toggle: bool) -> Result<(), String> {
        let mode = if toggle && self.active_is_merged() {
            MergeMode::Unmerge
        } else {
            mode
        };
        let range = self.sel();
        let dropped = self.target.xlsx().and_then(|wb| {
            open365_commands::merge_discards(wb, self.sheet, range, mode)
                .ok()
                .map(|v| v.len())
        });
        self.apply(Command::MergeCells {
            sheet: self.sheet,
            range,
            mode,
        })?;
        if dropped.is_some_and(|n| n > 0) {
            self.status =
                "Merging cells only keeps the upper-left value and discards other values."
                    .to_owned();
        }
        Ok(())
    }

    // ---- number ---------------------------------------------------------------------------------

    /// One entry of the Number Format list (see [`NUMBER_FORMAT_NAMES`]); the codes come from the
    /// locale of the model.
    pub fn home_number_format(&mut self, name: &str) -> Result<(), String> {
        let loc = self.locale();
        let cur = &loc.currency;
        let spec = match name.trim().to_ascii_lowercase().as_str() {
            "general" => NumberFormatSpec::Builtin(0),
            "number" => NumberFormatSpec::Code("0.00".to_owned()),
            "currency" => NumberFormatSpec::Code(format!("\"{cur}\"#,##0.00")),
            "accounting" => NumberFormatSpec::Code(accounting(cur)),
            "short date" => NumberFormatSpec::Builtin(14),
            "long date" => NumberFormatSpec::Code(loc.long_date.clone()),
            "time" => NumberFormatSpec::Code("[$-F400]h:mm:ss\\ AM/PM".to_owned()),
            "percentage" => NumberFormatSpec::Builtin(10),
            "fraction" => NumberFormatSpec::Builtin(12),
            "scientific" => NumberFormatSpec::Builtin(11),
            "text" => NumberFormatSpec::Builtin(49),
            other => return Err(format!("unknown number format {other}")),
        };
        self.change(CellFormatChange {
            number_format: Some(spec),
            ..CellFormatChange::default()
        })
    }

    /// Format Cells > Number > Custom: the format code `code` on the selection.
    pub fn home_number_code(&mut self, code: &str) -> Result<(), String> {
        self.change(CellFormatChange {
            number_format: Some(NumberFormatSpec::Code(code.to_owned())),
            ..CellFormatChange::default()
        })
    }

    /// Format Cells > Protection: Locked and Hidden (formulas hidden) on the selection.
    pub fn home_protection(&mut self, locked: bool, hidden: bool) -> Result<(), String> {
        self.change(CellFormatChange {
            locked: Some(locked),
            formula_hidden: Some(hidden),
            ..CellFormatChange::default()
        })
    }

    /// Accounting Number Format with the currency symbol `symbol` (the locale's when `None`).
    pub fn home_accounting(&mut self, symbol: Option<&str>) -> Result<(), String> {
        let loc = self.locale();
        let code = accounting(symbol.unwrap_or(&loc.currency));
        self.change(CellFormatChange {
            number_format: Some(NumberFormatSpec::Code(code)),
            ..CellFormatChange::default()
        })
    }

    pub fn home_percent(&mut self) -> Result<(), String> {
        self.change(CellFormatChange {
            number_format: Some(NumberFormatSpec::Builtin(9)),
            ..CellFormatChange::default()
        })
    }

    pub fn home_comma(&mut self) -> Result<(), String> {
        self.home_style("Comma")
    }

    pub fn home_decimals(&mut self, delta: i32) -> Result<(), String> {
        let range = self.sel();
        self.apply(Command::ChangeDecimals {
            sheet: self.sheet,
            range,
            delta,
        })
    }

    // ---- styles ---------------------------------------------------------------------------------

    /// Cell Styles gallery: applies the named style.
    pub fn home_style(&mut self, name: &str) -> Result<(), String> {
        let range = self.sel();
        self.apply(Command::ApplyCellStyle {
            sheet: self.sheet,
            range,
            style: name.to_owned(),
        })
    }

    /// The names the Cell Styles gallery offers.
    pub fn home_style_names(&self) -> Vec<String> {
        self.target
            .xlsx()
            .map(|wb| wb.styles().cell_style_names())
            .unwrap_or_default()
    }

    // ---- cells ----------------------------------------------------------------------------------

    fn selects_whole_rows(&self) -> bool {
        let (_, c1, _, c2) = self.range();
        c1 == 1 && c2 == MAX_COL
    }

    fn selects_whole_cols(&self) -> bool {
        let (r1, _, r2, _) = self.range();
        r1 == 1 && r2 == MAX_ROW
    }

    /// Insert > Insert Cells. Whole rows or columns insert rows or columns; a partial selection
    /// would shift cells within their columns, which the workbook model cannot do yet.
    pub fn home_insert(&mut self) -> Result<bool, String> {
        if self.selects_whole_rows() {
            self.insert_selected(true).map(|()| true)
        } else if self.selects_whole_cols() {
            self.insert_selected(false).map(|()| true)
        } else {
            self.status = "Shifting cells is not supported yet.".to_owned();
            Ok(false)
        }
    }

    /// Delete > Delete Cells (see [`Self::home_insert`] for the partial case).
    pub fn home_delete(&mut self) -> Result<bool, String> {
        if self.selects_whole_rows() {
            self.delete_selected(true).map(|()| true)
        } else if self.selects_whole_cols() {
            self.delete_selected(false).map(|()| true)
        } else {
            self.status = "Shifting cells is not supported yet.".to_owned();
            Ok(false)
        }
    }

    /// Format > Row Height: every selected row gets `points`.
    pub fn home_row_height(&mut self, points: f64) -> Result<(), String> {
        if !(0.0..=409.5).contains(&points) {
            return Err("Enter a number between 0 and 409.5.".to_owned());
        }
        let (r1, _, r2, _) = self.range();
        self.apply(Command::SetRowHeight {
            sheet: self.sheet,
            first: r1,
            last: r2,
            points,
        })
    }

    /// Format > Column Width: `chars` is the width in characters of the default font, as typed
    /// into Excel's dialog.
    pub fn home_col_width(&mut self, chars: f64) -> Result<(), String> {
        if !(0.0..=255.0).contains(&chars) {
            return Err("Enter a number between 0 and 255.".to_owned());
        }
        let (_, c1, _, c2) = self.range();
        let m = self.geometry().metrics();
        let px =
            (chars * f64::from(m.mdw)).round() as u32 + if chars > 0.0 { m.padding() } else { 0 };
        if self.selects_whole_cols() {
            // `set_col_px` gives every selected column the width at once.
            return self.set_col_px(c1, px);
        }
        let sheet = self.sheet;
        let width = chars + f64::from(m.padding()) / f64::from(m.mdw);
        self.apply(Command::SetColumnWidth {
            sheet,
            first: c1,
            last: c2,
            width,
        })
    }

    /// Format > AutoFit Row Height for the selected rows.
    pub fn home_autofit_rows(&mut self) -> Result<(), String> {
        let (r1, _, r2, _) = self.range();
        let points = self
            .target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .map_or(15.0, |ws| ws.sheet_format().default_row_height);
        self.apply(Command::AutoFitRows {
            sheet: self.sheet,
            rows: (r1..=r2).map(|r| (r, points)).collect(),
        })
    }

    /// Format > AutoFit Column Width for the selected columns; `measure` gives the width in
    /// logical pixels of a text.
    pub fn home_autofit_cols(&mut self, measure: &dyn Fn(&str) -> f32) -> Result<(), String> {
        let (_, c1, _, c2) = self.range();
        for col in c1..=c2.min(c1 + 255) {
            self.autofit_col(col, measure)?;
        }
        Ok(())
    }

    /// Format > Hide & Unhide for rows (`rows`) or columns.
    pub fn home_hide(&mut self, rows: bool, hide: bool) -> Result<(), String> {
        let (r1, c1, r2, c2) = self.range();
        let sheet = self.sheet;
        self.apply(match (rows, hide) {
            (true, true) => Command::HideRows {
                sheet,
                first: r1,
                last: r2,
            },
            (true, false) => Command::UnhideRows {
                sheet,
                first: r1,
                last: r2,
            },
            (false, true) => Command::HideColumns {
                sheet,
                first: c1,
                last: c2,
            },
            (false, false) => Command::UnhideColumns {
                sheet,
                first: c1,
                last: c2,
                default_width: None,
            },
        })
    }

    // ---- editing --------------------------------------------------------------------------------

    /// AutoSum and its menu: `func` is `SUM`, `AVERAGE`, `COUNT`, `MAX` or `MIN`. One cell selected:
    /// the formula goes in the active cell and takes the run of numbers above, else to the left.
    /// A block selected: a row of totals under it (a column to the right for a single row).
    pub fn home_auto_sum(&mut self, func: &str) -> Result<(), String> {
        let (r1, c1, r2, c2) = self.range();
        if self.is_range() {
            let mut cells: Vec<(u32, u32, String)> = Vec::new();
            if r2 > r1 {
                if r2 >= MAX_ROW {
                    return Ok(());
                }
                for c in c1..=c2 {
                    cells.push((
                        r2 + 1,
                        c,
                        format!(
                            "={func}({}:{})",
                            crate::sheet::a1(r1, c),
                            crate::sheet::a1(r2, c)
                        ),
                    ));
                }
            } else {
                if c2 >= MAX_COL {
                    return Ok(());
                }
                cells.push((
                    r1,
                    c2 + 1,
                    format!(
                        "={func}({}:{})",
                        crate::sheet::a1(r1, c1),
                        crate::sheet::a1(r1, c2)
                    ),
                ));
            }
            for (r, c, text) in cells {
                self.enter(r, c, &text)?;
            }
            return Ok(());
        }
        let (r, c) = self.active;
        let numeric = |m: &Self, rr: u32, cc: u32| {
            m.target
                .xlsx()
                .and_then(|wb| wb.cell(m.sheet, rr, cc))
                .is_some_and(|x| matches!(x.value, CellValue::Number(_)))
        };
        let mut top = r;
        while top > 1 && numeric(self, top - 1, c) {
            top -= 1;
        }
        let range = if top < r {
            Some((crate::sheet::a1(top, c), crate::sheet::a1(r - 1, c)))
        } else {
            let mut left = c;
            while left > 1 && numeric(self, r, left - 1) {
                left -= 1;
            }
            (left < c).then(|| (crate::sheet::a1(r, left), crate::sheet::a1(r, c - 1)))
        };
        match range {
            Some((a, b)) => self.enter(r, c, &format!("={func}({a}:{b})")),
            None => {
                self.status = "AutoSum found no numbers next to the cell.".to_owned();
                Ok(())
            }
        }
    }

    /// Fill > Down, Right, Up, Left.
    pub fn home_fill_dir(&mut self, dir: FillDir) -> Result<bool, String> {
        match dir {
            FillDir::Down => self.fill_copy(true),
            FillDir::Right => self.fill_copy(false),
            FillDir::Up | FillDir::Left => {
                let (r1, c1, r2, c2) = self.range();
                let up = dir == FillDir::Up;
                let single = if up { r1 == r2 } else { c1 == c2 };
                // The source is the last row (column) of a block, else the cell below (right).
                let (sr, sc) = match (up, single) {
                    (true, false) => (r2, c1),
                    (true, true) => (r1 + 1, c1),
                    (false, false) => (r1, c2),
                    (false, true) => (r1, c1 + 1),
                };
                if sr > MAX_ROW || sc > MAX_COL {
                    return Ok(false);
                }
                let (end_r, end_c) = if up {
                    (if single { r1 } else { r2 - 1 }, c2)
                } else {
                    (r2, if single { c1 } else { c2 - 1 })
                };
                let mut block: Vec<Vec<String>> = Vec::new();
                for r in r1..=end_r {
                    let mut row = Vec::new();
                    for c in c1..=end_c {
                        let (src_r, src_c) = if up { (sr, c) } else { (r, sc) };
                        let text = self.source(src_r, src_c);
                        row.push(if text.starts_with('=') {
                            open365_xlsx::shift_formula(
                                &text,
                                i64::from(r) - i64::from(src_r),
                                i64::from(c) - i64::from(src_c),
                            )
                        } else {
                            text
                        });
                    }
                    block.push(row);
                }
                if block.is_empty() || block[0].is_empty() {
                    return Ok(false);
                }
                self.apply(Command::XlsxPaste {
                    sheet: self.sheet,
                    row: r1,
                    col: c1,
                    rows: block,
                })?;
                Ok(true)
            }
        }
    }

    /// Clear > Clear Contents (the Delete key on the whole selection, one undo step).
    pub fn home_clear_contents(&mut self) -> Result<(), String> {
        let (r1, c1, r2, c2) = self.range();
        let (r2, c2) = match self
            .target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .and_then(|ws| ws.used_range())
        {
            Some(u) => (r2.min(u.2.max(r1)), c2.min(u.3.max(c1))),
            None => return Ok(()),
        };
        let blank = vec![vec![String::new(); (c2 - c1 + 1) as usize]; (r2 - r1 + 1) as usize];
        self.apply(Command::XlsxPaste {
            sheet: self.sheet,
            row: r1,
            col: c1,
            rows: blank,
        })
    }

    /// Clear > Clear Formats.
    pub fn home_clear_formats(&mut self) -> Result<(), String> {
        let range = self.sel();
        self.apply(Command::ClearFormats {
            sheet: self.sheet,
            range,
        })
    }

    /// Clear > Clear All.
    pub fn home_clear_all(&mut self) -> Result<(), String> {
        let range = self.sel();
        self.apply(Command::HomeOp {
            sheet: self.sheet,
            op: HomeOp::ClearAll { range },
        })
    }

    /// Excel's current region of `cell`: the block of non-empty cells around it, bounded by blank
    /// rows and columns.
    pub fn home_current_region(&self, cell: (u32, u32)) -> Rect4 {
        let Some(wb) = self.target.xlsx() else {
            return (cell.0, cell.1, cell.0, cell.1);
        };
        let filled: HashSet<(u32, u32)> = wb
            .iter_cells(self.sheet)
            .filter(|(_, _, c)| !matches!(c.value, CellValue::Empty) || c.formula.is_some())
            .map(|(r, c, _)| (r, c))
            .collect();
        let mut rect = (cell.0, cell.1, cell.0, cell.1);
        loop {
            let (r1, c1, r2, c2) = rect;
            let grown = (
                r1.saturating_sub(1).max(1),
                c1.saturating_sub(1).max(1),
                (r2 + 1).min(MAX_ROW),
                (c2 + 1).min(MAX_COL),
            );
            let mut next = rect;
            for (r, c) in filled.iter().copied() {
                if r >= grown.0 && r <= grown.2 && c >= grown.1 && c <= grown.3 {
                    next = (next.0.min(r), next.1.min(c), next.2.max(r), next.3.max(c));
                }
            }
            if next == rect {
                return rect;
            }
            rect = next;
        }
    }

    /// The range a sort or filter works on: the selection, or the current region of a lone cell.
    fn home_data_range(&self) -> Rect4 {
        if self.is_range() {
            self.range()
        } else {
            self.home_current_region(self.active)
        }
    }

    /// Sort & Filter > Sort A to Z / Z to A: by the column of the active cell.
    pub fn home_sort(&mut self, descending: bool) -> Result<(), String> {
        let range = self.home_data_range();
        self.home_sort_by(
            range,
            &[SortLevel {
                column: self.active.1.clamp(range.1, range.3),
                descending,
            }],
        )
    }

    /// Custom Sort with explicit levels, header row guessed as Excel does.
    pub fn home_sort_by(&mut self, range: Rect4, levels: &[SortLevel]) -> Result<(), String> {
        let loc = self.locale();
        self.apply(Command::HomeOp {
            sheet: self.sheet,
            op: HomeOp::Sort {
                range: rr(range),
                levels: levels.to_vec(),
                header: None,
                locale: Some(loc.tag.clone()),
            },
        })
    }

    /// Whether the sheet has an AutoFilter.
    pub fn home_has_filter(&self) -> bool {
        self.target
            .xlsx()
            .and_then(|wb| wb.sheet(self.sheet))
            .is_some_and(|ws| ws.auto_filter().is_some())
    }

    /// Sort & Filter > Filter: switches the AutoFilter on over the current region, or off.
    pub fn home_toggle_filter(&mut self) -> Result<(), String> {
        let range = if self.home_has_filter() {
            None
        } else {
            Some(rr(self.home_data_range()))
        };
        self.apply(Command::HomeOp {
            sheet: self.sheet,
            op: HomeOp::SetAutoFilter { range },
        })
    }

    /// Sort & Filter > Clear (`clear`) or Reapply.
    pub fn home_refilter(&mut self, clear: bool) -> Result<(), String> {
        if !self.home_has_filter() {
            return Ok(());
        }
        let locale = Some(self.locale().tag.clone());
        let today = today_serial();
        self.apply(Command::HomeOp {
            sheet: self.sheet,
            op: if clear {
                HomeOp::ClearFilter { locale, today }
            } else {
                HomeOp::ReapplyFilter { locale, today }
            },
        })
    }

    /// Find & Select > Find: selects the next cell after the active one that contains `text`
    /// (the search of the Find dialog without options). Returns whether a cell was found.
    pub fn home_find(&mut self, text: &str) -> bool {
        use open365_commands::{CellRef, FindScope, XlsxFind, next_match, xlsx_find_all};
        let Some(wb) = self.target.xlsx() else {
            return false;
        };
        let mut f = XlsxFind::new(text, FindScope::Sheet { sheet: self.sheet });
        f.look_in = open365_commands::LookIn::Formulas;
        let hits = xlsx_find_all(wb, &f);
        let from = Some(CellRef {
            sheet: self.sheet,
            row: self.active.0,
            col: self.active.1,
        });
        match next_match(&hits, from, true, false) {
            Some(h) => {
                self.select_cell((h.row, h.col));
                true
            }
            None => false,
        }
    }

    /// Find & Select > Go To: selects the cell or range `reference` (`B2`, `B2:D9`).
    pub fn home_goto(&mut self, reference: &str) -> bool {
        let reference = reference.trim();
        let (a, b) = reference.split_once(':').unwrap_or((reference, reference));
        let (Some(a), Some(b)) = (crate::sheet::parse_a1(a), crate::sheet::parse_a1(b)) else {
            return false;
        };
        self.select_cell(a);
        self.extend_to(b);
        true
    }
}

/// Fill > Down, Right, Up, Left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillDir {
    Down,
    Right,
    Up,
    Left,
}

fn accounting(symbol: &str) -> String {
    format!(
        "_(\"{s}\"* #,##0.00_);_(\"{s}\"* \\(#,##0.00\\);_(\"{s}\"* \"-\"??_);_(@_)",
        s = symbol
    )
}

/// Today's date as a serial (1900 system), for the date filters.
fn today_serial() -> f64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    (secs / 86_400) as f64 + 25_569.0
}
