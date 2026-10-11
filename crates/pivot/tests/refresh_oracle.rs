//! Refresh regeneration against Excel: the source of each `pivot-refresh-*` fixture is edited
//! (`edits.json`), every pivot is refreshed with `refresh_sheet`, and the pivot table, cache
//! definition and cache records are compared with the parts Excel saved after refreshing the
//! same edits (`after.xlsx`). The comparison is semantic: attribute order does not matter and the
//! values Excel stamps per run (`refreshedDate`, `cacheId`, `xr:uid`) are ignored.

use std::path::PathBuf;

use open365_opc::Limits;
use open365_pivot::{PivotStrings, refresh_sheet, sheet_pivots};
use open365_xlsx::{CellInput, CellValue, RawXml, Workbook, XmlElement, parse_xml};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn open(path: PathBuf) -> Workbook {
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Workbook::open(&bytes, &Limits::default()).expect("opens")
}

/// `A5` as (row, column), 1-based.
fn cell_ref(a: &str) -> (u32, u32) {
    let split = a.find(|c: char| c.is_ascii_digit()).expect("row digits");
    let col = a[..split]
        .bytes()
        .fold(0u32, |n, b| n * 26 + u32::from(b - b'A') + 1);
    (a[split..].parse().expect("row"), col)
}

const IGNORED: [&str; 4] = ["refreshedDate", "cacheId", "xr:uid", "refreshedBy"];

fn canon_attr(name: &str, value: &str) -> String {
    match value.parse::<f64>() {
        Ok(n) if value.bytes().next().is_some_and(|b| b != b'+') => format!("{name}={n}"),
        _ => format!("{name}={value}"),
    }
}

fn canon(e: &XmlElement, out: &mut Vec<String>, path: &str) {
    let here = format!("{path}/{}", e.head.local_name());
    let mut attrs: Vec<String> = e
        .head
        .attrs
        .iter()
        .filter(|a| !IGNORED.contains(&a.name.as_str()))
        .map(|a| canon_attr(&a.name, &a.value))
        .collect();
    attrs.sort();
    out.push(format!("{here} [{}]", attrs.join(" ")));
    for c in &e.children {
        match c {
            RawXml::Element(c) => canon(c, out, &here),
            RawXml::Text(t) if !t.trim().is_empty() => out.push(format!("{here} text {t}")),
            _ => {}
        }
    }
}

fn lines(bytes: &[u8]) -> Vec<String> {
    let xml = parse_xml(bytes).expect("xml");
    let mut out = Vec::new();
    canon(&xml.root, &mut out, "");
    out
}

fn part(wb: &Workbook, name: &str) -> Vec<u8> {
    wb.package()
        .part(name)
        .unwrap_or_else(|| panic!("part {name}"))
        .data()
        .to_vec()
}

fn first_difference(got: &[String], want: &[String]) -> Option<String> {
    for i in 0..got.len().max(want.len()) {
        if got.get(i) != want.get(i) {
            return Some(format!(
                "line {i}\n  got : {}\n  want: {}\n  (before: {:?})",
                got.get(i).map_or("<end>", String::as_str),
                want.get(i).map_or("<end>", String::as_str),
                &want[i.saturating_sub(2)..i.min(want.len())]
            ));
        }
    }
    None
}

