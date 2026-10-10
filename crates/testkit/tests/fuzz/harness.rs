//! Panic capture, stages, minimisation and repro files for the fuzz smoke test.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use open365_calc::{Engine, Value, parse_formula, to_formula_string};
use open365_commands::{CellEntry, Command, DocPos, Target};
use open365_docx::Document;
use open365_opc::{Limits, Package};
use open365_testkit::{inspect, inspect_hidden, roundtrip};
use open365_xlsx::Workbook;

use super::mutate::{Entry, zip};

/// Crates whose panics are fixed in this change (opc, testkit, cli, dependencies of ours).
/// Other crates have other owners: their panics are saved as repros but do not fail the run
/// unless `OPEN365_FUZZ_STRICT=1`.
pub const OWNED_HERE: &[&str] = &["opc", "testkit", "cli", "dependency", "unknown"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub owner: String,
    /// `file:line` of the innermost workspace frame (or the panic location).
    pub location: String,
    pub message: String,
}

impl Finding {
    pub fn key(&self) -> String {
        format!("{}@{}", self.owner, self.location)
    }
}

static LAST: Mutex<Option<Finding>> = Mutex::new(None);

fn normalise(p: &str) -> String {
    p.replace('\\', "/")
}

/// `crates/<name>/...` or `apps/cli/...` -> owner name.
fn owner_of(path: &str) -> Option<&'static str> {
    let p = normalise(path);
    for name in [
        "opc", "docx", "xlsx", "calc", "commands", "testkit", "layout", "text",
    ] {
        if p.contains(&format!("crates/{name}/")) {
            return Some(match name {
                "opc" => "opc",
                "docx" => "docx",
                "xlsx" => "xlsx",
                "calc" => "calc",
                "commands" => "commands",
                "layout" => "layout",
                "text" => "text",
                _ => "testkit",
            });
        }
    }
    p.contains("apps/cli/").then_some("cli")
}

fn short(path: &str) -> String {
    let p = normalise(path);
    for marker in ["crates/", "apps/"] {
        if let Some(i) = p.rfind(marker) {
            return p[i..].to_owned();
        }
    }
    p.rsplit('/')
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("/")
}

pub fn install_hook() {
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic>".to_owned());
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let mut owner = owner_of(&at);
        let mut location = at.clone();
        if owner.is_none() {
            // Panic raised in std or a dependency: use the innermost workspace frame.
            let bt = std::backtrace::Backtrace::force_capture().to_string();
            for line in bt.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("at ")
                    && let Some(o) = owner_of(rest)
                    && !rest.contains("/tests/fuzz")
                {
                    owner = Some(o);
                    location = format!("{rest} (via {at})");
                    break;
                }
            }
        }
        let owner = owner.unwrap_or(if at.contains("/tests/fuzz") {
            "testkit"
        } else if at.contains(".cargo") || at.contains("registry") {
            "dependency"
        } else {
            "unknown"
        });
        if let Ok(mut slot) = LAST.lock() {
            *slot = Some(Finding {
                owner: owner.to_owned(),
                location: short(&location),
                message: message.chars().take(300).collect(),
            });
        }
    }));
}

/// Runs `f`, returning the panic it raised, if any.
pub fn guarded(f: impl FnOnce()) -> Option<Finding> {
    if let Ok(mut slot) = LAST.lock() {
        *slot = None;
    }
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(()) => None,
        Err(_) => Some(
            LAST.lock()
                .ok()
                .and_then(|mut s| s.take())
                .unwrap_or(Finding {
                    owner: "unknown".to_owned(),
                    location: "?".to_owned(),
                    message: "panic without hook data".to_owned(),
                }),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Docx,
    Xlsx,
}

impl Kind {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
        }
    }
}

/// open -> save -> reopen (and inspection) for one package. Any outcome but a panic is fine.
/// Returns whether the format crate (`docx` or `xlsx`) accepted the input.
pub fn run_package(kind: Kind, bytes: &[u8]) -> bool {
    let limits = Limits::default();
    if let Ok(p) = Package::read(bytes, &limits) {
        let _ = p.check_integrity();
        if let Ok(w) = p.write() {
            let _ = Package::read(&w, &limits);
        }
    }
    let _ = roundtrip(bytes);
    let _ = inspect(bytes);
    let _ = inspect_hidden(bytes);
    match kind {
        Kind::Docx => {
            let Ok(mut doc) = Document::open(bytes, &limits) else {
                return false;
            };
            let n = doc.paragraph_count();
            for i in 0..n.min(50) {
                let _ = doc.paragraph_text(i);
            }
            if let Ok(saved) = doc.save() {
                let _ = Document::open(&saved, &limits);
            }
            true
        }
        Kind::Xlsx => {
            let Ok(mut wb) = Workbook::open(bytes, &limits) else {
                return false;
            };
            let _ = wb.cell(0, 1, 1);
            if let Ok(saved) = wb.save() {
                let _ = Workbook::open(&saved, &limits);
            }
            true
        }
    }
}

