//! Lossless round trip over the local Microsoft template corpus.
//!
//! The corpus is gitignored (`fixtures/external/ms-templates/`, see
//! `docs/research/corpus-licensing.md`); the test skips cleanly when it is absent. For every
//! file it:
//!
//! 1. opens and saves it unchanged: every part must come back byte-identical, same order;
//! 2. applies one small typed edit and saves: every part outside the edited ones must be
//!    byte-identical, and what the edit removed or added must be in the hygiene log;
//! 3. turns it into a template (content type swap), checks that, then instantiates it and
//!    checks that only `[Content_Types].xml` differs from the original.
//!
//! Outputs go to `fixtures/external/roundtrip-out/<stage>/` (gitignored; override with
//! `OPEN365_CORPUS_OUT`) so `cargo xtask corpus-roundtrip` can open each one in Word or
//! Excel through `tools/office/office-oracle.ps1 -Mode verify`. Derived files are as
//! restricted as their sources: never commit them.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use open365_docx::{Document, DocumentKind};
use open365_opc::{Limits, Package};
use open365_xlsx::{CellInput, SheetKind, Workbook, WorkbookKind};

/// Stages whose outputs the oracle opens.
const STAGES: [&str; 4] = ["unchanged", "edited", "template", "instantiated"];

/// Known failures: `(corpus file stem, cause)`. Each entry is reported, not hidden.
const KNOWN_EXCEPTIONS: &[(&str, &str)] = &[];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corpus_dir() -> PathBuf {
    std::env::var_os("OPEN365_CORPUS_DIR").map_or_else(
        || repo_root().join("fixtures/external/ms-templates"),
        PathBuf::from,
    )
}

fn out_dir() -> PathBuf {
    std::env::var_os("OPEN365_CORPUS_OUT").map_or_else(
        || repo_root().join("fixtures/external/roundtrip-out"),
        PathBuf::from,
    )
}

fn norm(name: &str) -> String {
    name.trim_start_matches('/').to_ascii_lowercase()
}

/// Compares two packages part by part. `may_change` parts are exempt from the byte check.
/// Removed parts must be mentioned in `log` (the hygiene log as text); so must added parts.
/// Returns a description of every violation.
fn compare(
    before: &Package,
    after: &Package,
    may_change: &[String],
    log: &[String],
) -> Vec<String> {
    let mut problems = Vec::new();
    let exempt = |n: &str| may_change.iter().any(|m| norm(m) == norm(n));
    let logged = |n: &str| {
        log.iter()
            .any(|l| l.to_ascii_lowercase().contains(&norm(n)))
    };
    for part in before.parts() {
        let name = part.name().as_str();
        match after.part(name) {
            None if logged(name) => {}
            None => problems.push(format!("part {name} lost and not logged")),
            Some(p) if p.data() != part.data() && !exempt(name) => {
                problems.push(format!(
                    "part {name} changed ({} -> {} bytes)",
                    part.data().len(),
                    p.data().len()
                ));
            }
            Some(_) => {}
        }
    }
    for part in after.parts() {
        let name = part.name().as_str();
        if before.part(name).is_none() && !logged(name) {
            problems.push(format!("part {name} added and not logged"));
        }
    }
    problems
}

fn identical_in_order(before: &Package, after: &Package) -> Result<(), String> {
    let a: Vec<_> = before
        .parts()
        .map(|p| (p.name().as_str(), p.data()))
        .collect();
    let b: Vec<_> = after
        .parts()
        .map(|p| (p.name().as_str(), p.data()))
        .collect();
    if a.len() != b.len() {
        return Err(format!("{} parts became {}", a.len(), b.len()));
    }
    for ((n1, d1), (n2, d2)) in a.iter().zip(&b) {
        if n1 != n2 {
            return Err(format!("part order changed at {n1} / {n2}"));
        }
        if d1 != d2 {
            return Err(format!("part {n1} changed"));
        }
    }
    Ok(())
}

fn read_package(bytes: &[u8]) -> Result<Package, String> {
    Package::read(bytes, &Limits::default()).map_err(|e| format!("package: {e}"))
}

