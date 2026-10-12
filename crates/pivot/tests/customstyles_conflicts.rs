//! Custom table style names against Excel (`fixtures/reference/xlsx/pivot-customstyles-conflicts`,
//! made by `tools/office/make-pivot-customstyles-conflicts.ps1`): each case is a workbook whose
//! `styles.xml` was edited to hold custom table styles with colliding names, opened in Excel,
//! and the display format of every pivot cell read back. Measured:
//!
//! * a custom style named like a style Excel ships (`PivotStyleLight16`, `PivotStyleMedium9`,
//!   `TableStyleMedium2`, in any case) never applies: the built-in one does;
//! * names compare without case (`mystyle` is `MyStyle`);
//! * of two custom styles with one name the first in the file applies;
//! * a custom style marked for tables only (`pivot="0"`) still applies to a pivot that names it.

use std::path::PathBuf;

use open365_opc::Limits;
use open365_pivot::style::{CustomStyles, Dxfs, style_cells_full};
use open365_pivot::{PivotStrings, sheet_pivots};
use open365_xlsx::{Workbook, parse_cell_ref};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn custom_style_names_resolve_like_excel() {
    let dir = root().join("fixtures/reference/xlsx/pivot-customstyles-conflicts");
    let refs: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("pivot.json")).expect("pivot.json"))
            .expect("json");
    let mut bad = Vec::new();
    let mut checked = 0;
    for p in refs["pivots"].as_array().expect("pivots") {
        let file = p["file"].as_str().expect("file");
        let bytes = std::fs::read(dir.join(file)).expect("case file");
        let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
        let si = wb.sheet_index("P1").expect("sheet");
        let pivot = sheet_pivots(&wb, si).into_iter().next().expect("pivot");
        let grid = pivot
            .grid_from_cache(&PivotStrings::default())
            .expect("grid");
        let custom = CustomStyles::from_workbook(&wb);
        let dxfs = Dxfs::from_workbook(&wb);
        let styled = style_cells_full(&pivot.table, &grid, wb.styles().theme(), &custom, &dxfs);
        // The built-in table styles (`TableStyle*`) are not in the style tables: no style.
        if p["style"]
            .as_str()
            .is_some_and(|s| s.starts_with("TableStyle"))
        {
            assert!(styled.is_none(), "{file}: a built-in table style name");
            continue;
        }
        let styled = styled.unwrap_or_else(|| panic!("{file}: no style"));
        checked += 1;
        for c in p["cells"].as_array().expect("cells") {
            let a = c["a"].as_str().expect("a");
            let (r, col) = parse_cell_ref(a).expect("address");
            let got = &styled.cells[(r - styled.top) as usize][(col - styled.left) as usize];
            let want_fill = c["f"].as_i64().unwrap_or(-1);
            let got_fill = got.fill.map_or(-1, |f| i64::from(f.bgr_u32()));
            if want_fill != got_fill || (c["b"].as_i64() == Some(1)) != got.bold {
                bad.push(format!(
                    "{file} {a}: fill want {want_fill} got {got_fill}, bold want {} got {}",
                    c["b"], got.bold
                ));
                break;
            }
        }
    }
    assert_eq!(checked, 7);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