/// open -> edit through the command layer -> undo/redo.
pub fn run_commands(kind: Kind, bytes: &[u8]) {
    match kind {
        Kind::Docx => {
            if let Ok(mut t) = Target::open_docx(bytes) {
                let cmds = [
                    Command::DocxInsertText {
                        para: 0,
                        offset: 1,
                        text: "x\u{301}y".to_owned(),
                    },
                    Command::DocxSetBold {
                        para: 0,
                        start: 0,
                        end: 3,
                        bold: true,
                    },
                    Command::DocxSplitParagraph {
                        pos: DocPos { para: 0, offset: 1 },
                    },
                    Command::DocxDeleteRange {
                        start: DocPos { para: 0, offset: 0 },
                        end: DocPos { para: 1, offset: 2 },
                    },
                ];
                for c in cmds {
                    let _ = t.apply(c);
                }
                let _ = t.undo();
                let _ = t.undo();
                let _ = t.redo();
            }
        }
        Kind::Xlsx => {
            if let Ok(mut t) = Target::open_xlsx(bytes) {
                let cmds = [
                    Command::XlsxSetCellValue {
                        sheet: 0,
                        row: 1,
                        col: 1,
                        value: CellEntry::Number(1.0),
                    },
                    Command::XlsxSetCellFormula {
                        sheet: 0,
                        row: 2,
                        col: 1,
                        formula: "=A1+1".to_owned(),
                    },
                    Command::XlsxSetCellFormula {
                        sheet: 0,
                        row: 1,
                        col: 1,
                        formula: "=A2".to_owned(),
                    },
                    Command::XlsxSetCellValue {
                        sheet: 0,
                        row: 1_048_576,
                        col: 16_384,
                        value: CellEntry::Text("t".to_owned()),
                    },
                ];
                for c in cmds {
                    let _ = t.apply(c);
                }
                let _ = t.undo();
                let _ = t.redo();
            }
        }
    }
}

/// Parse, print, load and evaluate one formula.
pub fn run_formula(formula: &str) {
    if let Ok(ast) = parse_formula(formula) {
        let _ = to_formula_string(&ast);
        let mut engine = Engine::new();
        engine.add_sheet("Sheet1");
        engine.set_value("Sheet1", 1, 1, Value::Number(2.0));
        if engine.set_formula("Sheet1", 1, 2, formula).is_ok() {
            let _ = engine.recalc_dirty();
            let _ = engine.get("Sheet1", 1, 2);
        }
    }
}

/// What to re-run when minimising or replaying.
#[derive(Debug, Clone, Copy)]
pub enum Stage {
    Package(Kind),
    Commands(Kind),
}

impl Stage {
    pub fn run(self, bytes: &[u8]) -> bool {
        match self {
            Self::Package(k) => run_package(k, bytes),
            Self::Commands(k) => {
                run_commands(k, bytes);
                false
            }
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Package(_) => "package",
            Self::Commands(_) => "commands",
        }
    }
}

/// True when `bytes` still raises a panic with `key` in `stage`.
fn reproduces(stage: Stage, key: &str, bytes: &[u8]) -> bool {
    guarded(|| {
        stage.run(bytes);
    })
    .is_some_and(|f| f.key() == key)
}

/// Delta-debugging style minimiser, bounded by `budget`. Works on zip entries when the case
/// has them, then on raw bytes.
pub fn minimise(
    stage: Stage,
    key: &str,
    bytes: &[u8],
    entries: Option<&[Entry]>,
    budget: Duration,
) -> Vec<u8> {
    let start = Instant::now();
    let mut best = bytes.to_vec();
    if let Some(entries) = entries {
        let mut cur: Vec<Entry> = entries.to_vec();
        // Drop whole entries.
        let mut i = 0;
        while i < cur.len() && start.elapsed() < budget {
            let mut trial = cur.clone();
            trial.remove(i);
            let z = zip(&trial);
            if reproduces(stage, key, &z) {
                cur = trial;
            } else {
                i += 1;
            }
        }
        // Shrink each entry's data.
        for idx in 0..cur.len() {
            let mut chunk = (cur[idx].data.len() / 2).max(1);
            while chunk >= 1 && start.elapsed() < budget {
                let mut pos = 0;
                while pos < cur[idx].data.len() && start.elapsed() < budget {
                    let end = (pos + chunk).min(cur[idx].data.len());
                    let mut trial = cur.clone();
                    trial[idx].data.drain(pos..end);
                    if reproduces(stage, key, &zip(&trial)) {
                        cur = trial;
                    } else {
                        pos += chunk;
                    }
                }
                if chunk == 1 {
                    break;
                }
                chunk /= 2;
            }
        }
        let z = zip(&cur);
        if reproduces(stage, key, &z) {
            best = z;
        }
    } else {
        let mut chunk = (best.len() / 2).max(1);
        while start.elapsed() < budget {
            let mut pos = 0;
            while pos < best.len() && start.elapsed() < budget {
                let end = (pos + chunk).min(best.len());
                let mut trial = best.clone();
                trial.drain(pos..end);
                if reproduces(stage, key, &trial) {
                    best = trial;
                } else {
                    pos += chunk;
                }
            }
            if chunk == 1 {
                break;
            }
            chunk /= 2;
        }
    }
    best
}

fn sanitise(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .chars()
        .take(90)
        .collect()
}

pub fn repro_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/repros")
}

/// Writes `data` plus a `.txt` note unless a repro for `key` exists. Returns the data path
/// when a new repro was written.
pub fn save_repro(finding: &Finding, ext: &str, data: &[u8], note: &str) -> Option<PathBuf> {
    let dir = repro_root().join(&finding.owner);
    std::fs::create_dir_all(&dir).ok()?;
    let stem = sanitise(&finding.location);
    let path = dir.join(format!("{stem}.{ext}"));
    if path.exists() {
        return None;
    }
    std::fs::write(&path, data).ok()?;
    let _ = std::fs::write(
        dir.join(format!("{stem}.txt")),
        format!(
            "owner: {}\nlocation: {}\npanic: {}\n{note}\n",
            finding.owner, finding.location, finding.message
        ),
    );
    Some(path)
}

#[derive(Debug, Default)]
pub struct Tally {
    pub by_key: BTreeMap<String, (Finding, usize)>,
}

impl Tally {
    pub fn record(&mut self, f: &Finding) -> bool {
        let slot = self.by_key.entry(f.key()).or_insert((f.clone(), 0));
        slot.1 += 1;
        slot.1 == 1
    }
}