/// Replaces the main part's content type in `[Content_Types].xml` and rewrites the package.
fn retype(bytes: &[u8], from: &str, to: &str) -> Result<Vec<u8>, String> {
    let mut package = read_package(bytes)?;
    let part = package
        .part("[Content_Types].xml")
        .ok_or("no [Content_Types].xml")?;
    let text = std::str::from_utf8(part.data()).map_err(|e| format!("content types: {e}"))?;
    if !text.contains(from) {
        return Err(format!(
            "content type {from} not found in [Content_Types].xml"
        ));
    }
    let new = text.replacen(from, to, 1).into_bytes();
    let name = part.name().clone();
    package.set_part(name, new);
    package.write().map_err(|e| format!("write: {e}"))
}

fn dropped_text<T: std::fmt::Debug>(items: &[T]) -> String {
    let mut shown: Vec<String> = items.iter().take(3).map(|i| format!("{i:?}")).collect();
    if items.len() > 3 {
        shown.push(format!("... {} in all", items.len()));
    }
    shown.join("; ")
}

struct Outputs {
    dir: PathBuf,
    stem: String,
    ext: String,
}

impl Outputs {
    fn save(&self, stage: &str, ext: &str, bytes: &[u8]) -> Result<(), String> {
        let dir = self.dir.join(stage);
        fs::create_dir_all(&dir).map_err(|e| format!("create {stage}: {e}"))?;
        fs::write(dir.join(format!("{}.{ext}", self.stem)), bytes)
            .map_err(|e| format!("write {stage}: {e}"))
    }
}

fn docx_file(original: &[u8], out: &Outputs) -> Result<Vec<String>, String> {
    let limits = Limits::default();
    let before = read_package(original)?;
    let mut notes = Vec::new();

    // 1. Unchanged.
    let mut doc = Document::open(original, &limits).map_err(|e| format!("open: {e}"))?;
    let kind = doc.kind();
    let main = doc.main_part().as_str().to_owned();
    if !doc.dropped().is_empty() {
        return Err(format!("dropped on open: {}", dropped_text(doc.dropped())));
    }
    let saved = doc.save().map_err(|e| format!("save unchanged: {e}"))?;
    identical_in_order(&before, &read_package(&saved)?).map_err(|e| format!("unchanged: {e}"))?;
    out.save("unchanged", &out.ext, &saved)?;

    // 2. Edit: insert a marker at the start of the first non-empty paragraph.
    let mut doc = Document::open(original, &limits).map_err(|e| format!("open: {e}"))?;
    let count = doc.paragraph_count();
    let target = (0..count)
        .find(|i| doc.paragraph_text(*i).is_some_and(|t| !t.is_empty()))
        .or((count > 0).then_some(0));
    if let Some(index) = target {
        let marker = "Open365 edit ";
        doc.insert_text(index, 0, marker)
            .map_err(|e| format!("insert_text: {e}"))?;
        let saved = doc.save().map_err(|e| format!("save edited: {e}"))?;
        let log: Vec<String> = doc.hygiene_log().iter().map(ToString::to_string).collect();
        let after = read_package(&saved)?;
        let may = vec![
            main.clone(),
            "[Content_Types].xml".to_owned(),
            "_rels/.rels".to_owned(),
        ];
        let problems = compare(&before, &after, &may, &log);
        if !problems.is_empty() {
            return Err(format!("edited: {}", problems.join("; ")));
        }
        let reopened =
            Document::open(&saved, &limits).map_err(|e| format!("reopen edited: {e}"))?;
        let text = reopened.paragraph_text(index).unwrap_or_default();
        if !text.starts_with(marker) {
            return Err(format!("edited text not found after reopen: {text:?}"));
        }
        out.save("edited", &out.ext, &saved)?;
    } else {
        notes.push("no paragraph: edit skipped".to_owned());
    }

    // 3. Template and instantiate (on documents; real templates instantiate directly).
    let (template_bytes, template_ext) = if kind.is_template() {
        (original.to_vec(), out.ext.clone())
    } else {
        let to = if kind.is_macro_enabled() {
            DocumentKind::MacroEnabledTemplate
        } else {
            DocumentKind::Template
        };
        let ext = if kind.is_macro_enabled() {
            "dotm"
        } else {
            "dotx"
        };
        (
            retype(original, kind.content_type(), to.content_type())?,
            ext.to_owned(),
        )
    };
    out.save("template", &template_ext, &template_bytes)?;
    let mut doc =
        Document::open(&template_bytes, &limits).map_err(|e| format!("open template: {e}"))?;
    if !doc
        .instantiate_template()
        .map_err(|e| format!("instantiate: {e}"))?
    {
        return Err("template did not instantiate".to_owned());
    }
    let saved = doc.save().map_err(|e| format!("save instantiated: {e}"))?;
    let log: Vec<String> = doc.hygiene_log().iter().map(ToString::to_string).collect();
    let after = read_package(&saved)?;
    let problems = compare(
        &read_package(&template_bytes)?,
        &after,
        &["[Content_Types].xml".to_owned()],
        &log,
    );
    if !problems.is_empty() {
        return Err(format!("instantiated: {}", problems.join("; ")));
    }
    let reopened =
        Document::open(&saved, &limits).map_err(|e| format!("reopen instantiated: {e}"))?;
    let expect = if kind.is_macro_enabled() || doc.has_vba_project() {
        DocumentKind::MacroEnabledDocument
    } else {
        DocumentKind::Document
    };
    if reopened.kind() != expect {
        return Err(format!(
            "instantiated kind {:?}, expected {expect:?}",
            reopened.kind()
        ));
    }
    let ext = if expect == DocumentKind::MacroEnabledDocument {
        "docm"
    } else {
        "docx"
    };
    out.save("instantiated", ext, &saved)?;
    Ok(notes)
}

