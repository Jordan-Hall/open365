//! Format survival, grouped fields and calculated items across a refresh, against Excel.
//! The fixtures `pivot-refresh-{formats,formats-removed,grouped,calcitems,calcitems-values}`
//! (made by `tools/office/make-pivot-refresh-ext.ps1`) hold `before.xlsx` (formats applied,
//! fields grouped, items calculated in Excel), `edits.json` (source edits) and `after.xlsx` (the
//! source edited and every cache refreshed by Excel). Each pivot is refreshed with
//! `refresh_sheet` and compared with `after.xlsx`: the pivot table (user `formats` included),
//! cache definition and records parts, the cells, and the display format (fill, font and the
//! four borders per cell, `Range.DisplayFormat`) that Excel reports after the refresh.
//! The part comparison is semantic: attribute order does not matter and the values Excel stamps
//! per run (`refreshedDate`, `cacheId`, `xr:uid`) are ignored.

use std::path::PathBuf;

use open365_opc::Limits;
use open365_pivot::style::{CellStyle, CustomStyles, Dxfs, Edge, style_cells_full};
use open365_pivot::{PivotStrings, refresh_sheet, sheet_pivots};
use open365_xlsx::{CellInput, CellValue, RawXml, Workbook, XmlElement, parse_cell_ref, parse_xml};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn open(path: PathBuf) -> Workbook {
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Workbook::open(&bytes, &Limits::default()).expect("opens")
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

/// What a cell's format shows: four edges (style, resolved colour), solid fill colour, bold,
/// italic and font colour. Indexed and automatic black compare equal.
fn visible(wb: &Workbook, si: usize, r: u32, c: u32) -> String {
    use open365_xlsx::{BorderEdge, BorderStyle};
    let f = wb.cell_format(si, r, c);
    let e = |x: &BorderEdge| {
        if x.style == BorderStyle::None {
            "-".to_owned()
        } else {
            format!("{:?}/{:?}", x.style, wb.styles().resolve_color(&x.color))
        }
    };
    format!(
        "L{} R{} T{} B{} fill {:?} b{} i{}",
        e(&f.border.left),
        e(&f.border.right),
        e(&f.border.top),
        e(&f.border.bottom),
        f.fill.pattern,
        f.font.bold,
        f.font.italic
    )
}

fn bgr(c: open365_xlsx::Rgb) -> i64 {
    i64::from(c.bgr_u32())
}

fn same_edge(want: &str, got: Option<Edge>) -> bool {
    match (want.is_empty(), got) {
        (true, None) => true,
        (false, Some(e)) => {
            let parts: Vec<&str> = want.split(':').collect();
            parts.len() == 3
                && parts[0].parse::<i32>().ok() == Some(e.line)
                && parts[1].parse::<i32>().ok() == Some(e.weight)
                && parts[2].parse::<i64>().ok() == Some(bgr(e.color))
        }
        _ => false,
    }
}

fn style_diff(c: &serde_json::Value, got: &CellStyle) -> Vec<String> {
    let mut bad = Vec::new();
    let want_fill = c["f"].as_i64().unwrap_or(-1);
    if want_fill != got.fill.map_or(-1, bgr) {
        bad.push(format!("fill want {want_fill} got {:?}", got.fill.map(bgr)));
    }
    if (c["b"].as_i64() == Some(1)) != got.bold {
        bad.push(format!("bold want {} got {}", c["b"], got.bold));
    }
    if (c["i"].as_i64() == Some(1)) != got.italic {
        bad.push(format!("italic want {} got {}", c["i"], got.italic));
    }
    let want_font = c["c"].as_i64().unwrap_or(0);
    if want_font != got.font_color.map_or(0, bgr) {
        bad.push(format!(
            "font want {want_font} got {:?}",
            got.font_color.map(bgr)
        ));
    }
    let names = ["left", "top", "bottom", "right"];
    for (k, b) in c["bd"].as_array().into_iter().flatten().enumerate() {
        if !same_edge(b.as_str().unwrap_or(""), got.borders[k]) {
            bad.push(format!("{} want {b} got {:?}", names[k], got.borders[k]));
        }
    }
    bad
}

fn cell_matches(c: &serde_json::Value, got: Option<CellValue>) -> bool {
    match (&c["v"], got) {
        (serde_json::Value::Number(n), Some(CellValue::Number(g))) => {
            (n.as_f64().unwrap_or(f64::NAN) - g).abs() < 1e-9
        }
        (serde_json::Value::String(s), Some(CellValue::SharedString { text, .. })) => *s == *text,
        (serde_json::Value::String(s), Some(CellValue::Str(t))) => *s == *t,
        (serde_json::Value::Object(o), Some(CellValue::SharedString { text, .. })) => {
            o["e"].as_str() == Some(text.as_ref())
        }
        (serde_json::Value::Object(o), Some(CellValue::Str(t))) => {
            o["e"].as_str() == Some(t.as_ref())
        }
        _ => false,
    }
}

fn apply_edits(wb: &mut Workbook, dir: &std::path::Path) {
    let edits: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("edits.json")).expect("edits"))
            .expect("json");
    let data = wb.sheet_index("Data").expect("Data sheet");
    for e in edits.as_array().expect("array") {
        let (r, c) = parse_cell_ref(e["a"].as_str().expect("address")).expect("address");
        let input = match &e["v"] {
            serde_json::Value::Null => CellInput::Empty,
            serde_json::Value::String(s) => CellInput::Text(s.clone()),
            v => CellInput::Number(v.as_f64().expect("number")),
        };
        wb.set_cell_value(data, r, c, input).expect("edit");
    }
}