fn run(set: &str) -> (usize, Vec<String>) {
    let dir = root().join(format!("fixtures/reference/xlsx/pivot-refresh-{set}"));
    let mut wb = open(root().join(format!("fixtures/xlsx/pivot-refresh-{set}.xlsx")));
    let after = open(dir.join("after.xlsx"));
    let edits: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("edits.json")).expect("edits"))
            .expect("json");
    let data = wb.sheet_index("Data").expect("Data sheet");
    for e in edits.as_array().expect("array") {
        let (r, c) = cell_ref(e["a"].as_str().expect("address"));
        let input = match &e["v"] {
            serde_json::Value::Null => CellInput::Empty,
            serde_json::Value::String(s) => CellInput::Text(s.clone()),
            v => CellInput::Number(v.as_f64().expect("number")),
        };
        wb.set_cell_value(data, r, c, input).expect("edit");
    }
    let reference: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("pivot.json")).expect("pivot.json"))
            .expect("json");
    let strings = PivotStrings::default();
    let mut failures = Vec::new();
    let mut pivots = 0;
    for (k, expected) in reference["pivots"]
        .as_array()
        .expect("pivots")
        .iter()
        .enumerate()
    {
        let name = expected["sheet"].as_str().expect("sheet");
        let si = wb.sheet_index(name).expect("pivot sheet");
        let before = sheet_pivots(&wb, si).remove(0);
        assert_eq!(refresh_sheet(&mut wb, si, &strings).expect("refresh"), 1);
        pivots += 1;
        let label = format!("{set} {name} ({})", expected["case"].as_str().unwrap_or(""));
        // Parts.
        let again = sheet_pivots(&wb, si).remove(0);
        assert!(
            !again.cache.refresh_on_load,
            "{label}: regenerated, no refreshOnLoad"
        );
        let after_si = after.sheet_index(name).expect("after sheet");
        let after_pivot = sheet_pivots(&after, after_si).remove(0);
        let parts = [
            ("table", before.part.as_str(), after_pivot.part.as_str()),
            (
                "cache",
                before.cache_part.as_str(),
                after_pivot.cache_part.as_str(),
            ),
            (
                "records",
                before.records_part.as_ref().expect("records").as_str(),
                after_pivot.records_part.as_ref().expect("records").as_str(),
            ),
        ];
        for (what, mine, theirs) in parts {
            let got = lines(&part(&wb, mine));
            let want = lines(&part(&after, theirs));
            if let Some(d) = first_difference(&got, &want) {
                failures.push(format!("{label}: {what} differs at {d}"));
            }
        }
        // The regenerated cache alone (stored item order) gives the same grid as the refresh.
        let stored = again
            .grid_from_cache(&strings)
            .expect("grid from the new cache");
        let fresh = again.grid_refreshed(&wb, &strings).expect("refreshed grid");
        if stored.cells != fresh.cells {
            failures.push(format!(
                "{label}: stored-cache grid differs from the refreshed grid"
            ));
        }
        // Cells.
        for c in expected["cells"].as_array().expect("cells") {
            let (r, col) = cell_ref(c["a"].as_str().expect("a"));
            let got = wb.cell(si, r, col).map(|x| x.value.clone());
            let ok = match (&c["v"], got) {
                (serde_json::Value::Number(n), Some(CellValue::Number(g))) => {
                    (n.as_f64().unwrap_or(f64::NAN) - g).abs() < 1e-9
                }
                (serde_json::Value::String(s), Some(CellValue::SharedString { text, .. })) => {
                    *s == *text
                }
                (serde_json::Value::String(s), Some(CellValue::Str(t))) => *s == *t,
                (serde_json::Value::Object(o), Some(CellValue::SharedString { text, .. })) => {
                    o["e"].as_str() == Some(text.as_ref())
                }
                (serde_json::Value::Object(o), Some(CellValue::Str(t))) => {
                    o["e"].as_str() == Some(t.as_ref())
                }
                _ => false,
            };
            if !ok {
                failures.push(format!("{label}: cell {} is {c}", c["a"]));
                break;
            }
        }
        let _ = k;
    }
    (pivots, failures)
}

#[test]
fn refresh_regeneration_matches_excel() {
    let mut all = Vec::new();
    let mut total = 0;
    for set in ["none", "values", "items", "removed"] {
        let (n, f) = run(set);
        total += n;
        all.extend(f);
    }
    assert_eq!(total, 28);
    assert!(
        all.is_empty(),
        "{} failures:\n{}",
        all.len(),
        all.join("\n")
    );
}
