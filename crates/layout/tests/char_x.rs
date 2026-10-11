//! Per-character x parity: where the engine puts every glyph inside its line against where Word's
//! own PDF (`fixtures/reference/pdf/<name>.pdf`, `tools/office/make-pdf-refs.ps1`) draws it.
//!
//! `word_parity.rs` scores the first glyph of each line, so justification, alignment, tabs, list
//! labels and mixed sizes can all be wrong inside a line while the line-level test passes. Here
//! the glyphs of a page, engine and Word, are put in reading order (rows by baseline, left to
//! right) and paired by longest common subsequence of their non-blank characters, so a line that
//! breaks elsewhere costs pairs rather than misaligning the page. A pair agrees when x differs by
//! at most 0.5 pt and the baselines by at most `ROW` (the same row; vertical errors are scored by
//! `word_parity.rs`).
//!
//! Floors live in `fixtures/reference/layout/charx-floor.toml` (glyphs that agree per fixture,
//! bundled and system-font mode) and only go up: `UPDATE_CHARX_FLOOR=1` raises them.
//! `CHARX_DETAIL=<fixture>` prints, per row, the worst x error and the first glyphs that disagree.

mod common;
#[path = "common/wordpdf.rs"]
mod wordpdf;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use common::{Mode, library, root};
use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_text::FontMatch;
use wordpdf::{WordGlyph, read_pages};

const TOLERANCE: f64 = 0.5;
const ROW: f64 = 3.0;
/// Word's PDF places glyph origins 0.024 pt right of the layout position.
const PDF_X_OFFSET: f64 = 0.024;

fn floor_path() -> PathBuf {
    root().join("fixtures/reference/layout/charx-floor.toml")
}

type Floors = BTreeMap<String, (usize, usize)>;

fn read_floors() -> (Floors, Floors) {
    let text = fs::read_to_string(floor_path()).unwrap_or_default();
    let table: toml::Table = text.parse().expect("parse charx-floor.toml");
    let section = |name: &str| {
        let mut out = Floors::new();
        if let Some(toml::Value::Table(fixtures)) = table.get(name) {
            for (fixture, v) in fixtures {
                let a = v["agree"].as_integer().expect("agree") as usize;
                let t = v["total"].as_integer().expect("total") as usize;
                out.insert(fixture.clone(), (a, t));
            }
        }
        out
    };
    (section("fixtures"), section("system"))
}

fn write_floors(bundled: &Floors, system: &Floors) {
    let mut out = String::from(concat!(
        "# Ratchet for crates/layout/tests/char_x.rs: glyphs whose x is within 0.5 pt of Word's PDF.\n",
        "# [fixtures.*] is the default bundled-font mode, [system.*] the OPEN365_PARITY_SYSTEM_FONTS=1 mode.\n",
        "# Raise `agree` with UPDATE_CHARX_FLOOR=1 cargo test -p open365-layout --test char_x; never lower it.\n",
        "# `total` is the number of non-blank glyphs in Word's PDF.\n",
    ));
    for (section, floors) in [("fixtures", bundled), ("system", system)] {
        for (name, (agree, total)) in floors {
            let _ = write!(
                out,
                "\n[{section}.{name}]\nagree = {agree}\ntotal = {total}\n"
            );
        }
    }
    fs::write(floor_path(), out).expect("write floor");
}

#[derive(Clone)]
struct G {
    ch: char,
    x: f64,
    y: f64,
}

/// Puts glyphs in reading order: rows (single-linkage on baseline, gap under 1 pt), then x.
fn reading_order(mut v: Vec<G>) -> Vec<G> {
    v.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    let mut out = Vec::with_capacity(v.len());
    let mut row: Vec<G> = Vec::new();
    for g in v {
        if row.last().is_some_and(|l| g.y - l.y > 1.0) {
            row.sort_by(|a, b| a.x.total_cmp(&b.x));
            out.append(&mut row);
        }
        row.push(g);
    }
    row.sort_by(|a, b| a.x.total_cmp(&b.x));
    out.append(&mut row);
    out
}

/// Capitals made by `w:caps` / `w:smallCaps` are drawn as capitals; the engine keeps the typed case.
fn same(a: char, b: char) -> bool {
    a == b || a.to_uppercase().eq(b.to_uppercase())
}

