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
