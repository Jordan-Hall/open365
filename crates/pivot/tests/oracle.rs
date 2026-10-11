//! Computed pivot grids against Excel's own: `fixtures/reference/xlsx/pivot-*/pivot.json`
//! holds every non-empty cell of each PivotTable as Excel refreshed it. In-sample sets were
//! used to fit the engine; `holdout-*` sets were generated from random layouts and not
//! looked at while fitting.

use std::collections::HashMap;
use std::path::PathBuf;

use open365_opc::Limits;
use open365_pivot::{GridValue, OrderMode, PivotStrings, sheet_pivots};
use open365_xlsx::{Workbook, cell_ref_string};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn open(name: &str) -> Workbook {
    let path = root().join(format!("fixtures/xlsx/pivot-{name}.xlsx"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Workbook::open(&bytes, &Limits::default()).expect("opens")
}

fn reference(name: &str) -> serde_json::Value {
    let path = root().join(format!("fixtures/reference/xlsx/{name}/pivot.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("json")
}

fn same(expected: &serde_json::Value, got: &GridValue) -> bool {
    match (expected, got) {
        (serde_json::Value::String(a), GridValue::Text(b)) => a == b,
        (serde_json::Value::Number(a), GridValue::Number(b)) => {
            let a = a.as_f64().unwrap_or(f64::NAN);
            (a - b).abs() <= 1e-9 * a.abs().max(1.0)
        }
        (serde_json::Value::Object(o), GridValue::Error(e)) => o["e"].as_str() == Some(e.as_str()),
        _ => false,
    }
}

/// (pivots matching, pivots total, cells matching, cells total) of one fixture workbook.
fn run(name: &str, mode: OrderMode) -> (usize, usize, usize, usize) {
    let wb = open(name);
    let refs = reference(&format!("pivot-{name}"));
    let strings = PivotStrings::default();
    let (mut pok, mut ptotal, mut cok, mut ctotal) = (0, 0, 0, 0);
    for p in refs["pivots"].as_array().expect("pivots") {
        let sheet = p["sheet"].as_str().expect("sheet");
        let si = wb.sheet_index(sheet).expect("sheet exists");
        let pivot = sheet_pivots(&wb, si).into_iter().next().expect("pivot");
        let grid = match mode {
            OrderMode::Stored => pivot.grid_from_cache(&strings),
            OrderMode::Refresh => pivot.grid_refreshed(&wb, &strings),
        };
        ptotal += 1;
        let Ok(grid) = grid else {
            ctotal += p["cells"].as_array().map_or(0, Vec::len);
            println!("{name}/{sheet} {}: {:?}", p["case"], grid.err());
            continue;
        };
        let mut got: HashMap<String, &GridValue> = grid
            .cells
            .iter()
            .map(|c| (cell_ref_string(c.row, c.col), &c.value))
            .collect();
        let mut bad = Vec::new();
        for c in p["cells"].as_array().expect("cells") {
            ctotal += 1;
            let a = c["a"].as_str().expect("a");
            match got.remove(a) {
                Some(g) if same(&c["v"], g) => cok += 1,
                Some(g) => bad.push(format!("{a}: want {} got {g:?}", c["v"])),
                None => bad.push(format!("{a}: want {} got nothing", c["v"])),
            }
        }
        for (a, g) in &got {
            bad.push(format!("{a}: unexpected {g:?}"));
        }
        if bad.is_empty() {
            pok += 1;
        } else if std::env::var_os("PIVOT_VERBOSE").is_some() {
            println!("{name}/{sheet} {} ({} diffs)", p["case"], bad.len());
            let cap: usize = std::env::var("PIVOT_VERBOSE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(6);
            for b in bad.iter().take(cap) {
                println!("    {b}");
            }
        }
    }
    println!("{name} {mode:?}: pivots {pok}/{ptotal}, cells {cok}/{ctotal}");
    (pok, ptotal, cok, ctotal)
}

#[test]
fn stored_cache_reproduces_excel_output() {
    for (name, floor) in FLOORS {
        let (pok, ..) = run(name, OrderMode::Stored);
        assert!(pok >= floor, "{name}: {pok} pivots match, floor {floor}");
    }
}

/// Pivots (per fixture workbook) that match Excel cell for cell. `showas` and `grouping` are
/// not computed yet and are absent on purpose. Raise a floor when the engine improves.
const FLOORS: [(&str, usize); 9] = [
    ("grouping", 17),
    ("holdout-showas", 39),
    ("showas", 21),
    ("values", 48),
    ("layouts", 30),
    ("aggregates", 26),
    ("subtotals", 28),
    ("holdout-a", 14),
    ("holdout-b", 14),
];

#[test]
fn refreshed_source_reproduces_excel_output() {
    for (name, floor) in FLOORS {
        let (pok, ..) = run(name, OrderMode::Refresh);
        assert!(
            pok >= floor,
            "{name}: {pok} pivots match after refresh, floor {floor}"
        );
    }
}

#[test]
fn untouched_parts_round_trip_byte_identical() {
    use open365_pivot::{CacheDefinition, CacheRecords, PivotTable};
    for name in [
        "values",
        "holdout-showas",
        "layouts",
        "aggregates",
        "subtotals",
        "showas",
        "grouping",
        "holdout-a",
        "holdout-b",
    ] {
        let wb = open(name);
        let mut seen = 0;
        for part in wb.package().parts() {
            let n = part.name().as_str();
            let bytes = part.data();
            let again = if n.starts_with("xl/pivotTables/pivotTable") && n.ends_with(".xml") {
                PivotTable::parse(bytes).expect("table").to_bytes()
            } else if n.starts_with("xl/pivotCache/pivotCacheDefinition") && n.ends_with(".xml") {
                CacheDefinition::parse(bytes, false)
                    .expect("cache")
                    .to_bytes()
            } else if n.starts_with("xl/pivotCache/pivotCacheRecords") && n.ends_with(".xml") {
                CacheRecords::parse(bytes, false)
                    .expect("records")
                    .to_bytes()
            } else {
                continue;
            };
            assert!(
                again == bytes,
                "{name}: {n} does not round-trip byte-identically"
            );
            seen += 1;
        }
        assert!(seen >= 3, "{name}: pivot parts found");
    }
}

/// `location` (ref, firstHeaderRow, firstDataRow, firstDataCol, page counts) of the computed
/// grid against the values Excel stored in the pivot part.
#[test]
fn location_matches_what_excel_wrote() {
    let (mut ok, mut total) = (0, 0);
    for (name, _) in FLOORS {
        let wb = open(name);
        let refs = reference(&format!("pivot-{name}"));
        for p in refs["pivots"].as_array().expect("pivots") {
            let sheet = p["sheet"].as_str().expect("sheet");
            let si = wb.sheet_index(sheet).expect("sheet exists");
            let pivot = sheet_pivots(&wb, si).into_iter().next().expect("pivot");
            let grid = pivot
                .grid_from_cache(&PivotStrings::default())
                .expect("grid");
            let t = &pivot.table;
            let stored = open365_pivot::Location {
                reference: t.reference.clone(),
                first_header_row: t.first_header_row,
                first_data_row: t.first_data_row,
                first_data_col: t.first_data_col,
                row_page_count: t.row_page_count,
                col_page_count: t.col_page_count,
            };
            total += 1;
            // Writing the stored location back leaves the part byte-identical.
            let mut again = t.clone();
            again.set_location(&stored);
            assert!(
                again.to_bytes() == t.to_bytes(),
                "{name}/{sheet}: set_location changed bytes"
            );
            if grid.location == stored {
                ok += 1;
            } else {
                println!(
                    "{name}/{sheet} {}: want {stored:?} got {:?}",
                    p["case"], grid.location
                );
            }
        }
    }
    println!("location: {ok}/{total}");
    assert_eq!(ok, total);
}

/// Calculated fields (`calc` set) and GETPIVOTDATA (`getpivotdata` set) against Excel.
#[test]
fn calculated_fields_and_getpivotdata_match_excel() {
    use open365_pivot::{PivotIndex, Value, get_pivot_data};
    let (pok, ptotal, ..) = run("calc", OrderMode::Stored);
    println!("calc stored: {pok}/{ptotal}");
    assert!(pok >= CALC_FLOOR, "calc: {pok} pivots, floor {CALC_FLOOR}");
    let (pok, ptotal, ..) = run("calc", OrderMode::Refresh);
    println!("calc refreshed: {pok}/{ptotal}");
    assert!(
        pok >= CALC_FLOOR,
        "calc refresh: {pok} pivots, floor {CALC_FLOOR}"
    );
    let wb = open("getpivotdata");
    let refs = reference("pivot-getpivotdata");
    let (mut ok, mut total) = (0, 0);
    for p in refs["pivots"].as_array().expect("pivots") {
        let si = wb
            .sheet_index(p["sheet"].as_str().expect("sheet"))
            .expect("sheet");
        let pivot = sheet_pivots(&wb, si).into_iter().next().expect("pivot");
        let index = PivotIndex::build(&pivot, &PivotStrings::default()).expect("index");
        for q in p["gpd"].as_array().into_iter().flatten() {
            let f = q["f"].as_str().expect("formula");
            let Some((field, pairs)) = parse_gpd(f) else {
                continue;
            };
            total += 1;
            let got = get_pivot_data(&index, &field, &pairs);
            let good = match (&q["v"], &got) {
                (serde_json::Value::Object(o), Err(e)) => o["e"].as_str() == Some(*e),
                (v, Ok(g)) => same(v, g),
                _ => false,
            };
            if good {
                ok += 1;
            } else {
                println!(
                    "getpivotdata {}: {f}: want {} got {got:?}",
                    p["case"], q["v"]
                );
            }
            let _: &[(String, Value)] = &pairs;
        }
    }
    println!("getpivotdata: {ok}/{total}");
    assert!(
        ok >= GPD_FLOOR,
        "getpivotdata: {ok}/{total}, floor {GPD_FLOOR}"
    );
}

/// The six calculated-field pivots and the nine calculated-item pivots.
const CALC_FLOOR: usize = 15;
const GPD_FLOOR: usize = 36;

/// `=GETPIVOTDATA("Sum of Units",$B$4,"Region","East",...)` into (data field, pairs).
fn parse_gpd(f: &str) -> Option<(String, Vec<(String, open365_pivot::Value)>)> {
    let inner = f.strip_prefix("=GETPIVOTDATA(")?.strip_suffix(')')?;
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in inner.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            ',' if !quoted => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    let text = |s: &str| s.trim_matches('"').to_owned();
    if parts.len() < 2 || !parts.len().is_multiple_of(2) || !parts[0].starts_with('"') {
        return None;
    }
    let pairs = parts[2..]
        .chunks(2)
        .map(|c| {
            let item = if c[1].starts_with('"') {
                open365_pivot::Value::Text(text(&c[1]))
            } else {
                open365_pivot::Value::Number(c[1].parse().unwrap_or(f64::NAN))
            };
            (text(&c[0]), item)
        })
        .collect();
    Some((text(&parts[0]), pairs))
}

/// Excel refuses Average, StdDev and Var data fields while a table has calculated items, so the
/// engine does too instead of inventing a value.
#[test]
fn calculated_items_refuse_average() {
    let wb = open("calc");
    let si = wb.sheet_index("P7").expect("sheet");
    let mut pivot = sheet_pivots(&wb, si).into_iter().next().expect("pivot");
    assert!(pivot.grid_from_cache(&PivotStrings::default()).is_ok());
    pivot.table.data_fields[0].func = open365_pivot::Func::Average;
    assert!(pivot.grid_from_cache(&PivotStrings::default()).is_err());
}

/// Hold-out for calculated items and fields: random formulas, layouts and data fields over a data
/// seed no rule was fitted on (written once; failures are not inspected to tune). Pivots that
/// match Excel cell for cell, from the stored cache and from the refreshed source.
#[test]
fn calculated_items_holdout_matches_excel() {
    let path = root().join("fixtures/xlsx/pivot-holdout-calc.xlsx");
    if !path.exists() {
        println!("holdout-calc: fixture not generated yet, skipped");
        return;
    }
    let (pok, ptotal, cok, ctotal) = run("holdout-calc", OrderMode::Stored);
    println!("holdout-calc stored: pivots {pok}/{ptotal}, cells {cok}/{ctotal}");
    assert!(
        pok >= HOLDOUT_CALC_FLOOR,
        "holdout-calc: {pok} pivots, floor {HOLDOUT_CALC_FLOOR}"
    );
    let (pok, ptotal, ..) = run("holdout-calc", OrderMode::Refresh);
    println!("holdout-calc refreshed: {pok}/{ptotal}");
    assert!(
        pok >= HOLDOUT_CALC_FLOOR,
        "holdout-calc refresh: {pok} pivots, floor {HOLDOUT_CALC_FLOOR}"
    );
}

/// Measured on the first run of the hold-out; raise when the engine improves.
const HOLDOUT_CALC_FLOOR: usize = 38;

/// Second in-sample round of calculated items (`calc2`: division, Count/Max/Min data fields,
/// items on inner levels, fields beside items), used to fit; pivots that match Excel cell for cell.
#[test]
fn calculated_items_second_round_matches_excel() {
    let path = root().join("fixtures/xlsx/pivot-calc2.xlsx");
    if !path.exists() {
        println!("calc2: fixture not generated yet, skipped");
        return;
    }
    let (pok, ptotal, cok, ctotal) = run("calc2", OrderMode::Stored);
    println!("calc2 stored: pivots {pok}/{ptotal}, cells {cok}/{ctotal}");
    assert!(
        pok >= CALC2_FLOOR,
        "calc2: {pok} pivots, floor {CALC2_FLOOR}"
    );
    let (pok, ptotal, ..) = run("calc2", OrderMode::Refresh);
    println!("calc2 refreshed: {pok}/{ptotal}");
    assert!(
        pok >= CALC2_FLOOR,
        "calc2 refresh: {pok} pivots, floor {CALC2_FLOOR}"
    );
}

const CALC2_FLOOR: usize = 24;
