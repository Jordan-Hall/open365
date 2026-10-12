//! Cell formats across a refresh. Excel keeps the formats a user applied to a PivotTable as
//! `formats/format` areas (`pivotArea` + `dxfId`, with "Preserve cell formatting on update" on)
//! and, on every refresh, lays them over the cells of the new layout again: the borders, fills
//! and fonts of the pivot cells in `cellXfs` always come from the format list, so a line that
//! moves takes its formats along, a new line gets the formats of the areas that select it and
//! a stale edge disappears.

use open365_xlsx::{
    BorderEdge, BorderPatch, BorderStyle, Color, ColorSource, FillSpec, FormatPatch, NumberFormat,
    Workbook,
};

use crate::compute::PivotGrid;
use crate::model::PivotTable;
use crate::style::{CellStyle, CustomStyles, Dxfs, Edge, style_cells_full};

fn border_style(e: &Edge) -> BorderStyle {
    match (e.line, e.weight) {
        (-4119, _) => BorderStyle::Double,
        (-4115, -4138) => BorderStyle::MediumDashed,
        (-4115, _) => BorderStyle::Dashed,
        (-4118, _) => BorderStyle::Dotted,
        (_, -4138) => BorderStyle::Medium,
        (_, 4) => BorderStyle::Thick,
        (_, 1) => BorderStyle::Hair,
        _ => BorderStyle::Thin,
    }
}

fn border_edge(e: Option<Edge>) -> Option<BorderEdge> {
    Some(match e {
        None => BorderEdge {
            style: BorderStyle::None,
            color: Color::AUTO,
        },
        Some(e) => BorderEdge {
            style: border_style(&e),
            color: Color {
                source: ColorSource::Rgb(e.color),
                tint: 0.0,
            },
        },
    })
}

/// Lays the user formats of `table` over the cells of the refreshed pivot area: every cell gets
/// exactly the borders, fill, bold, italic and font colour the format areas give it.
pub(crate) fn reapply_user_formats(
    wb: &mut Workbook,
    sheet: usize,
    table: &PivotTable,
    grid: &PivotGrid,
) {
    if table.user_formats().is_empty() {
        return;
    }
    // The user layer alone: no table style underneath (it is not stored in the cells).
    let mut bare = table.clone();
    bare.remove_style_info();
    let custom = CustomStyles::from_workbook(wb);
    let dxfs = Dxfs::from_workbook(wb);
    let Some(styled) = style_cells_full(&bare, grid, wb.styles().theme(), &custom, &dxfs) else {
        return;
    };
    for (i, row) in styled.cells.iter().enumerate() {
        for (j, st) in row.iter().enumerate() {
            let (r, c) = (styled.top + i as u32, styled.left + j as u32);
            let patch = cell_patch(wb, sheet, r, c, st, &dxfs);
            let _ = wb.format_range(sheet, (r, c, r, c), &patch);
        }
    }
}

fn cell_patch(
    wb: &Workbook,
    sheet: usize,
    r: u32,
    c: u32,
    st: &CellStyle,
    dxfs: &Dxfs,
) -> FormatPatch {
    let mut patch = FormatPatch {
        border: BorderPatch {
            left: border_edge(st.borders[0]),
            top: border_edge(st.borders[1]),
            bottom: border_edge(st.borders[2]),
            right: border_edge(st.borders[3]),
            ..BorderPatch::default()
        },
        ..FormatPatch::default()
    };
    let rgb = |x| Color {
        source: ColorSource::Rgb(x),
        tint: 0.0,
    };
    match st.fill {
        Some(f) => patch.fill = Some(FillSpec::Solid(rgb(f))),
        None if wb.cell_format(sheet, r, c).fill != open365_xlsx::Fill::default() => {
            patch.fill = Some(FillSpec::None);
        }
        None => {}
    }
    let font = &wb.cell_format(sheet, r, c).font;
    if st.bold != font.bold {
        patch.font.bold = Some(st.bold);
    }
    if st.italic != font.italic {
        patch.font.italic = Some(st.italic);
    }
    if let Some(f) = st.font_color {
        patch.font.color = Some(rgb(f));
    }
    if let Some(code) = st.num_fmt_id.and_then(|id| dxfs.number_format_code(id)) {
        patch.number_format = Some(NumberFormat::Code(code));
    }
    patch
}
