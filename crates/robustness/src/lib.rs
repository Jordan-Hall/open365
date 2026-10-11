//! Helpers for the robustness suite: zip building and patching, a deterministic mutator,
//! guarded in-process runs and a child-process runner with time and memory limits.
//!
//! The mutator is a small seeded xorshift so the suite needs no extra dependency and every
//! failure replays from `(fixture, seed)`.

use std::cell::RefCell;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use open365_opc::{Limits, Package};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// Limits handed to every parser under test. Tighter than the product defaults so a blow-up
/// below them is a real finding, not a configuration choice.
pub fn test_limits() -> Limits {
    Limits {
        max_parts: 5_000,
        max_part_bytes: 32 * 1024 * 1024,
        max_total_bytes: 96 * 1024 * 1024,
        max_ratio: 200,
        ratio_floor_bytes: 1024 * 1024,
    }
}

/// Wall-clock budget for one input: 20 s in release, 90 s in a debug build (about 15x slower).
pub const TIME_LIMIT: Duration = if cfg!(debug_assertions) {
    Duration::from_secs(90)
} else {
    Duration::from_secs(20)
};
/// Resident-memory budget for one child process, in bytes.
pub const MEMORY_LIMIT: u64 = 1024 * 1024 * 1024;

/// Runs one parser on `bytes`. `Ok` and `Err` are both acceptable outcomes; panics are not.
///
/// # Errors
/// The parser's error text, or an unknown-kind message.
pub fn exercise(kind: &str, bytes: &[u8]) -> Result<String, String> {
    let limits = test_limits();
    match kind {
        "opc" => opc(bytes, &limits),
        "docx" => docx(bytes, &limits),
        "xlsx" => xlsx(bytes, &limits),
        "package" => {
            let a = opc(bytes, &limits);
            let b = docx(bytes, &limits);
            let c = xlsx(bytes, &limits);
            a.and(b).and(c)
        }
        "formula" => formula(&String::from_utf8_lossy(bytes)),
        other => Err(format!("unknown kind {other}")),
    }
}

fn opc(bytes: &[u8], limits: &Limits) -> Result<String, String> {
    let package = Package::read(bytes, limits).map_err(|e| e.to_string())?;
    let n = package.parts().count();
    package.write().map_err(|e| e.to_string())?;
    Ok(format!("{n} parts"))
}

fn docx(bytes: &[u8], limits: &Limits) -> Result<String, String> {
    let mut doc = open365_docx::Document::open(bytes, limits).map_err(|e| e.to_string())?;
    let out = doc.save().map_err(|e| e.to_string())?;
    Ok(format!("saved {} bytes", out.len()))
}

fn xlsx(bytes: &[u8], limits: &Limits) -> Result<String, String> {
    let mut book = open365_xlsx::Workbook::open(bytes, limits).map_err(|e| e.to_string())?;
    let out = book.save().map_err(|e| e.to_string())?;
    Ok(format!("saved {} bytes", out.len()))
}

fn formula(text: &str) -> Result<String, String> {
    use open365_calc::{Engine, Value};
    let mut engine = Engine::new();
    engine.add_sheet("S");
    for row in 1..=10u32 {
        for col in 1..=3u32 {
            engine.set_value("S", row, col, Value::Number(f64::from(row * col)));
        }
    }
    engine
        .set_formula("S", 1, 5, text)
        .map_err(|e| e.to_string())?;
    engine.recalc_all();
    Ok(format!("{:?}", engine.get("S", 1, 5))
        .chars()
        .take(80)
        .collect())
}

/// Reads every entry of a zip as `(name, data)`.
///
/// # Panics
/// When `bytes` is not a readable zip; only used on trusted base fixtures.
pub fn read_zip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("base zip");
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).expect("entry");
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().expect("name").to_string();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("read");
        out.push((name, data));
    }
    out
}

/// Writes entries as a deflated zip.
///
/// # Panics
/// When the zip writer rejects an entry (for example a duplicate name).
pub fn write_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer
            .start_file(
                name.as_str(),
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .expect("start_file");
        writer.write_all(data).expect("write");
    }
    writer.finish().expect("finish").into_inner()
}

/// Replaces (or adds) one part of a package.
pub fn with_part(base: &[u8], name: &str, data: Vec<u8>) -> Vec<u8> {
    let mut entries = read_zip(base);
    match entries.iter_mut().find(|(n, _)| n == name) {
        Some(slot) => slot.1 = data,
        None => entries.push((name.to_string(), data)),
    }
    write_zip(&entries)
}