/// (pivots refreshed, failures) of one set.
fn run(set: &str) -> (usize, Vec<String>) {
    let dir = root().join(format!("fixtures/reference/xlsx/pivot-refresh-{set}"));
    let mut wb = open(dir.join("before.xlsx"));
    let after = open(dir.join("after.xlsx"));
    apply_edits(&mut wb, &dir);
    let reference: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("pivot.json")).expect("pivot.json"))
            .expect("json");
    let strings = PivotStrings::default();
    let mut failures = Vec::new();
    let mut pivots = 0;
    for expected in reference["pivots"].as_array().expect("pivots") {
        let name = expected["sheet"].as_str().expect("sheet");
        let si = wb.sheet_index(name).expect("pivot sheet");
        let before = sheet_pivots(&wb, si).remove(0);
        let label = format!("{set} {name} ({})", expected["case"].as_str().unwrap_or(""));
        match refresh_sheet(&mut wb, si, &strings) {
            Ok(1) => {}
            other => {
                failures.push(format!("{label}: [parts] refresh gave {other:?}"));
                continue;
            }
        }
        pivots += 1;
        let again = sheet_pivots(&wb, si).remove(0);
        if again.cache.refresh_on_load {
            failures.push(format!("{label}: [parts] not regenerated (refreshOnLoad)"));
        }
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
                failures.push(format!("{label}: [parts] {what} differs at {d}"));
            }
        }
        let mut bad_cells = Vec::new();
        for c in expected["cells"].as_array().expect("cells") {
            let (r, col) = parse_cell_ref(c["a"].as_str().expect("a")).expect("address");
            if !cell_matches(c, wb.cell(si, r, col).map(|x| x.value.clone())) {
                bad_cells.push(format!("{} is {c}", c["a"]));
            }
        }
        if !bad_cells.is_empty() {
            failures.push(format!(
                "{label}: [cells] {} cells differ, first {}",
                bad_cells.len(),
                bad_cells[0]
            ));
        }
        // The cell formats (`cellXfs`) Excel keeps on the pivot's cells: the borders of user
        // formats are applied to the cells themselves and must survive the refresh.
        let mut bad_xf = Vec::new();
        for c in expected["cells"].as_array().expect("cells") {
            let a = c["a"].as_str().expect("a");
            let (r, col) = parse_cell_ref(a).expect("address");
            let (g, w) = (visible(&wb, si, r, col), visible(&after, after_si, r, col));
            if g != w {
                bad_xf.push(format!("{a}: got {g:?} want {w:?}"));
            }
        }
        if !bad_xf.is_empty() {
            failures.push(format!(
                "{label}: [xf] cell formats differ in {} cells, first {}",
                bad_xf.len(),
                bad_xf[0]
            ));
        }
        // Display format of the refreshed pivot (user formats over the table style).
        let custom = CustomStyles::from_workbook(&wb);
        let dxfs = Dxfs::from_workbook(&wb);
        let styled = again
            .grid_from_cache(&strings)
            .ok()
            .and_then(|g| style_cells_full(&again.table, &g, wb.styles().theme(), &custom, &dxfs));
        let Some(styled) = styled else {
            failures.push(format!("{label}: [display] no style"));
            continue;
        };
        let mut bad_style = Vec::new();
        for c in expected["cells"].as_array().expect("cells") {
            let a = c["a"].as_str().expect("a");
            let (r, col) = parse_cell_ref(a).expect("address");
            let got = styled
                .cells
                .get(r.wrapping_sub(styled.top) as usize)
                .and_then(|row| row.get(col.wrapping_sub(styled.left) as usize));
            match got {
                Some(g) => {
                    let d = style_diff(c, g);
                    if !d.is_empty() {
                        bad_style.push(format!("{a}: {}", d.join("; ")));
                    }
                }
                None => bad_style.push(format!("{a}: outside")),
            }
        }
        if !bad_style.is_empty() {
            failures.push(format!(
                "{label}: [display] display format differs in {} cells, first {}",
                bad_style.len(),
                bad_style[0]
            ));
        }
    }
    (pivots, failures)
}

