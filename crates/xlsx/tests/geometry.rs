//! Sheet geometry against Excel: `fixtures/reference/xlsx/<name>/geometry.json` holds
//! `Range.Width` and `Range.Height` (points) of every column and row Excel laid out. Our pixel
//! results converted to points must be equal, with no tolerance.

mod common;

use open365_opc::Limits;
use open365_xlsx::{FrozenPanes, GridMetrics, Workbook};
use serde_json::Value;

use common::{fixture, fixtures, repo_root};

fn geometry_json(name: &str) -> Value {
    let path = repo_root()
        .join("fixtures")
        .join("reference")
        .join("xlsx")
        .join(name)
        .join("geometry.json");
    serde_json::from_slice(&std::fs::read(&path).expect("geometry.json exists"))
        .expect("geometry.json parses")
}

fn open(name: &str) -> Workbook {
    Workbook::open(&fixture(name), &Limits::default()).expect("opens")
}

fn points(v: &Value) -> Vec<f64> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|x| x.as_f64().expect("number"))
        .collect()
}

#[test]
fn widths_and_heights_equal_excel_for_every_fixture() {
    let (mut matched, mut total) = (0usize, 0usize);
    for (name, _) in fixtures() {
        let wb = open(&name);
        let json = geometry_json(&name);
        let dpi = json["dpi"].as_u64().expect("dpi") as u32;
        let metrics = GridMetrics::for_styles(wb.styles(), dpi);
        for sheet in json["sheets"].as_array().expect("sheets") {
            let sheet_name = sheet["name"].as_str().expect("name");
            let index = wb
                .sheets()
                .iter()
                .position(|s| s.name == sheet_name)
                .unwrap_or_else(|| panic!("{name}: sheet {sheet_name} missing"));
            let ws = wb
                .sheet(index)
                .unwrap_or_else(|| panic!("{name}: {sheet_name} is not a worksheet"));
            let geo = ws.geometry(metrics);
            for (i, want) in points(&sheet["col_widths_pt"]).into_iter().enumerate() {
                let col = i as u32 + 1;
                let got = metrics.px_to_pt(geo.col_px(col));
                total += 1;
                assert!(
                    got == want,
                    "{name}/{sheet_name} column {col}: ours {got} pt ({} px), Excel {want} pt",
                    geo.col_px(col)
                );
                matched += 1;
            }
            for (i, want) in points(&sheet["row_heights_pt"]).into_iter().enumerate() {
                let row = i as u32 + 1;
                let got = metrics.px_to_pt(geo.row_px(row));
                total += 1;
                assert!(
                    got == want,
                    "{name}/{sheet_name} row {row}: ours {got} pt ({} px), Excel {want} pt",
                    geo.row_px(row)
                );
                matched += 1;
            }
        }
    }
    println!("geometry parity: {matched}/{total}");
    assert_eq!(matched, total);
    assert!(total > 500, "only {total} measurements compared");
}

#[test]
fn merged_cells_are_read() {
    let wb = open("merged-cells");
    let ws = wb.sheet(0).expect("sheet");
    assert_eq!(ws.merged_cells(), vec![(1, 1, 1, 3), (3, 1, 5, 1)]);
    assert!(
        open("number-formats")
            .sheet(0)
            .unwrap()
            .merged_cells()
            .is_empty()
    );
}

#[test]
fn columns_format_rows_and_panes_are_read() {
    let wb = open("grid-geometry");
    let ws = wb.sheet(0).expect("sheet");
    let cols = ws.columns();
    assert_eq!(cols.len(), 11);
    assert_eq!(cols[0].width, Some(3.77734375));
    assert!(cols[0].custom_width && !cols[0].hidden);
    assert!(cols[10].hidden);
    assert_eq!(ws.frozen_panes(), Some(FrozenPanes { rows: 2, cols: 2 }));
    let row5 = ws.rows().iter().find(|r| r.r == 5).expect("row 5");
    assert_eq!(row5.height(), Some(30.0));
    assert!(row5.custom_height() && !row5.hidden());
    assert!(ws.rows().iter().find(|r| r.r == 8).expect("row 8").hidden());

    let ws = wb.sheet(1).expect("sheet");
    let format = ws.sheet_format();
    assert_eq!(format.default_col_width, Some(12.77734375));
    assert_eq!(format.default_row_height, 20.25);
    assert!(format.custom_height);
    assert_eq!(ws.frozen_panes(), Some(FrozenPanes { rows: 0, cols: 1 }));
    assert_eq!(ws.merged_cells(), vec![(2, 2, 2, 3)]);
}

#[test]
fn unfrozen_sheet_has_no_panes() {
    assert_eq!(
        open("formulas-basic").sheet(0).unwrap().frozen_panes(),
        None
    );
}

#[test]
fn offsets_and_hit_testing_agree() {
    let wb = open("grid-geometry");
    let geo = wb
        .sheet(0)
        .unwrap()
        .geometry(GridMetrics::for_styles(wb.styles(), 96));
    for col in [1u32, 2, 3, 9, 10, 11, 12, 40] {
        let x = geo.col_x(col);
        let sum: u64 = (1..col).map(|c| u64::from(geo.col_px(c))).sum();
        assert_eq!(x, sum, "col_x({col})");
        if geo.col_px(col) > 0 {
            assert_eq!(geo.col_at(x, 100), col);
            assert_eq!(geo.col_at(x + u64::from(geo.col_px(col)) - 1, 100), col);
        }
    }
    for row in [1u32, 4, 5, 8, 9, 10, 11, 60] {
        let y = geo.row_y(row);
        let sum: u64 = (1..row).map(|r| u64::from(geo.row_px(r))).sum();
        assert_eq!(y, sum, "row_y({row})");
        if geo.row_px(row) > 0 {
            assert_eq!(geo.row_at(y, 100), row);
        }
    }
    // Hidden row 8 and hidden column 11 take no space.
    assert_eq!(geo.row_px(8), 0);
    assert_eq!(geo.col_px(11), 0);
    assert_eq!(geo.row_y(8), geo.row_y(9));
}