fn xlsx_file(original: &[u8], out: &Outputs) -> Result<Vec<String>, String> {
    let limits = Limits::default();
    let before = read_package(original)?;
    let mut notes = Vec::new();

    // 1. Unchanged.
    let mut wb = Workbook::open(original, &limits).map_err(|e| format!("open: {e}"))?;
    let kind = wb.kind();
    if !wb.dropped().is_empty() {
        return Err(format!("dropped on open: {}", dropped_text(wb.dropped())));
    }
    let saved = wb.save().map_err(|e| format!("save unchanged: {e}"))?;
    identical_in_order(&before, &read_package(&saved)?).map_err(|e| format!("unchanged: {e}"))?;
    out.save("unchanged", &out.ext, &saved)?;

    // 2. Edit: a number and a text below the used range of the first worksheet.
    let mut wb = Workbook::open(original, &limits).map_err(|e| format!("open: {e}"))?;
    let sheet = wb
        .sheets()
        .iter()
        .position(|s| s.kind == SheetKind::Worksheet);
    if let Some(sheet) = sheet {
        let next = wb
            .sheet(sheet)
            .and_then(|w| w.used_range())
            .map_or(1, |r| r.2 + 1);
        wb.set_cell_value(sheet, next, 1, CellInput::Text("Open365 edit".to_owned()))
            .map_err(|e| format!("set text: {e}"))?;
        wb.set_cell_value(sheet, next, 2, CellInput::Number(42.5))
            .map_err(|e| format!("set number: {e}"))?;
        let saved = wb.save().map_err(|e| format!("save edited: {e}"))?;
        let log: Vec<String> = wb.hygiene_log().iter().map(ToString::to_string).collect();
        let after = read_package(&saved)?;
        let mut may = vec![
            wb.workbook_part().as_str().to_owned(),
            "[Content_Types].xml".to_owned(),
            "_rels/.rels".to_owned(),
            "xl/_rels/workbook.xml.rels".to_owned(),
        ];
        if let Some(p) = wb.sheet_part(sheet) {
            may.push(p.as_str().to_owned());
        }
        if let Some(sst) = wb.shared_strings() {
            let _ = sst;
            may.push("xl/sharedStrings.xml".to_owned());
        }
        let problems = compare(&before, &after, &may, &log);
        if !problems.is_empty() {
            return Err(format!("edited: {}", problems.join("; ")));
        }
        let reopened =
            Workbook::open(&saved, &limits).map_err(|e| format!("reopen edited: {e}"))?;
        let ok = reopened.cell(sheet, next, 2).is_some() && reopened.cell(sheet, next, 1).is_some();
        if !ok {
            return Err("edited cells not found after reopen".to_owned());
        }
        out.save("edited", &out.ext, &saved)?;
    } else {
        notes.push("no worksheet: edit skipped".to_owned());
    }

    // 3. Template and instantiate.
    let (template_bytes, template_ext) = if kind.is_template() {
        (original.to_vec(), out.ext.clone())
    } else {
        let to = if kind.is_macro_enabled() {
            WorkbookKind::MacroEnabledTemplate
        } else {
            WorkbookKind::Template
        };
        let ext = if kind.is_macro_enabled() {
            "xltm"
        } else {
            "xltx"
        };
        (
            retype(original, kind.content_type(), to.content_type())?,
            ext.to_owned(),
        )
    };
    out.save("template", &template_ext, &template_bytes)?;
    let mut wb =
        Workbook::open(&template_bytes, &limits).map_err(|e| format!("open template: {e}"))?;
    if !wb
        .instantiate_template()
        .map_err(|e| format!("instantiate: {e}"))?
    {
        return Err("template did not instantiate".to_owned());
    }
    let saved = wb.save().map_err(|e| format!("save instantiated: {e}"))?;
    let log: Vec<String> = wb.hygiene_log().iter().map(ToString::to_string).collect();
    let after = read_package(&saved)?;
    let problems = compare(
        &read_package(&template_bytes)?,
        &after,
        &["[Content_Types].xml".to_owned()],
        &log,
    );
    if !problems.is_empty() {
        return Err(format!("instantiated: {}", problems.join("; ")));
    }
    let reopened =
        Workbook::open(&saved, &limits).map_err(|e| format!("reopen instantiated: {e}"))?;
    let expect = if kind.is_macro_enabled() || wb.has_vba_project() {
        WorkbookKind::MacroEnabledWorkbook
    } else {
        WorkbookKind::Workbook
    };
    if reopened.kind() != expect {
        return Err(format!(
            "instantiated kind {:?}, expected {expect:?}",
            reopened.kind()
        ));
    }
    let ext = if expect == WorkbookKind::MacroEnabledWorkbook {
        "xlsm"
    } else {
        "xlsx"
    };
    out.save("instantiated", ext, &saved)?;
    Ok(notes)
}