/// Root of the repository, found from this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// All fixtures with the given extension under `fixtures/<dir>`, sorted by name.
///
/// # Panics
/// When the fixture directory cannot be read.
pub fn fixtures(dir: &str, ext: &str) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let path = repo_root().join("fixtures").join(dir);
    for entry in std::fs::read_dir(&path).expect("fixture dir").flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == ext) {
            let name = p.file_name().expect("name").to_string_lossy().into_owned();
            out.push((name, std::fs::read(&p).expect("read fixture")));
        }
    }
    out.sort();
    out
}

/// One fixture by file name.
///
/// # Panics
/// When the file does not exist.
pub fn fixture(dir: &str, name: &str) -> Vec<u8> {
    std::fs::read(repo_root().join("fixtures").join(dir).join(name)).expect("fixture")
}

/// Seeded xorshift64*.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Value below `n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next_u64() % (n.max(1) as u64)).unwrap_or(0)
    }
}

const TOKENS: &[&str] = &[
    "<",
    ">",
    "&",
    "&#x0;",
    "&#xD800;",
    "&amp;",
    "]]>",
    "<![CDATA[",
    "<!--",
    "<?xml",
    "\"",
    "/>",
    "</",
    "<a:b xmlns:a=\"",
    "4294967295",
    "4294967296",
    "-1",
    "-2147483649",
    "2147483648",
    "99999999999999999999999",
    "1e308",
    "NaN",
    "0",
    "1048577",
    "16385",
    "\u{0}",
    "\u{FEFF}",
    "../",
    "http://127.0.0.1:1/",
];

/// Applies a few random edits to a part's bytes: flip, delete, duplicate, truncate, insert a
/// token, or replace a digit run by an extreme number.
pub fn mutate(rng: &mut Rng, data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    for _ in 0..=rng.below(4) {
        if out.is_empty() {
            out.extend_from_slice(b"<");
            continue;
        }
        let at = rng.below(out.len());
        match rng.below(7) {
            0 => out[at] ^= 1 << rng.below(8),
            1 => {
                let end = (at + 1 + rng.below(64)).min(out.len());
                out.drain(at..end);
            }
            2 => {
                let end = (at + 1 + rng.below(256)).min(out.len());
                let chunk = out[at..end].to_vec();
                let times = 1 + rng.below(8);
                for _ in 0..times {
                    out.splice(at..at, chunk.iter().copied());
                }
            }
            3 => out.truncate(at),
            4 | 5 => {
                let token = TOKENS[rng.below(TOKENS.len())];
                out.splice(at..at, token.bytes());
            }
            _ => {
                if let Some(start) = out[at..].iter().position(u8::is_ascii_digit) {
                    let start = at + start;
                    let end = start
                        + out[start..]
                            .iter()
                            .take_while(|b| b.is_ascii_digit())
                            .count();
                    let token = TOKENS[14 + rng.below(8)];
                    out.splice(start..end, token.bytes());
                }
            }
        }
    }
    out
}

/// Mutates one random part of a package (and rezips, so checksums stay valid).
pub fn mutate_package(rng: &mut Rng, base: &[u8]) -> Vec<u8> {
    let mut entries = read_zip(base);
    if entries.is_empty() {
        return base.to_vec();
    }
    let i = rng.below(entries.len());
    entries[i].1 = mutate(rng, &entries[i].1);
    write_zip(&entries)
}

/// How one guarded run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The parser returned `Ok`.
    Accepted,
    /// The parser returned an error value.
    Rejected(String),
    /// A panic, with message and `file:line`.
    Panicked(String),
    /// The time budget ran out.
    TimedOut,
    /// The child died without a panic (stack overflow, abort) with this exit code.
    Crashed(Option<i32>, String),
    /// The child exceeded the memory budget (peak bytes seen).
    TooMuchMemory(u64),
}

impl Outcome {
    /// True for the two acceptable outcomes.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Accepted | Self::Rejected(_))
    }
}

thread_local! {
    static LAST_PANIC: RefCell<String> = const { RefCell::new(String::new()) };
}

fn install_hook() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        std::panic::set_hook(Box::new(|info| {
            let place = info
                .location()
                .map_or_else(String::new, |l| format!("{}:{}", l.file(), l.line()));
            LAST_PANIC.with(|slot| *slot.borrow_mut() = format!("{info} at {place}"));
        }));
    });
}

/// Runs `exercise` in this process on a worker thread with a time limit. Panics are caught.
/// A stack overflow would still abort the test binary; hostile cases use [`run_child`].
pub fn run_in_process(kind: &'static str, bytes: Vec<u8>) -> Outcome {
    install_hook();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let result = std::panic::catch_unwind(|| exercise(kind, &bytes));
            let outcome = match result {
                Ok(Ok(_)) => Outcome::Accepted,
                Ok(Err(e)) => Outcome::Rejected(e),
                Err(_) => Outcome::Panicked(LAST_PANIC.with(|slot| slot.borrow().clone())),
            };
            let _ = tx.send(outcome);
        })
        .expect("spawn");
    rx.recv_timeout(TIME_LIMIT).unwrap_or(Outcome::TimedOut)
}

