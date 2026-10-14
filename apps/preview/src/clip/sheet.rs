//! Sheets side of the rich clipboard: a range to a [`SheetClip`] for copy, and a pasted clip to
//! the commands that enter its values and apply its formats.

use std::collections::HashMap;

use open365_clipboard::{
    Borders, CellFont, CellKind, ClipCell, Edge, HAlign, Line, Rgb as ClipRgb, SheetClip, Source,
    Underline, VAlign,
};
use open365_commands::{
    AlignmentChange, BorderChange, CellEntry, CellFormatChange, CellRef, ColorSpec, Command,
    FillChange, FontChange, HAlign as CmdH, LineSpec, NumberFormatSpec, RangeRef, UnderlineKind,
    VAlign as CmdV, cell_display_text, cell_source_text, paste_rows,
};
use open365_xlsx::{
    BorderEdge, BorderStyle, CellValue, ColorSource, FillPattern, HorizontalAlignment,
    Underline as XUnderline, VerticalAlignment, Workbook,
};

fn line_of(s: BorderStyle) -> Option<Line> {
    Some(match s {
        BorderStyle::None => return None,
        BorderStyle::Thin => Line::Thin,
        BorderStyle::Medium => Line::Medium,
        BorderStyle::Dashed => Line::Dashed,
        BorderStyle::Dotted => Line::Dotted,
        BorderStyle::Thick => Line::Thick,
        BorderStyle::Double => Line::Double,
        BorderStyle::Hair => Line::Hair,
        BorderStyle::MediumDashed => Line::MediumDashed,
        BorderStyle::DashDot => Line::DashDot,
        BorderStyle::MediumDashDot => Line::MediumDashDot,
        BorderStyle::DashDotDot => Line::DashDotDot,
        BorderStyle::MediumDashDotDot => Line::MediumDashDotDot,
        BorderStyle::SlantDashDot => Line::SlantDashDot,
    })
}

/// Copies the cells of `(row1, col1, row2, col2)` on `sheet` with values, number formats,
/// fonts, fills, borders and alignment. `None` when the range holds more than `max_cells`.
pub fn copy_range(
    wb: &Workbook,
    sheet: usize,
    rect: (u32, u32, u32, u32),
    max_cells: usize,
) -> Option<SheetClip> {
    let (r1, r2) = (rect.0.min(rect.2), rect.0.max(rect.2));
    let (c1, c2) = (rect.1.min(rect.3), rect.1.max(rect.3));
    let count = usize::try_from(r2 - r1 + 1).ok()? * usize::try_from(c2 - c1 + 1).ok()?;
    if count > max_cells {
        return None;
    }
    let styles = wb.styles();
    let rgb = |c: &open365_xlsx::Color| -> Option<ClipRgb> {
        if matches!(c.source, ColorSource::Auto) {
            return None;
        }
        let v = styles.resolve_color(c);
        Some(ClipRgb::new(v.r, v.g, v.b)).filter(|c| *c != ClipRgb::BLACK)
    };
    let edge = |e: &BorderEdge| -> Option<Edge> {
        line_of(e.style).map(|line| Edge {
            line,
            color: rgb(&e.color),
        })
    };
    let mut rows = Vec::new();
    for row in r1..=r2 {
        let mut line = Vec::new();
        for col in c1..=c2 {
            let mut cell = ClipCell {
                text: cell_display_text(wb, sheet, row, col),
                entry: cell_source_text(wb, sheet, row, col),
                ..ClipCell::default()
            };
            if let Some(c) = wb.cell(sheet, row, col) {
                cell.kind = match &c.value {
                    CellValue::Empty => CellKind::Blank,
                    CellValue::Number(n) => CellKind::Number(*n),
                    CellValue::Bool(b) => CellKind::Bool(*b),
                    CellValue::Error(_) => CellKind::Error,
                    _ => CellKind::Text,
                };
                if cell.kind == CellKind::Blank && !cell.entry.is_empty() {
                    cell.kind = CellKind::Unparsed;
                }
                if c.style.is_some() {
                    let xf = c.style.unwrap_or(0);
                    let f = styles.cell_format(xf);
                    let code = styles.number_format(xf);
                    cell.number_format = (code != "General").then(|| code.to_owned());
                    cell.font = CellFont {
                        name: Some(f.font.name.clone()),
                        size_pt: Some(f.font.size as f32),
                        bold: f.font.bold,
                        italic: f.font.italic,
                        underline: match f.font.underline {
                            XUnderline::None => Underline::None,
                            XUnderline::Double | XUnderline::DoubleAccounting => Underline::Double,
                            _ => Underline::Single,
                        },
                        strike: f.font.strike,
                        color: rgb(&f.font.color),
                    };
                    if f.fill.pattern == FillPattern::Solid {
                        let v = styles.resolve_color(&f.fill.fg);
                        cell.fill = Some(ClipRgb::new(v.r, v.g, v.b));
                    }
                    cell.borders = Borders {
                        left: edge(&f.border.left),
                        right: edge(&f.border.right),
                        top: edge(&f.border.top),
                        bottom: edge(&f.border.bottom),
                    };
                    cell.halign = match f.alignment.horizontal {
                        HorizontalAlignment::General => None,
                        HorizontalAlignment::Left => Some(HAlign::Left),
                        HorizontalAlignment::Right => Some(HAlign::Right),
                        HorizontalAlignment::Fill => Some(HAlign::Fill),
                        HorizontalAlignment::Justify | HorizontalAlignment::Distributed => {
                            Some(HAlign::Justify)
                        }
                        HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => {
                            Some(HAlign::Center)
                        }
                    };
                    cell.valign = match f.alignment.vertical {
                        VerticalAlignment::Bottom => None,
                        VerticalAlignment::Top => Some(VAlign::Top),
                        VerticalAlignment::Center => Some(VAlign::Center),
                        VerticalAlignment::Justify | VerticalAlignment::Distributed => {
                            Some(VAlign::Center)
                        }
                    };
                    cell.wrap = f.alignment.wrap_text;
                }
            }
            line.push(cell);
        }
        rows.push(line);
    }
    Some(SheetClip {
        origin: Some((r1, c1)),
        rows,
    })
}

