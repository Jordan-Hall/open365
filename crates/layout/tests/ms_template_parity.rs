//! Compares the layout engine with Word on the public Microsoft template corpus.
//!
//! References: `fixtures/reference/ms-templates/<id>/layout.json`, written by
//! `tools/office/word-layout-oracle.ps1` for `fixtures/external/ms-templates/<id>.docx`.
//! Both directories are local and gitignored (no redistribution); without them the test prints a
//! note and passes. The comparison is the one `word_parity.rs` makes (page, first-character
//! offset, baseline within 0.5 pt, first glyph x, cell and picture rectangles), always with the
//! real fonts from the machine: every family a template names is loaded, and a family the machine
//! lacks falls back, which the summary reports.
//!
//! `fixtures/reference/ms-templates-layout-floor.toml` holds, per template id, the matched and
//! total counts only. It is a ratchet like `parity-floor.toml`: a lower result fails, a higher
//! one prints a note, and `UPDATE_PARITY_FLOOR=1` raises it (never lowers). `PARITY_DETAIL=<id>`
//! (or `all`) prints every reference line with the engine's answer.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use common::{Mode, library_with, root, score_doc};
use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_text::{FontLibrary, FontMatch};

fn floor_path() -> PathBuf {
    root().join("fixtures/reference/ms-templates-layout-floor.toml")
}

type Floors = BTreeMap<String, (usize, usize)>;

fn read_floors() -> Floors {
    let Ok(text) = fs::read_to_string(floor_path()) else {
        return Floors::new();
    };
    let table: toml::Table = text.parse().expect("parse ms-templates-layout-floor.toml");
    let mut out = Floors::new();
    for (id, v) in &table {
        let m = v["matched"].as_integer().expect("matched") as usize;
        let t = v["total"].as_integer().expect("total") as usize;
        out.insert(id.clone(), (m, t));
    }
    out
}

fn write_floors(floors: &Floors) {
    let mut out = String::from(concat!(
        "# Ratchet for crates/layout/tests/ms_template_parity.rs: lines, cells and pictures that must\n",
        "# match Word on the Microsoft template corpus (system fonts). Counts only, keyed by template\n",
        "# id; no template content. Raise `matched` when the engine improves\n",
        "# (UPDATE_PARITY_FLOOR=1 cargo test -p open365-layout --test ms_template_parity); never lower it.\n",
    ));
    for (id, (matched, total)) in floors {
        let _ = write!(out, "\n[{id}]\nmatched = {matched}\ntotal = {total}\n");
    }
    fs::write(floor_path(), out).expect("write floor");
}

#[test]
fn layout_matches_word_on_the_template_corpus_at_least_as_well_as_the_floor() {
    let root = root();
    let corpus = root.join("fixtures/external/ms-templates");
    let refs = root.join("fixtures/reference/ms-templates");
    let detail = std::env::var("PARITY_DETAIL").ok();
    let update = std::env::var_os("UPDATE_PARITY_FLOOR").is_some();

    // (id, parsed document) for every template that has both the file and a reference.
    let mut ids: Vec<String> = fs::read_dir(&refs)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|id| refs.join(id).join("layout.json").is_file())
        .filter(|id| corpus.join(format!("{id}.docx")).is_file())
        .collect();
    ids.sort();
    if ids.is_empty() {
        println!("ms-template corpus absent (fixtures/external/ms-templates); skipped");
        return;
    }

    let mut docs: Vec<(String, Result<Document, String>)> = Vec::new();
    for id in &ids {
        let bytes = fs::read(corpus.join(format!("{id}.docx"))).expect("read template");
        let doc = Document::open(&bytes, &Limits::default()).map_err(|e| e.to_string());
        docs.push((id.clone(), doc));
    }

    // Load every family the templates ask for from the machine.
    let bundled = FontLibrary::bundled();
    let mut families: BTreeSet<String> = BTreeSet::new();
    for (_, doc) in &docs {
        if let Ok(doc) = doc {
            let layout = layout_document(doc, &bundled, LayoutOptions::default());
            families.extend(layout.fonts.iter().map(|f| f.requested.clone()));
        }
    }
    let extra: Vec<String> = families.into_iter().collect();
    let fonts = library_with(Mode::System, &extra);

    let mut floors = read_floors();
    let mut failures = Vec::new();
    let mut rows: Vec<(usize, String)> = Vec::new();
    let (mut all_matched, mut all_total, mut perfect) = (0, 0, 0);
    for (id, doc) in &docs {
        let want = detail.as_deref().is_some_and(|d| d == id || d == "all");
        let result = match doc {
            Err(e) => Err(format!("open failed: {e}")),
            Ok(doc) => catch_unwind(AssertUnwindSafe(|| {
                let missing: Vec<String> = layout_document(doc, &fonts, LayoutOptions::default())
                    .fonts
                    .iter()
                    .filter(|f| f.matched != FontMatch::Exact)
                    .map(|f| f.requested.clone())
                    .collect();
                let s = score_doc(
                    id,
                    &refs.join(id).join("layout.json"),
                    doc,
                    want,
                    Mode::System,
                    &fonts,
                    false,
                )
                .expect("not strict");
                (s, missing)
            }))
            .map_err(|_| "layout panicked".to_owned()),
        };
        let Ok((s, missing)) = result else {
            let why = result.err().unwrap_or_default();
            failures.push(format!("{id}: {why}"));
            continue;
        };
        all_matched += s.matched;
        all_total += s.total;
        perfect += usize::from(s.matched == s.total);
        let note = if missing.is_empty() {
            String::new()
        } else {
            format!("  missing fonts: {}", missing.join(", "))
        };
        rows.push((
            s.total - s.matched,
            format!(
                "{id} {:>5}/{:<5} breaks {:>5} page {:>5} y {:>5} x {:>5}{note}",
                s.matched, s.total, s.breaks, s.page, s.y, s.x
            ),
        ));
        match floors.get(id).copied() {
            None => {
                if !update {
                    failures.push(format!(
                        "{id}: no entry in ms-templates-layout-floor.toml (matched {}/{})",
                        s.matched, s.total
                    ));
                }
            }
            Some((fm, ft)) => {
                if ft != s.total {
                    failures.push(format!(
                        "{id}: reference has {} lines, floor file says {ft}",
                        s.total
                    ));
                }
                if s.matched < fm {
                    failures.push(format!("{id}: matched {} lines, floor is {fm}", s.matched));
                }
            }
        }
        if update {
            let keep = floors.get(id).map_or(0, |f| f.0).max(s.matched);
            floors.insert(id.clone(), (keep, s.total));
        }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut summary =
        String::from("\nMS template corpus parity, system fonts (id matched/total; worst first)\n");
    for (_, row) in &rows {
        let _ = writeln!(summary, "{row}");
    }
    let _ = writeln!(
        summary,
        "all scored {all_matched}/{all_total}; {perfect} of {} templates at 100%",
        rows.len()
    );
    println!("{summary}");
    if update {
        write_floors(&floors);
    }
    assert!(
        failures.is_empty(),
        "template parity regressions:\n{}",
        failures.join("\n")
    );
}