fn align(a: &[G], b: &[G]) -> Vec<(usize, usize)> {
    let mut head = 0;
    while head < a.len() && head < b.len() && same(a[head].ch, b[head].ch) {
        head += 1;
    }
    let mut tail = 0;
    while tail < a.len() - head
        && tail < b.len() - head
        && same(a[a.len() - 1 - tail].ch, b[b.len() - 1 - tail].ch)
    {
        tail += 1;
    }
    let (am, bm) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut pairs: Vec<(usize, usize)> = (0..head).map(|i| (i, i)).collect();
    if !am.is_empty() && !bm.is_empty() && am.len() * bm.len() <= 60_000_000 {
        let w = bm.len() + 1;
        let mut t = vec![0u16; (am.len() + 1) * w];
        for i in (0..am.len()).rev() {
            for j in (0..bm.len()).rev() {
                t[i * w + j] = if same(am[i].ch, bm[j].ch) {
                    t[(i + 1) * w + j + 1] + 1
                } else {
                    t[(i + 1) * w + j].max(t[i * w + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < am.len() && j < bm.len() {
            if same(am[i].ch, bm[j].ch) {
                pairs.push((head + i, head + j));
                i += 1;
                j += 1;
            } else if t[(i + 1) * w + j] >= t[i * w + j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    let (an, bn) = (a.len() - tail, b.len() - tail);
    pairs.extend((0..tail).map(|k| (an + k, bn + k)));
    pairs
}

fn word_glyphs(glyphs: &[WordGlyph]) -> Vec<G> {
    reading_order(
        glyphs
            .iter()
            .filter(|g| !g.ch.is_whitespace() && g.ch != '\u{fffd}')
            .map(|g| G {
                ch: g.ch,
                x: g.x - PDF_X_OFFSET,
                y: g.y,
            })
            .collect(),
    )
}

struct Row {
    page: usize,
    y: f64,
    n: usize,
    bad: usize,
    worst: f64,
    first: Vec<String>,
}

struct Outcome {
    total: usize,
    paired: usize,
    agree: usize,
    rows: Vec<Row>,
}

fn score_fixture(name: &str, mode: Mode, detail: bool) -> Result<Outcome, String> {
    let root = root();
    let pdf = fs::read(root.join(format!("fixtures/reference/pdf/{name}.pdf")))
        .map_err(|_| "no reference pdf".to_owned())?;
    let bytes = fs::read(root.join(format!("fixtures/docx/{name}.docx")))
        .map_err(|_| "no docx".to_owned())?;
    let doc = Document::open(&bytes, &Limits::default()).map_err(|e| format!("{e:?}"))?;
    let fonts = library(mode);
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());
    let unusable: Vec<String> = layout
        .fonts
        .iter()
        .filter(|f| match mode {
            Mode::Bundled => f.matched == FontMatch::Fallback,
            Mode::System => f.matched != FontMatch::Exact,
        })
        .map(|f| f.requested.clone())
        .collect();
    if !unusable.is_empty() {
        return Err(format!("needs {}", unusable.join(", ")));
    }
    let word_pages = read_pages(&pdf);
    let mut out = Outcome {
        total: 0,
        paired: 0,
        agree: 0,
        rows: Vec::new(),
    };
    for (pi, wp) in word_pages.iter().enumerate() {
        let wc = word_glyphs(&wp.glyphs);
        let mut ours = Vec::new();
        if let Some(page) = layout.pages.iter().find(|p| p.number == pi + 1) {
            for l in page
                .header
                .iter()
                .chain(&page.lines)
                .chain(page.frames.iter().flat_map(|f| f.lines.iter()))
                .chain(&page.notes)
                .chain(&page.footer)
            {
                for r in &l.runs {
                    for g in &r.glyphs {
                        let t = l.cluster_text(g);
                        for ch in t.chars().filter(|c| !c.is_whitespace()) {
                            ours.push(G {
                                ch,
                                x: g.x,
                                y: l.baseline - g.y_offset,
                            });
                        }
                    }
                }
            }
        }
        let oc = reading_order(ours);
        let pairs = align(&wc, &oc);
        let mut ok = vec![false; wc.len()];
        let mut dxs: Vec<Option<f64>> = vec![None; wc.len()];
        for (i, j) in &pairs {
            let dx = oc[*j].x - wc[*i].x;
            dxs[*i] = Some(dx);
            ok[*i] = dx.abs() <= TOLERANCE && (oc[*j].y - wc[*i].y).abs() <= ROW;
        }
        out.total += wc.len();
        out.paired += pairs.len();
        out.agree += ok.iter().filter(|b| **b).count();
        if detail {
            // One report row per Word row (baseline cluster).
            let mut k = 0;
            while k < wc.len() {
                let mut e = k + 1;
                while e < wc.len() && (wc[e].y - wc[k].y).abs() < 1.0 {
                    e += 1;
                }
                if let Ok(want) = std::env::var("CHARX_ROW")
                    && want == format!("{}:{:.2}", pi + 1, wc[k].y)
                {
                    let mut line = String::new();
                    for i in k..e {
                        let _ = write!(
                            line,
                            "{}{:.2}/{:+.2} ",
                            wc[i].ch,
                            wc[i].x,
                            dxs[i].unwrap_or(f64::NAN)
                        );
                    }
                    println!("ROW {want}: {line}");
                }
                let bad = (k..e).filter(|&i| !ok[i]).count();
                let worst = (k..e)
                    .filter_map(|i| dxs[i])
                    .fold(0.0_f64, |m, d| if d.abs() > m.abs() { d } else { m });
                let first: Vec<String> = (k..e)
                    .filter(|&i| !ok[i])
                    .take(4)
                    .map(|i| match dxs[i] {
                        Some(d) => format!("{}@{:.1}({:+.2})", wc[i].ch, wc[i].x, d),
                        None => format!("{}@{:.1}(unpaired)", wc[i].ch, wc[i].x),
                    })
                    .collect();
                out.rows.push(Row {
                    page: pi + 1,
                    y: wc[k].y,
                    n: e - k,
                    bad,
                    worst,
                    first,
                });
                k = e;
            }
        }
        let _ = wp.width;
    }
    Ok(out)
}

fn names() -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(root().join("fixtures/reference/pdf"))
        .expect("pdf dir")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? != "pdf" {
                return None;
            }
            Some(p.file_stem()?.to_string_lossy().into_owned())
        })
        .filter(|n| root().join(format!("fixtures/docx/{n}.docx")).exists())
        .collect();
    v.sort();
    v
}

#[test]
fn glyph_x_matches_words_pdf_at_least_as_well_as_the_floor() {
    let detail = std::env::var("CHARX_DETAIL").ok();
    let update = std::env::var_os("UPDATE_CHARX_FLOOR").is_some();
    let mode = Mode::from_env();
    let (mut bundled, mut system) = read_floors();
    let floor = match mode {
        Mode::Bundled => &mut bundled,
        Mode::System => &mut system,
    };
    let mut failures = Vec::new();
    let mut summary = String::from("\nper-character x parity (glyphs within 0.5 pt of Word)\n");
    let (mut all_agree, mut all_total) = (0, 0);
    let only = std::env::var("CHARX_ONLY").ok();
    for name in names() {
        if only
            .as_ref()
            .is_some_and(|o| !o.split(',').any(|n| n == name))
        {
            continue;
        }
        let want = detail.as_deref().is_some_and(|d| d == name);
        let o = match score_fixture(&name, mode, want) {
            Ok(o) => o,
            Err(e) => {
                let _ = writeln!(summary, "skipped {name}: {e}");
                continue;
            }
        };
        all_agree += o.agree;
        all_total += o.total;
        let _ = writeln!(
            summary,
            "{name:<34} {:>6}/{:<6} {:>5.1}%  paired {:>5.1}%",
            o.agree,
            o.total,
            100.0 * o.agree as f64 / o.total.max(1) as f64,
            100.0 * o.paired as f64 / o.total.max(1) as f64
        );
        for r in &o.rows {
            if r.bad > 0 {
                let _ = writeln!(
                    summary,
                    "  p{} y {:>7.2} {:>3}/{:<3} bad, worst {:+.2}: {}",
                    r.page,
                    r.y,
                    r.bad,
                    r.n,
                    r.worst,
                    r.first.join(" ")
                );
            }
        }
        match floor.get(&name).copied() {
            None => {
                if !update {
                    failures.push(format!(
                        "{name}: no entry in charx-floor.toml ({}/{})",
                        o.agree, o.total
                    ));
                }
            }
            Some((fa, ft)) => {
                if ft != o.total {
                    failures.push(format!(
                        "{name}: Word has {} glyphs, floor file says {ft}",
                        o.total
                    ));
                }
                if o.agree < fa {
                    failures.push(format!("{name}: {} glyphs agree, floor is {fa}", o.agree));
                } else if o.agree > fa {
                    let _ = writeln!(
                        summary,
                        "  note: {name} improved to {} (floor {fa})",
                        o.agree
                    );
                }
            }
        }
        if update {
            let keep = floor.get(&name).map_or(0, |f| f.0).max(o.agree);
            floor.insert(name, (keep, o.total));
        }
    }
    let _ = writeln!(
        summary,
        "{:<34} {:>6}/{:<6} {:>5.1}%",
        "all",
        all_agree,
        all_total,
        100.0 * all_agree as f64 / all_total.max(1) as f64
    );
    println!("{summary}");
    if update {
        write_floors(&bundled, &system);
    }
    assert!(
        failures.is_empty(),
        "char x regressions:\n{}",
        failures.join("\n")
    );
}