/// What a Paste Special puts in the destination (the dialog's "Paste" column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    All,
    Formulas,
    Values,
    Formats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub mode: Mode,
    /// Blank source cells leave the destination as it is.
    pub skip_blanks: bool,
    pub transpose: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::All,
            skip_blanks: false,
            transpose: false,
        }
    }
}

/// The commands of one paste, in the order to apply them (formats first, so one undo takes back
/// the values), and the rectangle they cover.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub commands: Vec<Command>,
    /// `(row, col, row, col)`, 1-based, inclusive; `None` when nothing is pasted.
    pub rect: Option<(u32, u32, u32, u32)>,
    /// The paste overwrites cells that hold something (the caller confirms first).
    pub values: Vec<Command>,
}

fn spec(c: ClipRgb) -> ColorSpec {
    ColorSpec::rgb(&c.css()[1..])
}

fn change_of(c: &ClipCell) -> CellFormatChange {
    CellFormatChange {
        font: FontChange {
            name: c.font.name.clone(),
            size: c.font.size_pt.map(f64::from),
            bold: Some(c.font.bold),
            italic: Some(c.font.italic),
            underline: Some(match c.font.underline {
                Underline::None => UnderlineKind::None,
                Underline::Single => UnderlineKind::Single,
                Underline::Double => UnderlineKind::Double,
            }),
            strike: Some(c.font.strike),
            color: Some(c.font.color.map_or_else(ColorSpec::default, spec)),
        },
        fill: Some(c.fill.map_or(FillChange::None, |color| FillChange::Solid {
            color: spec(color),
        })),
        alignment: AlignmentChange {
            horizontal: Some(match c.halign {
                None => CmdH::General,
                Some(HAlign::Left) => CmdH::Left,
                Some(HAlign::Center) => CmdH::Center,
                Some(HAlign::Right) => CmdH::Right,
                Some(HAlign::Justify) => CmdH::Justify,
                Some(HAlign::Fill) => CmdH::Fill,
            }),
            vertical: Some(match c.valign {
                None | Some(VAlign::Bottom) => CmdV::Bottom,
                Some(VAlign::Top) => CmdV::Top,
                Some(VAlign::Center) => CmdV::Center,
            }),
            wrap_text: Some(c.wrap),
            ..AlignmentChange::default()
        },
        number_format: Some(NumberFormatSpec::Code(
            c.number_format
                .clone()
                .unwrap_or_else(|| "General".to_owned()),
        )),
        ..CellFormatChange::default()
    }
}

fn line_spec(e: &Option<Edge>) -> Option<LineSpec> {
    e.map(|e| LineSpec {
        style: e.line.ooxml().to_owned(),
        color: e.color.map(spec),
    })
}

fn borders_of(b: &Borders) -> Option<BorderChange> {
    if b.is_empty() {
        return None;
    }
    Some(BorderChange {
        left: line_spec(&b.left),
        right: line_spec(&b.right),
        top: line_spec(&b.top),
        bottom: line_spec(&b.bottom),
        ..BorderChange::default()
    })
}