/// Resident memory of a process in bytes, where the platform allows reading it cheaply.
fn resident_bytes(pid: u32) -> Option<u64> {
    if cfg!(windows) {
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let field = text.trim().rsplit("\",\"").next()?;
        let digits: String = field.chars().filter(char::is_ascii_digit).collect();
        digits.parse::<u64>().ok().map(|kib| kib * 1024)
    } else {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
        let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kib * 1024)
    }
}

/// Runs the `o365-probe` child on `bytes`, enforcing [`TIME_LIMIT`] and [`MEMORY_LIMIT`].
///
/// # Panics
/// When the probe binary cannot be started.
pub fn run_child(kind: &str, name: &str, bytes: &[u8]) -> Outcome {
    // Antivirus scanners can briefly lock a fresh file; the probe then exits with code 2.
    let mut outcome = run_child_once(kind, name, bytes);
    for _ in 0..3 {
        if !matches!(outcome, Outcome::Crashed(Some(2), _)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
        outcome = run_child_once(kind, name, bytes);
    }
    outcome
}

fn run_child_once(kind: &str, name: &str, bytes: &[u8]) -> Outcome {
    let dir = std::env::temp_dir().join("open365-robustness");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join(format!("{}-{name}", std::process::id()));
    std::fs::write(&file, bytes).expect("write input");
    if let Err(e) = std::fs::read(&file) {
        return Outcome::Rejected(format!("input unreadable after write: {e}"));
    }
    let mut child = Command::new(probe_path())
        .arg(kind)
        .arg(&file)
        .env("RUST_BACKTRACE", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start probe");
    let pid = child.id();
    let start = Instant::now();
    let mut peak = 0u64;
    let outcome = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            let mut stdout = String::new();
            let mut stderr = String::new();
            if let Some(mut s) = child.stdout.take() {
                let _ = s.read_to_string(&mut stdout);
            }
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_string(&mut stderr);
            }
            break if status.code() == Some(101) {
                let line = stderr
                    .lines()
                    .find(|l| l.contains("panicked"))
                    .map(|l| format!("{l} {}", stderr.lines().nth(1).unwrap_or("")))
                    .unwrap_or_default();
                Outcome::Panicked(line)
            } else if let Some(rest) = stdout.strip_prefix("ERR ") {
                Outcome::Rejected(rest.trim().to_string())
            } else if stdout.starts_with("OK") {
                Outcome::Accepted
            } else {
                Outcome::Crashed(status.code(), stderr.chars().take(300).collect())
            };
        }
        if start.elapsed() > TIME_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            break Outcome::TimedOut;
        }
        if let Some(bytes) = resident_bytes(pid) {
            peak = peak.max(bytes);
            if bytes > MEMORY_LIMIT {
                let _ = child.kill();
                let _ = child.wait();
                break Outcome::TooMuchMemory(peak);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    match std::env::var_os("ROBUST_KEEP") {
        Some(keep) => {
            let _ = std::fs::create_dir_all(&keep);
            let _ = std::fs::copy(&file, Path::new(&keep).join(name));
            let _ = std::fs::remove_file(&file);
        }
        None => {
            let _ = std::fs::remove_file(&file);
        }
    }
    outcome
}

/// Fails the test unless the outcome is clean.
///
/// # Panics
/// When `outcome` is a panic, timeout, crash or memory blow-up.
pub fn assert_clean(label: &str, outcome: &Outcome) {
    assert!(outcome.is_clean(), "{label}: {outcome:?}");
}

/// Overwrites the 4-byte little-endian field at `offset` inside every zip header that starts
/// with signature `sig` (`0x0201_4b50` central directory, `0x0403_4b50` local header).
pub fn patch_headers(zip: &mut [u8], sig: u32, offset: usize, value: u32) {
    let pattern = sig.to_le_bytes();
    let mut i = 0;
    while i + offset + 4 <= zip.len() {
        if zip[i..i + 4] == pattern {
            zip[i + offset..i + offset + 4].copy_from_slice(&value.to_le_bytes());
            i += 4;
        } else {
            i += 1;
        }
    }
}

/// The `o365-probe` binary: cargo builds it before integration tests and puts it one
/// directory above the test executable.
fn probe_path() -> PathBuf {
    let mut path = std::env::current_exe().expect("current exe");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join(format!("o365-probe{}", std::env::consts::EXE_SUFFIX))
}
