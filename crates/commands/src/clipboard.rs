//! What the clipboard carries: Word selections as plain text and HTML, Excel ranges as TSV and
//! HTML, and TSV pasted back in.
//!
//! Nothing here touches the operating system; the shell owns the clipboard. Pasting a range
//! the application copied itself keeps the formulas, and relative references shift by the
//! distance between source and destination as Excel's copy and paste does.
//!
//! The shift is `open365_xlsx::shift_formula`, a token scanner over A1 references (the one that
//! expands shared formulas). It is text-level: when the calculation crate offers a copy-offset
//! on parsed formulas, swap it in here.

use std::fmt::Write as _;

use open365_docx::{Alignment, DocPos, Document, Inline, Paragraph, Run, RunContent};
use open365_xlsx::{CellValue, Workbook};

use crate::command::CellRef;
use crate::find::{cell_display_text, cell_source_text};

/// Text and HTML for the same selection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Clip {
    pub text: String,
    pub html: String,
}

fn escape_html(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
}

fn collect_runs<'a>(content: &'a [Inline], out: &mut Vec<&'a Run>) {
    for inline in content {
        match inline {
            Inline::Run(r) => out.push(r),
            Inline::Group(g) => collect_runs(&g.content, out),
            Inline::Raw(_) => {}
        }
    }
}

fn run_style(doc: &Document, para: &Paragraph, run: &Run) -> String {
    let Some(styles) = doc.styles() else {
        return String::new();
    };
    let e = styles.effective_run_props(para, run);
    let mut css = format!(
        "font-family:'{}';font-size:{}pt",
        e.font.replace(['\'', '"', '<', '>', ';', '&'], ""),
        f64::from(e.size_half_points) / 2.0
    );
    if e.bold {
        css.push_str(";font-weight:bold");
    }
    if e.italic {
        css.push_str(";font-style:italic");
    }
    match (e.underline, e.strike) {
        (true, true) => css.push_str(";text-decoration:underline line-through"),
        (true, false) => css.push_str(";text-decoration:underline"),
        (false, true) => css.push_str(";text-decoration:line-through"),
        (false, false) => {}
    }
    if let Some([r, g, b]) = e.color {
        let _ = write!(css, ";color:#{r:02x}{g:02x}{b:02x}");
    }
    css
}

/// The text between two positions (either order) as plain text, paragraphs separated by
/// `\n`, and as an HTML fragment that keeps character formatting and alignment.
pub fn docx_clip(doc: &Document, a: DocPos, b: DocPos) -> Clip {
    let (start, end) = if a <= b { (a, b) } else { (b, a) };
    let mut text = String::new();
    let mut html = String::new();
    let last = end.para.min(doc.paragraph_count().saturating_sub(1));
    for index in start.para..=last {
        let Some(para) = doc.paragraph(index) else {
            continue;
        };
        let lo = if index == start.para { start.offset } else { 0 };
        let hi = if index == end.para {
            end.offset
        } else {
            usize::MAX
        };
        let tag = match para.style_id() {
            Some("Heading1") => "h1",
            Some("Heading2") => "h2",
            Some("Heading3") => "h3",
            _ => "p",
        };
        let align = doc
            .styles()
            .map(|s| s.effective_para_props(para).alignment)
            .unwrap_or(Alignment::Left);
        let align = match align {
            Alignment::Center => "center",
            Alignment::Right => "right",
            Alignment::Justify => "justify",
            _ => "left",
        };
        let _ = write!(html, "<{tag} style=\"margin:0;text-align:{align}\">");
        let mut runs = Vec::new();
        collect_runs(&para.content, &mut runs);
        let mut at = 0;
        for run in runs {
            let len = run.char_len();
            let (from, to) = (at.max(lo), (at + len).min(hi));
            at += len;
            if from >= to {
                continue;
            }
            let (skip, take) = (from - (at - len), to - from);
            let mut seen = 0;
            let mut piece = String::new();
            let mut piece_html = String::new();
            for item in &run.content {
                let n = item.char_len();
                if n == 0 {
                    continue;
                }
                let (s, e) = (seen, seen + n);
                seen = e;
                if e <= skip || s >= skip + take {
                    continue;
                }
                match item {
                    RunContent::Text { text: t, .. } => {
                        let part: String = t
                            .chars()
                            .skip(skip.saturating_sub(s))
                            .take((skip + take).min(e) - s.max(skip))
                            .collect();
                        escape_html(&part, &mut piece_html);
                        piece.push_str(&part);
                    }
                    RunContent::Tab => {
                        piece.push('\t');
                        piece_html.push_str("&emsp;");
                    }
                    RunContent::Break(_) => {
                        piece.push('\n');
                        piece_html.push_str("<br>");
                    }
                    RunContent::Raw(_) => {}
                }
            }
            text.push_str(&piece);
            let _ = write!(
                html,
                "<span style=\"{}\">{piece_html}</span>",
                run_style(doc, para, run)
            );
        }
        let _ = writeln!(html, "</{tag}>");
        if index != last {
            text.push('\n');
        }
    }
    Clip {
        text,
        html: format!("<html><body><!--StartFragment-->{html}<!--EndFragment--></body></html>"),
    }
}