fn none_line() -> Option<LineSpec> {
    Some(LineSpec {
        style: "none".to_owned(),
        color: None,
    })
}

/// A value to enter instead of a formula (Paste Special > Values).
fn value_entry(c: &ClipCell) -> String {
    match c.kind {
        CellKind::Number(n) => format!("{n}"),
        CellKind::Bool(b) => if b { "TRUE" } else { "FALSE" }.to_owned(),
        CellKind::Text | CellKind::Error => c.text.clone(),
        CellKind::Blank | CellKind::Unparsed => c.entry.clone(),
    }
}

/// Text that the editor would read as a number, a boolean or a formula, but that the source
/// holds as text: it is entered as text afterwards.
fn needs_text_entry(c: &ClipCell) -> bool {
    c.kind == CellKind::Text
        && !c.entry.is_empty()
        && (c.entry.trim_start().starts_with('=')
            || CellEntry::from_user_text(&c.entry) != CellEntry::Text(c.entry.clone()))
}

/// Plans a paste of `clip` with its top-left cell at `dest` (`(row, col)`, 1-based). Formats
/// are applied only for clips that carry them (`Source::Text` has none); formulas shift only for
/// clips copied inside the application (`Source::Private`).
pub fn plan(
    clip: &SheetClip,
    source: Source,
    sheet: usize,
    dest: (u32, u32),
    opts: Options,
) -> Plan {
    let clip = if opts.transpose {
        clip.transposed()
    } else {
        clip.clone()
    };
    let (h, w) = (clip.height(), clip.width());
    if h == 0 || w == 0 {
        return Plan {
            commands: Vec::new(),
            rect: None,
            values: Vec::new(),
        };
    }
    let end = (
        dest.0.saturating_add(h as u32 - 1),
        dest.1.saturating_add(w as u32 - 1),
    );
    let cell_at = |r: usize, c: usize| clip.rows[r].get(c);
    let mut formats = Vec::new();
    let mut values = Vec::new();
    let mut fixups = Vec::new();

    if matches!(opts.mode, Mode::All | Mode::Formats) && source != Source::Text {
        // Pasted cells take the source's borders, so the old ones go first.
        formats.push(Command::SetBorders {
            sheet,
            range: RangeRef {
                first_row: dest.0,
                first_col: dest.1,
                last_row: end.0,
                last_col: end.1,
            },
            borders: Box::new(BorderChange {
                left: none_line(),
                right: none_line(),
                top: none_line(),
                bottom: none_line(),
                inside_horizontal: none_line(),
                inside_vertical: none_line(),
                ..BorderChange::default()
            }),
        });
        // Cells with equal formats merge into rectangles: one command per rectangle.
        let mut keys: Vec<(CellFormatChange, Option<BorderChange>)> = Vec::new();
        let mut grid: Vec<Vec<Option<usize>>> = vec![vec![None; w]; h];
        for (r, grid_row) in grid.iter_mut().enumerate() {
            for (c, slot) in grid_row.iter_mut().enumerate() {
                let Some(cell) = cell_at(r, c) else { continue };
                if cell.covered || (!cell.is_formatted() && cell.font == CellFont::default()) {
                    continue;
                }
                let key = (change_of(cell), borders_of(&cell.borders));
                let id = keys.iter().position(|k| *k == key).unwrap_or_else(|| {
                    keys.push(key);
                    keys.len() - 1
                });
                *slot = Some(id);
            }
        }
        // (first col, last col, key) -> (first row, last row)
        let mut open: HashMap<(usize, usize, usize), (usize, usize)> = HashMap::new();
        let mut rects: Vec<(usize, usize, usize, usize, usize)> = Vec::new();
        for r in 0..=h {
            let mut seen = Vec::new();
            if let Some(grid_row) = grid.get(r) {
                let mut c = 0;
                while c < w {
                    let Some(id) = grid_row[c] else {
                        c += 1;
                        continue;
                    };
                    let mut e = c;
                    while e + 1 < w && grid_row[e + 1] == Some(id) {
                        e += 1;
                    }
                    let k = (c, e, id);
                    match open.get_mut(&k) {
                        Some(span) if span.1 + 1 == r => span.1 = r,
                        _ => {
                            if let Some(old) = open.insert(k, (r, r)) {
                                rects.push((old.0, old.1, k.0, k.1, k.2));
                            }
                        }
                    }
                    seen.push(k);
                    c = e + 1;
                }
            }
            let closed: Vec<_> = open.keys().copied().filter(|k| !seen.contains(k)).collect();
            for k in closed {
                if let Some(span) = open.remove(&k) {
                    rects.push((span.0, span.1, k.0, k.1, k.2));
                }
            }
        }
        rects.sort_unstable();
        let mut border_cmds = Vec::new();
        for (r1, r2, c1, c2, id) in rects {
            let range = RangeRef {
                first_row: dest.0 + r1 as u32,
                first_col: dest.1 + c1 as u32,
                last_row: dest.0 + r2 as u32,
                last_col: dest.1 + c2 as u32,
            };
            let (change, borders) = &keys[id];
            formats.push(Command::FormatCells {
                sheet,
                range,
                change: Box::new(change.clone()),
            });
            if let Some(b) = borders {
                border_cmds.push(Command::SetBorders {
                    sheet,
                    range,
                    borders: Box::new(b.clone()),
                });
            }
        }
        formats.extend(border_cmds);
    }

    if matches!(opts.mode, Mode::All | Mode::Formulas | Mode::Values) {
        let mut entries: Vec<Vec<String>> = (0..h)
            .map(|r| {
                (0..w)
                    .map(|c| {
                        cell_at(r, c).map_or_else(String::new, |cell| match opts.mode {
                            Mode::Values => value_entry(cell),
                            _ => cell.entry.clone(),
                        })
                    })
                    .collect()
            })
            .collect();
        if let (Source::Private, Some((r, c))) = (source, clip.origin)
            && opts.mode != Mode::Values
        {
            let from = CellRef {
                sheet,
                row: r,
                col: c,
            };
            let to = CellRef {
                sheet,
                row: dest.0,
                col: dest.1,
            };
            entries = paste_rows(&entries, Some(from), to);
        }
        // Text the editor would read as something else is entered as text afterwards.
        for (r, entry_row) in entries.iter_mut().enumerate() {
            for (c, entry) in entry_row.iter_mut().enumerate() {
                if let Some(cell) = cell_at(r, c)
                    && needs_text_entry(cell)
                {
                    entry.clear();
                    fixups.push(Command::XlsxSetCellValue {
                        sheet,
                        row: dest.0 + r as u32,
                        col: dest.1 + c as u32,
                        value: CellEntry::Text(cell.entry.clone()),
                    });
                }
            }
        }
        let blank = |r: usize, c: usize| {
            cell_at(r, c).is_none_or(|x| x.entry.is_empty() && x.kind == CellKind::Blank)
        };
        if opts.skip_blanks {
            // One paste per run of non-blank cells in a row.
            for (r, entry_row) in entries.iter().enumerate() {
                let mut c = 0;
                while c < w {
                    if blank(r, c) {
                        c += 1;
                        continue;
                    }
                    let mut e = c;
                    while e + 1 < w && !blank(r, e + 1) {
                        e += 1;
                    }
                    values.push(Command::XlsxPaste {
                        sheet,
                        row: dest.0 + r as u32,
                        col: dest.1 + c as u32,
                        rows: vec![entry_row[c..=e].to_vec()],
                    });
                    c = e + 1;
                }
            }
        } else {
            values.push(Command::XlsxPaste {
                sheet,
                row: dest.0,
                col: dest.1,
                rows: entries,
            });
        }
    }

    let mut commands = formats;
    commands.extend(values.iter().cloned());
    commands.extend(fixups);
    Plan {
        commands,
        rect: Some((dest.0, dest.1, end.0, end.1)),
        values,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use open365_commands::Target;

    fn blank() -> Target {
        Target::open_xlsx(&crate::sheet::blank::xlsx()).expect("the blank workbook opens")
    }

    fn excel_html() -> &'static str {
        "<html xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><head><style><!--\
         td {font-size:11.0pt; font-family:Calibri, sans-serif; color:windowtext; vertical-align:bottom;}\
         .money {mso-number-format:\"\\0022\\$\\0022\\#\\,\\#\\#0\\.00\"; font-weight:700; color:#C00000; background:#FFFF00; mso-pattern:black none;}\
         .edge {border-top:.5pt solid windowtext; border-bottom:1.5pt solid #0000FF;}\
         --></style></head><body><!--StartFragment--><table>\
         <tr><td class=money x:num=\"1234.5\">$1,234.50</td><td class=edge x:str>00123</td><td x:num x:fmla=\"=A1*2\">2469</td></tr>\
         <tr><td>plain</td><td class=money x:num=\"2\">$2.00</td><td></td></tr>\
         </table><!--EndFragment--></body></html>"
    }

    fn pasted() -> (Target, SheetClip) {
        let clip = SheetClip::from_html(excel_html()).expect("html parses");
        (blank(), clip)
    }

    #[test]
    fn excel_html_paste_keeps_values_formats_fills_and_borders() {
        let (mut t, clip) = pasted();
        let p = plan(&clip, Source::Html, 0, (2, 2), Options::default());
        for c in &p.commands {
            t.apply(c.clone()).unwrap();
        }
        let wb = t.xlsx().unwrap();
        // Values: number, forced text keeps its zeros, formula.
        assert_eq!(cell_display_text(wb, 0, 2, 2), "$1,234.50");
        assert!(matches!(wb.cell(0, 2, 2).unwrap().value, CellValue::Number(n) if n == 1234.5));
        assert_eq!(cell_display_text(wb, 0, 2, 3), "00123");
        assert!(matches!(
            wb.cell(0, 2, 3).unwrap().value,
            CellValue::Str(_) | CellValue::SharedString { .. } | CellValue::InlineStr(_)
        ));
        assert_eq!(cell_source_text(wb, 0, 2, 4), "=A1*2");
        // Round trip through our own reader: formats are on the cells.
        let back = copy_range(wb, 0, (2, 2, 3, 4), 1000).unwrap();
        let a = &back.rows[0][0];
        assert_eq!(a.number_format.as_deref(), Some("\"$\"#,##0.00"));
        assert!(a.font.bold);
        assert_eq!(a.font.color, Some(ClipRgb::new(0xC0, 0, 0)));
        assert_eq!(a.fill, Some(ClipRgb::new(255, 255, 0)));
        assert_eq!(a.font.name.as_deref(), Some("Calibri"));
        let b = &back.rows[0][1];
        assert_eq!(b.borders.top.map(|e| e.line), Some(Line::Thin));
        assert_eq!(b.borders.bottom.map(|e| e.line), Some(Line::Medium));
        assert_eq!(
            b.borders.bottom.and_then(|e| e.color),
            Some(ClipRgb::new(0, 0, 255))
        );
        assert_eq!(back.rows[1][0].fill, None);
        // 6 cells, 3 distinct formats, 5 rectangles (the default style joins C1:C2).
        let formats = p
            .commands
            .iter()
            .filter(|c| matches!(c, Command::FormatCells { .. }))
            .count();
        assert_eq!(formats, 5, "one command per rectangle of equal format");
    }

    #[test]
    fn private_clip_shifts_formulas_and_modes_work() {
        let (mut t, clip) = pasted();
        let mut private = clip.clone();
        private.origin = Some((2, 2));
        // Paste the formula one column right.
        let p = plan(&private, Source::Private, 0, (2, 3), Options::default());
        for c in &p.commands {
            t.apply(c.clone()).unwrap();
        }
        assert_eq!(cell_source_text(t.xlsx().unwrap(), 0, 2, 5), "=B1*2");
        // Values: the formula becomes its value, no formats.
        let v = plan(
            &private,
            Source::Private,
            0,
            (10, 1),
            Options {
                mode: Mode::Values,
                ..Options::default()
            },
        );
        assert!(
            !v.commands
                .iter()
                .any(|c| matches!(c, Command::FormatCells { .. }))
        );
        for c in &v.commands {
            t.apply(c.clone()).unwrap();
        }
        assert_eq!(cell_source_text(t.xlsx().unwrap(), 0, 10, 3), "2469");
        // Formats only: no values.
        let f = plan(
            &private,
            Source::Private,
            0,
            (20, 1),
            Options {
                mode: Mode::Formats,
                ..Options::default()
            },
        );
        assert!(f.values.is_empty());
        assert!(
            f.commands
                .iter()
                .any(|c| matches!(c, Command::FormatCells { .. }))
        );
    }

    #[test]
    fn tsv_paste_has_no_formats_and_transpose_swaps() {
        let clip = SheetClip::from_tsv("a\tb\r\n1\t2\r\n").unwrap();
        let p = plan(&clip, Source::Text, 0, (1, 1), Options::default());
        assert!(
            p.commands
                .iter()
                .all(|c| matches!(c, Command::XlsxPaste { .. }))
        );
        let t = plan(
            &clip,
            Source::Text,
            0,
            (1, 1),
            Options {
                transpose: true,
                ..Options::default()
            },
        );
        assert_eq!(
            t.commands,
            vec![Command::XlsxPaste {
                sheet: 0,
                row: 1,
                col: 1,
                rows: vec![vec!["a".into(), "1".into()], vec!["b".into(), "2".into()]],
            }]
        );
        let s = plan(
            &SheetClip::from_grid(&[vec!["x".into(), String::new(), "y".into()]]),
            Source::Text,
            0,
            (1, 1),
            Options {
                skip_blanks: true,
                ..Options::default()
            },
        );
        assert_eq!(s.values.len(), 2);
    }
}
