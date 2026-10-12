//! Controlled probes of user border formats against Excel (`fixtures/reference/xlsx/pivot-formats-probe`,
//! made by `tools/office/make-pivot-formats-probe.ps1`): one pivot per case, one border format (or two
//! that meet on an edge) applied through the Excel object model, the workbook saved after a refresh.
//! `probe.json` holds `Range.DisplayFormat` borders of every cell of the pivot range (the entries
//! marked `[refreshed]` match the saved workbook). The rules the engine follows: a dxf border is the
//! outline of the selected cells unless `outline="0"`, and a shared edge shows the heavier line, then
//! the darker colour, on both cells.

use std::path::PathBuf;

use open365_opc::Limits;
use open365_pivot::style::{CustomStyles, Dxfs, style_cells_full};
use open365_pivot::{PivotStrings, sheet_pivots};
use open365_xlsx::{Workbook, parse_cell_ref};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn edge(have: Option<open365_pivot::style::Edge>) -> String {
    have.map_or(String::new(), |e| {
        format!("{}:{}:{}", e.line, e.weight, e.color.bgr_u32())
    })
}

/// Probe cases that must match Excel on every cell of the pivot range (of 43: the open ones are the
/// repeated `(blank)` item box in outline form and a tabular item label range).
const EXACT: usize = 41;

#[test]
fn border_format_probes_match_excel() {
    let dir = root().join("fixtures/reference/xlsx/pivot-formats-probe");
    let refs: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("probe.json")).expect("probe.json"))
            .expect("json");
    let bytes = std::fs::read(dir.join("probe.xlsx")).expect("probe.xlsx");
    let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    let (mut ok, mut total) = (0, 0);
    let mut bad = Vec::new();
    for p in refs["pivots"].as_array().expect("pivots") {
        let case = p["case"].as_str().expect("case");
        if !case.ends_with("[refreshed]") {
            continue;
        }
        let sheet = p["sheet"].as_str().expect("sheet");
        let si = wb.sheet_index(sheet).expect("sheet");
        let pivot = sheet_pivots(&wb, si).into_iter().next().expect("pivot");
        let grid = pivot
            .grid_from_cache(&PivotStrings::default())
            .expect("grid");
        let styled = style_cells_full(
            &pivot.table,
            &grid,
            wb.styles().theme(),
            &CustomStyles::from_workbook(&wb),
            &Dxfs::from_workbook(&wb),
        )
        .expect("style");
        total += 1;
        let mut diffs = Vec::new();
        for c in p["cells"].as_array().expect("cells") {
            let a = c["a"].as_str().expect("a");
            let (r, col) = parse_cell_ref(a).expect("address");
            let Some(got) = styled
                .cells
                .get(r.wrapping_sub(styled.top) as usize)
                .and_then(|row| row.get(col.wrapping_sub(styled.left) as usize))
            else {
                continue;
            };
            for (k, b) in c["disp"].as_array().expect("disp").iter().enumerate() {
                let (want, have) = (b.as_str().unwrap_or(""), edge(got.borders[k]));
                if want != have {
                    diffs.push(format!("{a} side {k} want {want:?} got {have:?}"));
                }
            }
        }
        if diffs.is_empty() {
            ok += 1;
        } else {
            bad.push(format!("{sheet} {case}: {}", diffs.join("; ")));
        }
    }
    if std::env::var("PIVOT_VERBOSE").is_ok() {
        for b in &bad {
            println!("PROBE {b}");
        }
    }
    println!("formats-probe: {ok}/{total} cases");
    assert!(ok >= EXACT, "{ok}/{total} probe cases match, floor {EXACT}");
}