// ---------------------------------------------------------------------------
// Excel
// ---------------------------------------------------------------------------

/// A copied range: what to put on the clipboard, and what to paste back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridClip {
    /// Top-left cell of the source.
    pub origin: CellRef,
    /// Cell contents as typed (`=...` for formulas), row by row.
    pub rows: Vec<Vec<String>>,
    /// Tab-separated displayed text, one `\n`-ended line per row.
    pub tsv: String,
    pub html: String,
}

fn tsv_field(text: &str, out: &mut String) {
    if text.contains(['\t', '\n', '\r', '"']) {
        out.push('"');
        out.push_str(&text.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(text);
    }
}

/// Copies the cells of `(row1, col1)..=(row2, col2)` on `sheet`. Corners may be in any order;
/// the range is limited to `max_cells` cells.
pub fn grid_copy(
    wb: &Workbook,
    sheet: usize,
    corner_a: (u32, u32),
    corner_b: (u32, u32),
    max_cells: usize,
) -> Option<GridClip> {
    let (r1, r2) = (corner_a.0.min(corner_b.0), corner_a.0.max(corner_b.0));
    let (c1, c2) = (corner_a.1.min(corner_b.1), corner_a.1.max(corner_b.1));
    let count = usize::try_from(r2 - r1 + 1).ok()? * usize::try_from(c2 - c1 + 1).ok()?;
    if count > max_cells {
        return None;
    }
    let mut rows = Vec::new();
    let mut tsv = String::new();
    let mut html = String::from("<html><body><!--StartFragment--><table>");
    for row in r1..=r2 {
        let mut line = Vec::new();
        html.push_str("<tr>");
        for col in c1..=c2 {
            let shown = cell_display_text(wb, sheet, row, col);
            if col > c1 {
                tsv.push('\t');
            }
            tsv_field(&shown, &mut tsv);
            let numeric = matches!(
                wb.cell(sheet, row, col).map(|c| &c.value),
                Some(CellValue::Number(_))
            );
            html.push_str(if numeric {
                "<td style=\"text-align:right\">"
            } else {
                "<td>"
            });
            escape_html(&shown, &mut html);
            html.push_str("</td>");
            line.push(cell_source_text(wb, sheet, row, col));
        }
        html.push_str("</tr>");
        tsv.push('\n');
        rows.push(line);
    }
    html.push_str("</table><!--EndFragment--></body></html>");
    Some(GridClip {
        origin: CellRef {
            sheet,
            row: r1,
            col: c1,
        },
        rows,
        tsv,
        html,
    })
}

/// Splits tab-separated text into rows of fields. Fields in double quotes may hold tabs, line
/// breaks and doubled quotes. A final line break does not start another row.
pub fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    let mut field_started = false;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if !field_started => {
                quoted = true;
                field_started = true;
            }
            '\t' => {
                row.push(std::mem::take(&mut field));
                field_started = false;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                field_started = false;
            }
            other => {
                field.push(other);
                field_started = true;
            }
        }
    }
    if field_started || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// The cells a paste at `dest` writes: the copied rows, with formulas shifted when the copy
/// came from this application (`from` is its origin). Unshifted when `from` is `None`, as for
/// text from elsewhere.
pub fn paste_rows(rows: &[Vec<String>], from: Option<CellRef>, dest: CellRef) -> Vec<Vec<String>> {
    let (d_row, d_col) = from.map_or((0, 0), |o| {
        (
            i64::from(dest.row) - i64::from(o.row),
            i64::from(dest.col) - i64::from(o.col),
        )
    });
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|cell| {
                    if cell.starts_with('=') {
                        open365_xlsx::shift_formula(cell, d_row, d_col)
                    } else {
                        cell.clone()
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_round_trips_quoted_fields() {
        let rows = parse_tsv("a\t\"b\tc\"\r\n\"say \"\"hi\"\"\"\t\n");
        assert_eq!(
            rows,
            vec![
                vec!["a".to_owned(), "b\tc".to_owned()],
                vec!["say \"hi\"".to_owned(), String::new()],
            ]
        );
        let mut out = String::new();
        tsv_field("b\tc", &mut out);
        assert_eq!(out, "\"b\tc\"");
        assert_eq!(parse_tsv("x"), vec![vec!["x".to_owned()]]);
        assert!(parse_tsv("").is_empty());
    }

    #[test]
    fn paste_shifts_only_formulas_from_our_own_copy() {
        let rows = vec![vec!["=A1".to_owned(), "A1".to_owned()]];
        let from = CellRef {
            sheet: 0,
            row: 1,
            col: 2,
        };
        let dest = CellRef {
            sheet: 0,
            row: 3,
            col: 2,
        };
        assert_eq!(paste_rows(&rows, Some(from), dest)[0], ["=A3", "A1"]);
        assert_eq!(paste_rows(&rows, None, dest)[0], ["=A1", "A1"]);
    }
}