#[test]
fn corpus_roundtrip() {
    let dir = corpus_dir();
    let Ok(read) = fs::read_dir(&dir) else {
        eprintln!("corpus_roundtrip: SKIPPED, no corpus at {}", dir.display());
        return;
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| !n.starts_with("~$"))
                && p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    matches!(
                        e,
                        "docx" | "dotx" | "dotm" | "docm" | "xlsx" | "xltx" | "xltm" | "xlsm"
                    )
                })
        })
        .collect();
    files.sort();
    if files.is_empty() {
        eprintln!(
            "corpus_roundtrip: SKIPPED, {} holds no Office files",
            dir.display()
        );
        return;
    }
    let out_root = out_dir();
    for stage in STAGES {
        let _ = fs::remove_dir_all(out_root.join(stage));
    }

    let (mut ok, mut known) = (0usize, 0usize);
    let mut failures = Vec::new();
    let mut report = String::new();
    for path in &files {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_owned();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        let outputs = Outputs {
            dir: out_root.clone(),
            stem: stem.clone(),
            ext: ext.clone(),
        };
        let result = match fs::read(path) {
            Err(e) => Err(format!("read: {e}")),
            Ok(bytes) if ext.starts_with("doc") || ext.starts_with("dot") => {
                docx_file(&bytes, &outputs)
            }
            Ok(bytes) => xlsx_file(&bytes, &outputs),
        };
        match result {
            Ok(notes) => {
                ok += 1;
                for n in notes {
                    let _ = writeln!(report, "NOTE {stem}.{ext}: {n}");
                }
            }
            Err(cause) => {
                let known_cause = KNOWN_EXCEPTIONS.iter().find(|(s, _)| *s == stem);
                if let Some((_, why)) = known_cause {
                    known += 1;
                    let _ = writeln!(report, "KNOWN {stem}.{ext}: {cause} ({why})");
                } else {
                    let _ = writeln!(report, "FAIL {stem}.{ext}: {cause}");
                    failures.push(format!("{stem}.{ext}: {cause}"));
                }
            }
        }
    }
    let _ = fs::create_dir_all(&out_root);
    let _ = fs::write(out_root.join("report.txt"), &report);
    eprintln!(
        "corpus_roundtrip: {} files, {ok} ok, {known} known exceptions, {} failed; outputs in {}",
        files.len(),
        failures.len(),
        out_root.display()
    );
    assert!(
        failures.is_empty(),
        "corpus failures:\n{}",
        failures.join("\n")
    );
}