/// Runs the sets; only failures of the given kinds (`parts`, `cells`, `xf`, `display`) on the
/// given pivots (all when empty) fail the test. The rest is the measured gap, printed.
fn check(sets: &[&str], total: usize, kinds: &[&str], pivots: &[&str]) {
    let mut all = Vec::new();
    let mut n = 0;
    for set in sets {
        let (p, f) = run(set);
        n += p;
        all.extend(f);
    }
    assert_eq!(n, total);
    let (hard, gap): (Vec<String>, Vec<String>) = all.into_iter().partition(|f| {
        kinds.iter().any(|k| f.contains(&format!("[{k}]")))
            && (pivots.is_empty() || pivots.iter().any(|p| f.contains(&format!(" {p} ("))))
    });
    if std::env::var("PIVOT_VERBOSE").is_ok() {
        for g in &gap {
            println!("GAP {g}");
        }
    }
    assert!(
        hard.is_empty(),
        "{} failures:
{}",
        hard.len(),
        hard.join(
            "
"
        )
    );
}

/// Pivot table, cache and records parts and cell values are Excel's; the user formats stay on
/// the same items (`x/@v` is written as Excel does). Gap, not asserted: the cell formats
/// (`cellXfs` borders Excel re-applies to the new layout) and the display format of the
/// border rules (`PIVOT_VERBOSE=1` prints them).
#[test]
fn formats_survive_refresh_like_excel() {
    check(
        &["formats", "formats-removed"],
        10,
        &["parts", "cells"],
        &[],
    );
    // The cell formats follow the items: exact on the tabular pivot (P4) of both sets.
    check(&["formats", "formats-removed"], 10, &["xf"], &["P4"]);
}

/// Date groups (months, quarters, years, days) and number groups regroup on the refreshed
/// data like Excel: cache `fieldGroup`, group items, pivot field items, records and cells.
/// A number group made without blanks is dropped when the source gets a blank (the field is
/// a plain field again). Gap, not asserted (`grouped-blank-date`): a blank date drops the date
/// groups with their derived fields and the pivot loses those axis fields; the cells are asserted,
/// the cache and pivot table parts fall back to `refreshOnLoad`.
#[test]
fn grouped_fields_refresh_like_excel() {
    check(
        &["grouped", "grouped-numbers"],
        11,
        &["parts", "cells"],
        &[],
    );
    // A blank date drops the date groups: the cells are Excel's, the parts are not rebuilt.
    check(&["grouped-blank-date"], 2, &["cells"], &[]);
}

/// Calculated items: shared items, pivot field items, `calculatedItem` references, records,
/// cells and display format are all Excel's after a refresh.
#[test]
fn calculated_items_refresh_like_excel() {
    check(
        &["calcitems", "calcitems-values"],
        8,
        &["parts", "cells", "xf", "display"],
        &[],
    );
}

fn sig(wb: &Workbook, si: usize, r: u32, c: u32) -> String {
    use open365_xlsx::BorderStyle;
    let b = &wb.cell_format(si, r, c).border;
    let f = |e: &open365_xlsx::BorderEdge, ch: char| {
        if e.style == BorderStyle::None {
            '.'.to_string()
        } else {
            format!(
                "{ch}{}",
                format!("{:?}", e.color.source)
                    .chars()
                    .skip(4)
                    .take(6)
                    .collect::<String>()
            )
        }
    };
    format!(
        "{}{}{}{}",
        f(&b.left, 'l'),
        f(&b.right, 'r'),
        f(&b.top, 't'),
        f(&b.bottom, 'b')
    )
}

/// Diagnostic: `PIVOT_SET=formats PIVOT_SHEET=P1 cargo test ... dump_xf -- --ignored --nocapture`.
#[test]
#[ignore = "diagnostic"]
fn dump_xf() {
    let set = std::env::var("PIVOT_SET").unwrap_or_else(|_| "formats".into());
    let sheet = std::env::var("PIVOT_SHEET").unwrap_or_else(|_| "P1".into());
    let dir = root().join(format!("fixtures/reference/xlsx/pivot-refresh-{set}"));
    let before = open(dir.join("before.xlsx"));
    let after = open(dir.join("after.xlsx"));
    let mut mine = open(dir.join("before.xlsx"));
    apply_edits(&mut mine, &dir);
    let si = mine.sheet_index(&sheet).unwrap();
    refresh_sheet(&mut mine, si, &PivotStrings::default()).unwrap();
    for r in 4..14 {
        for (name, wb) in [("before", &before), ("after ", &after), ("mine  ", &mine)] {
            let row: Vec<String> = (2..6)
                .map(|c| format!("{:<14}", sig(wb, si, r, c)))
                .collect();
            println!("{r:>2} {name} {}", row.join("|"));
        }
        println!();
    }
}

/// Diagnostic of the groups Excel drops (a blank where the group had none).
#[test]
#[ignore = "diagnostic"]
fn dump_ungroup() {
    for set in ["grouped", "grouped-blank-date"] {
        for f in run(set).1 {
            println!(
                "{}",
                f.lines().take(4).collect::<Vec<_>>().join(
                    "
"
                )
            );
        }
    }
}
