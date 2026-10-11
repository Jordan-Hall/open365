//! A pivot refresh saves: the recomputed cells, the pivot table `location` and the cache
//! definition are written, and every other part of the package stays byte-identical.

use std::collections::BTreeMap;
use std::path::PathBuf;

use open365_opc::Limits;
use open365_pivot::{GridValue, PivotStrings, refresh_sheet, sheet_pivots};
use open365_xlsx::{CellInput, CellValue, Workbook};

fn load(name: &str) -> (Vec<u8>, Workbook) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../fixtures/xlsx/pivot-{name}.xlsx"));
    let bytes = std::fs::read(path).expect("fixture");
    let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    (bytes, wb)
}

fn parts(wb: &Workbook) -> BTreeMap<String, Vec<u8>> {
    wb.package()
        .parts()
        .map(|p| (p.name().as_str().to_owned(), p.data().to_vec()))
        .collect()
}

#[test]
fn refresh_writes_cells_location_and_cache_and_keeps_other_parts() {
    let (bytes, mut wb) = load("layouts");
    let before = parts(&wb);
    let si = (0..wb.sheets().len())
        .find(|i| !sheet_pivots(&wb, *i).is_empty())
        .expect("a sheet with a pivot");
    let pivot = sheet_pivots(&wb, si).remove(0);
    let table_part = pivot.part.as_str().to_owned();
    let cache_part = pivot.cache_part.as_str().to_owned();
    // Change one source value of the first data field.
    let data = wb.sheet_index("Data").expect("Data sheet");
    let col = pivot.table.data_fields[0].fld + 1;
    let old = match wb.cell(data, 2, col).map(|c| c.value.clone()) {
        Some(CellValue::Number(n)) => n,
        other => panic!("numeric source cell expected, got {other:?}"),
    };
    wb.set_cell_value(data, 2, col, CellInput::Number(old + 1000.0))
        .expect("edit source");
    let strings = PivotStrings::default();
    assert_eq!(refresh_sheet(&mut wb, si, &strings).expect("refresh"), 1);
    assert!(wb.is_dirty());
    let saved = wb.save().expect("saves");
    assert_ne!(saved, bytes);

    let reopened = Workbook::open(&saved, &Limits::default()).expect("reopens");
    let after = parts(&reopened);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "no part added or removed"
    );
    let changed: Vec<&String> = before
        .iter()
        .filter(|(n, d)| after.get(*n) != Some(*d))
        .map(|(n, _)| n)
        .collect();
    for n in &changed {
        let allowed = **n == table_part
            || **n == cache_part
            || n.starts_with("xl/worksheets/sheet")
            || n.as_str() == "xl/workbook.xml"
            || n.as_str() == "xl/sharedStrings.xml"
            || n.as_str() == "xl/styles.xml";
        assert!(allowed, "unexpected change of {n}");
    }
    assert!(
        changed.contains(&&cache_part),
        "cache definition written: {changed:?}"
    );
    // The `layouts` pivots share one cache, which is not regenerated (Excel does it on load) and
    // its records stay as they were. Regeneration is tested in `refresh_oracle.rs`.
    assert!(
        before
            .iter()
            .filter(|(n, _)| n.starts_with("xl/pivotCache/pivotCacheRecords"))
            .all(|(n, d)| after.get(n) == Some(d))
    );
    let again = sheet_pivots(&reopened, si).remove(0);
    assert!(again.cache.refresh_on_load);
    // The saved cells are the refreshed grid of the saved source.
    let grid = again.grid_refreshed(&reopened, &strings).expect("grid");
    assert!(!grid.cells.is_empty());
    for c in &grid.cells {
        let got = reopened.cell(si, c.row, c.col).map(|x| x.value.clone());
        let same = match (&c.value, got) {
            (GridValue::Number(a), Some(CellValue::Number(b))) => (a - b).abs() < 1e-9,
            (GridValue::Text(a), Some(CellValue::SharedString { text, .. })) => *a == *text,
            (GridValue::Text(a), Some(CellValue::Str(t))) => *a == *t,
            _ => false,
        };
        assert!(same, "cell {},{} differs: {:?}", c.row, c.col, c.value);
    }
}
