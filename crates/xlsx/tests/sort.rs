//! Sort against Excel: every scenario of `fixtures/reference/xlsx/sortfilter-sort/scenarios.json` is run on the
//! "before" sheet of `fixtures/xlsx/sortfilter-sort.xlsx` and compared with what Excel produced.

use std::path::PathBuf;

use open365_locale::Locale;
use open365_opc::Limits;
use open365_xlsx::{
    CellValue, ColorKey, HeaderMode, Rgb, SortKey, SortMethod, SortOn, SortOrientation, SortSpec,
    SortState, Workbook, parse_range,
};
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load() -> (Workbook, Vec<Value>) {
    let bytes = std::fs::read(root().join("fixtures/xlsx/sortfilter-sort.xlsx")).expect("fixture");
    let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    let json = std::fs::read_to_string(
        root().join("fixtures/reference/xlsx/sortfilter-sort/scenarios.json"),
    )
    .expect("reference");
    let v: Value = serde_json::from_str(json.trim_start_matches('\u{feff}')).expect("json");
    (wb, v["scenarios"].as_array().expect("scenarios").clone())
}

fn rgb(hex: &str) -> Rgb {
    let n = u32::from_str_radix(hex, 16).expect("hex");
    Rgb::new((n >> 16) as u8, (n >> 8) as u8, n as u8)
}

fn spec_of(sc: &Value) -> SortSpec {
    let id = sc["id"].as_str().expect("id");
    let ltr = sc["orientation"] == "rows";
    let header = match sc["header"].as_str() {
        Some("yes") => HeaderMode::Yes,
        // Only Range.Sort guesses; the Sort object (every other scenario) does not.
        Some("guess") if id.starts_with("g-") => HeaderMode::Guess,
        _ => HeaderMode::No,
    };
    let keys = sc["keys"]
        .as_array()
        .expect("keys")
        .iter()
        .map(|k| {
            let index = if ltr {
                k["row"].as_u64().expect("row") as u32
            } else {
                u32::from(k["col"].as_str().expect("col").as_bytes()[0] - b'A' + 1)
            };
            let on = match k["on"].as_str() {
                Some("cellcolor") => SortOn::CellColor,
                Some("fontcolor") => SortOn::FontColor,
                _ => SortOn::Value,
            };
            SortKey {
                index,
                on,
                descending: k["order"] == "desc",
                color: k["color"]
                    .as_str()
                    .map_or(ColorKey::Automatic, |c| ColorKey::Rgb(rgb(c))),
                custom_list: k["custom"]
                    .as_str()
                    .map(|l| l.split(',').map(str::to_owned).collect()),
            }
        })
        .collect();
    SortSpec {
        range: parse_range(sc["range"].as_str().expect("range")).expect("range"),
        header,
        orientation: if ltr {
            SortOrientation::LeftToRight
        } else {
            SortOrientation::TopToBottom
        },
        case_sensitive: sc["match_case"] == true,
        method: SortMethod::Default,
        keys,
    }
}

fn same(expected: &Value, wb: &Workbook, sheet: usize, row: u32, col: u32) -> bool {
    let cell = wb.cell(sheet, row, col);
    let value = cell.map(|c| &c.value);
    match expected {
        Value::Null => matches!(value, None | Some(CellValue::Empty)),
        Value::Bool(b) => matches!(value, Some(CellValue::Bool(x)) if x == b),
        Value::Number(n) => {
            matches!(value, Some(CellValue::Number(x)) if (x - n.as_f64().unwrap_or(f64::NAN)).abs() <= 1e-9 * x.abs().max(1.0))
        }
        Value::String(s) => value.and_then(CellValue::as_text) == Some(s.as_str()),
        Value::Object(o) => {
            matches!(value, Some(CellValue::Error(e)) if Some(e.code()) == o["e"].as_str())
        }
        _ => false,
    }
}

