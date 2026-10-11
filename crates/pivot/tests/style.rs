//! Table style formatting against what Excel shows (`Range.DisplayFormat` of every cell of
//! `TableRange2`): fixtures `pivot-styles` (in-sample, used to fit the element rules) and
//! `pivot-holdout-styles` (random styles, flags and layouts, not looked at while fitting).

use std::path::PathBuf;

use open365_opc::Limits;
use open365_pivot::style::{CellStyle, CustomStyles, style_cells_with};
use open365_pivot::{PivotStrings, sheet_pivots};
use open365_xlsx::{Workbook, cell_ref_string, parse_cell_ref};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bgr(c: open365_xlsx::Rgb) -> i64 {
    i64::from(c.bgr_u32())
}

/// Reference border `"<line>:<weight>:<bgr>"` or empty.
fn same_edge(want: &str, got: Option<open365_pivot::style::Edge>) -> bool {
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

fn diff(c: &serde_json::Value, got: &CellStyle) -> Vec<String> {
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

/// (pivots matching, pivots, cells matching, cells) of one fixture.
fn run(name: &str) -> (usize, usize, usize, usize) {
    let path = root().join(format!("fixtures/xlsx/pivot-{name}.xlsx"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    let rpath = root().join(format!("fixtures/reference/xlsx/pivot-{name}/pivot.json"));
    let refs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&rpath).expect("reference")).expect("json");
    let custom = CustomStyles::from_workbook(&wb);
    let (mut pok, mut ptotal, mut cok, mut ctotal) = (0, 0, 0, 0);
    let verbose = std::env::var("PIVOT_VERBOSE").ok();
    for p in refs["pivots"].as_array().expect("pivots") {
        let sheet = p["sheet"].as_str().expect("sheet");
        let si = wb.sheet_index(sheet).expect("sheet");
        let pivot = sheet_pivots(&wb, si).into_iter().next().expect("pivot");
        let grid = pivot
            .grid_from_cache(&PivotStrings::default())
            .expect("grid");
        let styled = style_cells_with(&pivot.table, &grid, wb.styles().theme(), &custom);
        ptotal += 1;
        let Some(styled) = styled else {
            ctotal += p["cells"].as_array().map_or(0, Vec::len);
            println!("{name}/{sheet} {}: no style", p["style"]);
            continue;
        };
        let mut bad = Vec::new();
        for c in p["cells"].as_array().expect("cells") {
            ctotal += 1;
            let a = c["a"].as_str().expect("a");
            let (r, col) = parse_cell_ref(a).expect("address");
            let got = styled
                .cells
                .get((r - styled.top) as usize)
                .and_then(|row| row.get((col - styled.left) as usize));
            let Some(got) = got else {
                bad.push(format!("{}: outside", cell_ref_string(r, col)));
                continue;
            };
            let d = diff(c, got);
            if d.is_empty() {
                cok += 1;
            } else {
                bad.push(format!("{a}: {}", d.join("; ")));
            }
        }
        if std::env::var("PIVOT_DUMP").is_ok_and(|d| d == sheet) {
            dump(p, &styled, &grid);
        }
        if bad.is_empty() {
            pok += 1;
        } else if let Some(v) = &verbose {
            let cap: usize = v.parse().unwrap_or(6);
            println!(
                "{name}/{sheet} {} {} {} ({} diffs)",
                p["case"],
                p["style"],
                p["flags"],
                bad.len()
            );
            for b in bad.iter().take(cap) {
                println!("    {b}");
            }
        }
    }
    println!("{name}: pivots {pok}/{ptotal}, cells {cok}/{ctotal}");
    (pok, ptotal, cok, ctotal)
}

#[test]
fn styles_match_excel_display_format() {
    for (name, floor) in [
        ("styles", FLOOR_IN),
        ("styles2", FLOOR_IN2),
        ("holdout-styles", FLOOR_OUT),
        ("customstyles", FLOOR_CUSTOM),
        ("holdout-customstyles", FLOOR_CUSTOM_OUT),
    ] {
        // `PIVOT_SET=styles` runs (and prints diffs of) one set only: look at the in-sample set
        // while fitting and run the hold-out set for confirmation, never to tune.
        if std::env::var("PIVOT_SET").is_ok_and(|v| v != name) {
            continue;
        }
        if !root()
            .join(format!("fixtures/xlsx/pivot-{name}.xlsx"))
            .exists()
        {
            println!("{name}: fixture not generated yet, skipped");
            continue;
        }
        let (pok, ..) = run(name);
        assert!(pok >= floor, "{name}: {pok} pivots match, floor {floor}");
    }
}

/// Pivots (per set) that match Excel cell for cell. Raise a floor when the engine improves.
const FLOOR_IN: usize = 40;
const FLOOR_IN2: usize = 62;
const FLOOR_OUT: usize = 38;
const FLOOR_CUSTOM: usize = 36;
const FLOOR_CUSTOM_OUT: usize = 27;

/// `PIVOT_DUMP=P3`: Excel's and our formatting of one pivot as two character maps. Per cell:
/// fill letter, `B`/`I`, font letter, then four border digits (left, top, bottom, right).
fn dump(
    p: &serde_json::Value,
    styled: &open365_pivot::style::StyledGrid,
    grid: &open365_pivot::PivotGrid,
) {
    use std::collections::BTreeMap;
    let mut fills: BTreeMap<i64, char> = BTreeMap::new();
    let mut edges: BTreeMap<String, char> = BTreeMap::new();
    let mut fonts: BTreeMap<i64, char> = BTreeMap::new();
    let mut want: BTreeMap<(u32, u32), String> = BTreeMap::new();
    let mut got: BTreeMap<(u32, u32), String> = BTreeMap::new();
    fn code<K: Ord>(m: &mut BTreeMap<K, char>, k: K, base: u8) -> char {
        let n = m.len();
        *m.entry(k).or_insert((base + n as u8) as char)
    }
    let mut render = |f: i64, b: bool, i: bool, c: i64, bd: [String; 4]| -> String {
        let mut o = String::new();
        o.push(if f < 0 {
            '.'
        } else {
            code(&mut fills, f, b'a')
        });
        o.push(if b { 'B' } else { '-' });
        o.push(if i { 'I' } else { '-' });
        o.push(if c == 0 {
            '.'
        } else {
            code(&mut fonts, c, b'x')
        });
        for e in bd {
            o.push(if e.is_empty() {
                '.'
            } else {
                code(&mut edges, e, b'1')
            });
        }
        o
    };
    for c in p["cells"].as_array().expect("cells") {
        let a = c["a"].as_str().expect("a");
        let (r, col) = parse_cell_ref(a).expect("address");
        let bd = |v: &serde_json::Value| -> [String; 4] {
            std::array::from_fn(|k| v[k].as_str().unwrap_or("").to_owned())
        };
        want.insert(
            (r, col),
            render(
                c["f"].as_i64().unwrap_or(-1),
                c["b"].as_i64() == Some(1),
                c["i"].as_i64() == Some(1),
                c["c"].as_i64().unwrap_or(0),
                bd(&c["bd"]),
            ),
        );
        if let Some(g) = styled
            .cells
            .get((r - styled.top) as usize)
            .and_then(|row| row.get((col - styled.left) as usize))
        {
            let gb: [String; 4] = std::array::from_fn(|k| {
                g.borders[k].map_or(String::new(), |e| {
                    format!("{}:{}:{}", e.line, e.weight, bgr(e.color))
                })
            });
            got.insert(
                (r, col),
                render(
                    g.fill.map_or(-1, bgr),
                    g.bold,
                    g.italic,
                    g.font_color.map_or(0, bgr),
                    gb,
                ),
            );
        }
    }
    let cl: Vec<String> = grid
        .col_lines
        .iter()
        .map(|l| format!("{:?}{}{}", l.kind, l.level, if l.leaf { "L" } else { "" }))
        .collect();
    println!(
        "cols {cl:?} label_cols {} hdr {}",
        grid.label_cols, grid.header_rows
    );
    println!(
        "fills {fills:?}
fonts {fonts:?}
edges {edges:?}"
    );
    let rows: Vec<u32> = want
        .keys()
        .map(|k| k.0)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let cols: Vec<u32> = want
        .keys()
        .map(|k| k.1)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    for r in rows {
        let line = |m: &BTreeMap<(u32, u32), String>| -> String {
            cols.iter()
                .map(|c| {
                    m.get(&(r, *c))
                        .cloned()
                        .unwrap_or_else(|| "        ".into())
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let (w, g) = (line(&want), line(&got));
        let info = (r - styled.top)
            .checked_sub((grid.body.0 - grid.range.0) + grid.header_rows as u32)
            .and_then(|i| grid.row_lines.get(i as usize))
            .map_or_else(
                || "hdr".to_owned(),
                |l| format!("{:?}{}{}", l.kind, l.level, if l.leaf { "L" } else { "" }),
            );
        println!("{r:>3} {info:<12} want {w}");
        if w != g {
            println!("    {:<12} got  {g}", "");
        }
    }
}

/// `cargo test --test style edge_conflicts -- --ignored --nocapture` (set `PIVOT_SET`): for every
/// shared border whose two cells name different borders, which element's border Excel shows.
/// Keyed by (element of the left/top cell, element of the right/bottom cell, H or V).
#[test]
#[ignore = "diagnostic"]
fn edge_conflicts() {
    use open365_pivot::style::edge_sources;
    use std::collections::BTreeMap;
    let set = std::env::var("PIVOT_SET").unwrap_or_else(|_| "styles".into());
    let path = root().join(format!("fixtures/xlsx/pivot-{set}.xlsx"));
    let bytes = std::fs::read(&path).expect("fixture");
    let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    let rpath = root().join(format!("fixtures/reference/xlsx/pivot-{set}/pivot.json"));
    let refs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&rpath).expect("reference")).expect("json");
    let mut tally: BTreeMap<(usize, usize, char), [usize; 3]> = BTreeMap::new();
    let mut disagree = 0;
    for p in refs["pivots"].as_array().expect("pivots") {
        let si = wb
            .sheet_index(p["sheet"].as_str().expect("sheet"))
            .expect("sheet");
        let pivot = sheet_pivots(&wb, si).into_iter().next().expect("pivot");
        let grid = pivot
            .grid_from_cache(&PivotStrings::default())
            .expect("grid");
        let Some(src) = edge_sources(&pivot.table, &grid, wb.styles().theme()) else {
            continue;
        };
        let mut want: std::collections::HashMap<(u32, u32), [String; 4]> = Default::default();
        for c in p["cells"].as_array().expect("cells") {
            let (r, col) = parse_cell_ref(c["a"].as_str().expect("a")).expect("address");
            want.insert(
                (r, col),
                std::array::from_fn(|k| c["bd"][k].as_str().unwrap_or("").to_owned()),
            );
        }
        let (r0, c0) = (grid.body.0, grid.body.1);
        let key =
            |e: &open365_pivot::style::Edge| format!("{}:{}:{}", e.line, e.weight, bgr(e.color));
        for (r, row) in src.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                for (side, dr, dc, opp, o) in
                    [(2usize, 1usize, 0usize, 1usize, 'H'), (3, 0, 1, 0, 'V')]
                {
                    let Some(n) = src.get(r + dr).and_then(|x| x.get(c + dc)) else {
                        continue;
                    };
                    let (ra, ca) = (r0 + r as u32, c0 + c as u32);
                    let (rb, cb) = (ra + dr as u32, ca + dc as u32);
                    let (Some(wa), Some(wb_)) = (want.get(&(ra, ca)), want.get(&(rb, cb))) else {
                        continue;
                    };
                    if wa[side] != wb_[opp] {
                        disagree += 1;
                    }
                    let (Some(a), Some(b)) = (cell[side].last(), n[opp].last()) else {
                        continue;
                    };
                    if key(&a.1) == key(&b.1) {
                        continue;
                    }
                    let slot = tally.entry((a.0, b.0, o)).or_insert([0; 3]);
                    if wa[side] == key(&a.1) {
                        slot[0] += 1;
                    } else if wa[side] == key(&b.1) {
                        slot[1] += 1;
                    } else {
                        slot[2] += 1;
                    }
                }
            }
        }
    }
    println!("cells whose two sides disagree: {disagree}");
    for ((a, b, o), [aw, bw, nw]) in tally {
        println!("{o} first={a:>2} second={b:>2}: first wins {aw}, second wins {bw}, neither {nw}");
    }
}
