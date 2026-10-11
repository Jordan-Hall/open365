//! Compares the layout engine with Microsoft Word, line by line.
//!
//! Reference: `fixtures/reference/layout/<name>.json`, written by
//! `tools/office/word-layout-oracle.ps1` from the real Word. Each reference line must be
//! reproduced with the same page, the same first-character offset (so identical line breaks),
//! the same vertical position (baseline from the PDF when the oracle has it, else the screen
//! line top) within 0.5 pt, and the same x of the first drawn glyph within 0.5 pt (from the PDF
//! when available: the screen `left`/`lastX` values are unreliable for aligned text).
//!
//! The expected number of matching lines per fixture lives in
//! `fixtures/reference/layout/parity-floor.toml`. The floor only goes up: a lower result fails
//! the test; a higher one prints a note, and `UPDATE_PARITY_FLOOR=1` rewrites the file.
//! `PARITY_DETAIL=<fixture>` prints every reference line with the engine's answer.
//!
//! Two font modes. The default uses only the bundled fonts (Calibri -> Carlito, Cambria ->
//! Caladea), so results do not depend on the machine; a fixture that asks for a family the
//! bundle cannot stand in for (Calibri Light, Arial, Aptos ...) is reported as skipped, never as
//! passed. Caladea is not metric-compatible with the Cambria Word used (its glyphs are up to
//! 10 % narrower), so Cambria text only approximates. With `OPEN365_PARITY_SYSTEM_FONTS=1` the
//! test loads the real faces from the machine (Windows font folders and Office's cloud font
//! cache for Aptos) and scores against the `[system.*]` floors; a fixture whose families the
//! machine lacks is skipped. The references were produced on a machine with those fonts.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use common::{Mode, library, root, score};

fn floor_path() -> PathBuf {
    root().join("fixtures/reference/layout/parity-floor.toml")
}

type Floors = BTreeMap<String, (usize, usize)>;

fn read_floors() -> (Floors, Floors) {
    let text = fs::read_to_string(floor_path()).expect("parity-floor.toml");
    let table: toml::Table = text.parse().expect("parse parity-floor.toml");
    let section = |name: &str| {
        let mut out = Floors::new();
        if let Some(toml::Value::Table(fixtures)) = table.get(name) {
            for (fixture, v) in fixtures {
                let m = v["matched"].as_integer().expect("matched") as usize;
                let t = v["total"].as_integer().expect("total") as usize;
                out.insert(fixture.clone(), (m, t));
            }
        }
        out
    };
    (section("fixtures"), section("system"))
}

fn write_floors(bundled: &Floors, system: &Floors) {
    let mut out = String::from(concat!(
        "# Ratchet for crates/layout/tests/word_parity.rs: lines that must match Word.
",
        "# [fixtures.*] is the default bundled-font mode, [system.*] the
",
        "# OPEN365_PARITY_SYSTEM_FONTS=1 mode. Raise `matched` when the engine improves
",
        "# (UPDATE_PARITY_FLOOR=1 cargo test -p open365-layout --test word_parity); never
",
        "# lower it. `total` is the number of reference lines.
",
    ));
    for (section, floors) in [("fixtures", bundled), ("system", system)] {
        for (name, (matched, total)) in floors {
            let _ = write!(
                out,
                "
[{section}.{name}]
matched = {matched}
total = {total}
"
            );
        }
    }
    fs::write(floor_path(), out).expect("write floor");
}

fn reference_names() -> Vec<String> {
    let dir = root().join("fixtures/reference/layout");
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("reference dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        // `<name>.lists.json` holds Word's list labels, not line positions.
        .filter(|n| !n.ends_with(".lists"))
        .collect();
    names.sort();
    names
}

#[test]
fn layout_matches_word_at_least_as_well_as_the_floor() {
    let detail = std::env::var("PARITY_DETAIL").ok();
    let update = std::env::var_os("UPDATE_PARITY_FLOOR").is_some();
    let mode = Mode::from_env();
    let (mut bundled_floor, mut system_floor) = read_floors();
    let floor = match mode {
        Mode::Bundled => &mut bundled_floor,
        Mode::System => &mut system_floor,
    };
    let fonts = library(mode);
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    let mut summary = format!(
        "
Word layout parity, {} fonts (matched / total lines; stage columns count lines whose break is found)
",
        match mode {
            Mode::Bundled => "bundled",
            Mode::System => "system",
        }
    );
    let _ = writeln!(
        summary,
        "{:<24} {:>9}  {:>6} {:>5} {:>5} {:>5} {:>5} {:>6}",
        "fixture", "matched", "breaks", "page", "y", "x", "end", "lastX"
    );
    let (mut all_matched, mut all_total) = (0, 0);
    for name in reference_names() {
        let want = detail.as_deref().is_some_and(|d| d == name || d == "all");
        let s = match score(&name, want, mode, &fonts) {
            Ok(s) => s,
            Err(missing) => {
                skipped.push(format!("{name} (needs {missing})"));
                continue;
            }
        };
        all_matched += s.matched;
        all_total += s.total;
        let _ = writeln!(
            summary,
            "{:<24} {:>4}/{:<4}  {:>6} {:>5} {:>5} {:>5} {:>5} {:>6}",
            name, s.matched, s.total, s.breaks, s.page, s.y, s.x, s.end, s.last_x
        );
        match floor.get(&name).copied() {
            None => {
                if !update {
                    failures.push(format!(
                        "{name}: no entry in parity-floor.toml (matched {}/{})",
                        s.matched, s.total
                    ));
                }
            }
            Some((fm, ft)) => {
                if ft != s.total {
                    failures.push(format!(
                        "{name}: reference has {} lines, floor file says {ft}",
                        s.total
                    ));
                }
                if s.matched < fm {
                    failures.push(format!(
                        "{name}: matched {} lines, floor is {fm}",
                        s.matched
                    ));
                } else if s.matched > fm {
                    let _ = writeln!(
                        summary,
                        "  note: {name} improved to {} (floor {fm}); raise the floor",
                        s.matched
                    );
                }
            }
        }
        if update {
            let keep = floor.get(&name).map_or(0, |f| f.0).max(s.matched);
            floor.insert(name, (keep, s.total));
        }
    }
    let _ = writeln!(
        summary,
        "{:<24} {:>4}/{:<4}",
        "all scored", all_matched, all_total
    );
    for item in &skipped {
        let _ = writeln!(summary, "skipped: {item}");
    }
    println!("{summary}");
    if update {
        write_floors(&bundled_floor, &system_floor);
    }
    assert!(
        failures.is_empty(),
        "parity regressions:
{}",
        failures.join(
            "
"
        )
    );
}