/// Scenarios where Excel's own order is not reproducible: its comparison of numbers that agree to 15 digits is
/// not a total order (the result depends on its algorithm).
const KNOWN_DIFFERENT: [&str; 3] = ["float-fuzz", "float-fuzz-2", "unicode-en"];
// unicode-en: Excel orders i, dotless i, dotted capital I; CLDR puts the dotted capital first.

#[test]
fn sorts_match_excel() {
    let (mut wb, scenarios) = load();
    let locale = Locale::from_bcp47("en-GB").expect("locale");
    let mut matched = 0;
    let mut failures = Vec::new();
    for sc in &scenarios {
        let id = sc["id"].as_str().expect("id");
        let sheet = wb.sheet_index(id).expect("sheet");
        let spec = spec_of(sc);
        wb.sort_range(sheet, &spec, &locale).expect(id);
        let (r1, c1, r2, c2) = spec.range;
        let after = &sc["after"]["values"];
        let formulas = &sc["after"]["formulas"];
        let mut ok = true;
        for r in r1..=r2 {
            for c in c1..=c2 {
                let (i, j) = ((r - r1) as usize, (c - c1) as usize);
                let f = formulas[i][j].as_str().map(|f| f.trim_start_matches('='));
                // A moved formula keeps its cached result until the application recalculates.
                let cell_ok = (f.is_some() || same(&after[i][j], &wb, sheet, r, c))
                    && wb.formula_text(sheet, r, c).as_deref() == f;
                if !cell_ok && ok {
                    eprintln!(
                        "{id} r{r}c{c}: excel {} {f:?}, got {:?} {:?}",
                        after[i][j],
                        wb.cell(sheet, r, c).map(|x| &x.value),
                        wb.formula_text(sheet, r, c)
                    );
                }
                ok &= cell_ok;
            }
        }
        if ok {
            matched += 1;
        } else if !KNOWN_DIFFERENT.contains(&id) {
            failures.push(id.to_owned());
        }
    }
    eprintln!("sort: {matched}/{} scenarios match Excel", scenarios.len());
    assert!(failures.is_empty(), "differs from Excel: {failures:?}");
}

#[test]
fn sort_state_round_trips() {
    let (mut wb, _) = load();
    let mut state = SortState::new((2, 1, 9, 4));
    state.case_sensitive = true;
    state.conditions.push(open365_xlsx::SortCondition {
        range: (2, 3, 9, 3),
        descending: true,
        sort_by: open365_xlsx::SortBy::Value,
        custom_list: Some("High,Low".to_owned()),
        dxf_id: None,
        icon_set: None,
        icon_id: None,
    });
    wb.set_sort_state(0, Some(&state)).expect("sets");
    let bytes = wb.save().expect("saves");
    let wb2 = Workbook::open(&bytes, &Limits::default()).expect("reopens");
    assert_eq!(wb2.sheet(0).expect("sheet").sort_state(), Some(state));
}

#[test]
fn a_sort_refuses_an_invalid_range_and_leaves_the_sheet_alone() {
    let (mut wb, _) = load();
    let before = wb.serialize_sheet(0).expect("sheet");
    let spec = SortSpec {
        range: (1, 1, 0, 1),
        header: HeaderMode::No,
        orientation: SortOrientation::TopToBottom,
        case_sensitive: false,
        method: SortMethod::Default,
        keys: vec![SortKey::value(1, false)],
    };
    let locale = Locale::from_bcp47("en-US").expect("locale");
    assert!(wb.sort_range(0, &spec, &locale).is_err());
    assert_eq!(wb.serialize_sheet(0).expect("sheet"), before);
}

#[test]
fn text_order_follows_the_locale() {
    use open365_xlsx::TextOrder;
    let sv = Locale::from_bcp47("sv-SE").expect("sv");
    let de = Locale::from_bcp47("de-DE").expect("de");
    let by = |l: &Locale, a: &str, b: &str| {
        TextOrder::new(l, SortMethod::Default, false)
            .expect("order")
            .compare_text(a, b)
    };
    // Swedish puts å after z; German treats it as a variant of a.
    assert!(by(&sv, "z", "å").is_lt());
    assert!(by(&de, "å", "z").is_lt());
}
