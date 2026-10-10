//! Seeded, deterministic fuzz smoke test (see `docs/testing/fuzzing.md`).
//!
//! Every fixture is mutated at the zip level and the XML level; each mutant goes through
//! open -> save -> reopen and the command layer. Any outcome except a panic is accepted.
//! Budget: `OPEN365_FUZZ_SECS` (default 60). Seed: `OPEN365_FUZZ_SEED`. Panics in crates
//! owned elsewhere are saved under `fuzz/repros/<crate>/` and reported, and fail the run
//! only with `OPEN365_FUZZ_STRICT=1`.

mod fuzz;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fuzz::harness::{
    Finding, Kind, OWNED_HERE, Stage, Tally, guarded, install_hook, minimise, run_commands,
    run_formula, run_package, save_repro,
};
use fuzz::mutate::{Case, Rng, formula, unzip, xml_level, zip_level};

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn fixtures() -> Vec<(PathBuf, Kind, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut out = Vec::new();
    for (dir, kind) in [("docx", Kind::Docx), ("xlsx", Kind::Xlsx)] {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(root.join(dir))
            .expect("read fixture dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == dir))
            .collect();
        paths.sort();
        for p in paths {
            let bytes = std::fs::read(&p).expect("read fixture");
            // Debug builds are ~20x slower: leave the multi-megabyte stress fixture to
            // release runs so the default budget is honoured.
            if cfg!(debug_assertions) && bytes.len() > 512 * 1024 {
                continue;
            }
            out.push((p, kind, bytes));
        }
    }
    out
}

fn formula_seeds() -> Vec<String> {
    let mut seeds: Vec<String> = [
        "=SUM(A1:B2)*2",
        "=IF(A1>0,\"yes\",\"no\")",
        "='Sheet 1'!A1+Sheet2!$B$2",
        "=VLOOKUP(A1,B1:C9,2,FALSE)",
        "=-2^-2&\"x\"",
        "=SUM(A:A)/COUNT(1:1)",
        "=TEXT(1234.5,\"#,##0.00\")",
        "=A1:B2 C3:D4",
        "={1,2;3,4}",
        "=DATE(2024,2,30)+TIME(25,61,61)",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/calc/conformance.json");
    if let Ok(text) = std::fs::read_to_string(path)
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(cases) = v["cases"].as_array()
    {
        seeds.extend(
            cases
                .iter()
                .filter_map(|c| c["formula"].as_str().map(str::to_owned)),
        );
    }
    seeds
}

fn mutate(seed: &[u8], rng: &mut Rng) -> Case {
    let mut case = if rng.chance(5) {
        zip_level(seed, rng)
    } else {
        xml_level(seed, rng)
    };
    if rng.chance(6) {
        let second = zip_level(&case.bytes, rng);
        case.label = format!("{}+{}", case.label, second.label);
        case.bytes = second.bytes;
        case.entries = second.entries;
    }
    case
}

#[test]
fn fuzz_smoke() {
    install_hook();
    let secs = env_u64("OPEN365_FUZZ_SECS", 60);
    let seed = env_u64("OPEN365_FUZZ_SEED", 0x0005_EED0_0365);
    let strict = std::env::var("OPEN365_FUZZ_STRICT").is_ok_and(|v| v == "1");
    let verbose = std::env::var("OPEN365_FUZZ_VERBOSE").is_ok();
    let max_iters = env_u64("OPEN365_FUZZ_ITERS", u64::MAX);
    let fixtures = fixtures();
    assert!(fixtures.len() >= 20, "fixtures missing");
    let formulas = formula_seeds();
    // Writing a file per case is slow on hosts with real-time scanning, so it is opt-in: set
    // OPEN365_FUZZ_LAST_CASE=<path> to keep the case being run for crash (abort) triage.
    let last_case = std::env::var_os("OPEN365_FUZZ_LAST_CASE").map(PathBuf::from);
    let first_iter = env_u64("OPEN365_FUZZ_FROM", 0);

    // The harness must see panics, or "no findings" means nothing.
    let probe = guarded(|| {
        let v: Vec<u8> = Vec::new();
        let i = std::hint::black_box(3);
        let _ = v[i];
    });
    assert!(
        probe.is_some_and(|f| f.owner == "testkit"),
        "panic probe not caught"
    );

    // Sanity: unmutated fixtures must not panic.
    for (path, kind, bytes) in &fixtures {
        let f = guarded(|| {
            run_package(*kind, bytes);
            run_commands(*kind, bytes);
        });
        assert!(f.is_none(), "{}: {f:?}", path.display());
        assert!(!unzip(bytes).is_empty(), "{}", path.display());
    }

    let start = Instant::now();
    let deadline = Duration::from_secs(secs);
    let mut tally = Tally::default();
    let (mut iters, mut formula_iters, mut slow) = (first_iter, 0u64, 0u64);
    let (mut opened, mut package_cases) = (0u64, 0u64);
    let mut last_progress = Instant::now();
    let mut new_repros: Vec<PathBuf> = Vec::new();

    while start.elapsed() < deadline && iters - first_iter < max_iters {
        if last_progress.elapsed() > Duration::from_secs(30) {
            last_progress = Instant::now();
            eprintln!("progress: iteration {iters} at {:?}", start.elapsed());
        }
        let mut rng = Rng::new(seed.wrapping_add(iters.wrapping_mul(0x9E37_79B9_7F4A_7C15)));
        iters += 1;

        if rng.chance(6) {
            formula_iters += 1;
            let text = formula(&formulas[rng.below(formulas.len())], &mut rng);
            if let Some(p) = &last_case {
                let _ = std::fs::write(p, text.as_bytes());
            }
            let t = Instant::now();
            let found = guarded(|| run_formula(&text));
            if verbose && t.elapsed() > Duration::from_millis(50) {
                eprintln!("  {:?} formula len {}", t.elapsed(), text.len());
            }
            if let Some(f) = found
                && tally.record(&f)
            {
                eprintln!("PANIC {} {}", f.key(), f.message);
                let note = format!(
                    "seed: {seed}\niteration: {}\nformula follows in this file's data",
                    iters - 1
                );
                if let Some(p) = save_repro(&f, "formula", text.as_bytes(), &note) {
                    new_repros.push(p);
                }
            }
            continue;
        }

        let (path, kind, bytes) = &fixtures[rng.below(fixtures.len())];
        let tm = Instant::now();
        let case = mutate(bytes, &mut rng);
        if verbose && tm.elapsed() > Duration::from_millis(50) {
            eprintln!(
                "  {:?} mutate {} {}",
                tm.elapsed(),
                case.label,
                case.bytes.len()
            );
        }
        if let Some(p) = &last_case {
            let _ = std::fs::write(p, &case.bytes);
        }
        for stage in [Stage::Package(*kind), Stage::Commands(*kind)] {
            let t = Instant::now();
            let mut accepted = false;
            let found = guarded(|| accepted = stage.run(&case.bytes));
            opened += u64::from(accepted);
            if matches!(stage, Stage::Package(_)) {
                package_cases += 1;
            }
            if verbose && t.elapsed() > Duration::from_millis(50) {
                eprintln!(
                    "  {:?} {} {} {}",
                    t.elapsed(),
                    stage.name(),
                    case.label,
                    case.bytes.len()
                );
            }
            if t.elapsed() > Duration::from_secs(10) {
                slow += 1;
                eprintln!(
                    "SLOW {:?} {} {} (seed {seed} iter {})",
                    t.elapsed(),
                    stage.name(),
                    case.label,
                    iters - 1
                );
            }
            let Some(f) = found else { continue };
            if !tally.record(&f) {
                continue;
            }
            eprintln!(
                "PANIC {} {} [{} on {}] (seed {seed} iter {})",
                f.key(),
                f.message,
                case.label,
                path.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                iters - 1
            );
            let min = minimise(
                stage,
                &f.key(),
                &case.bytes,
                case.entries.as_deref(),
                Duration::from_secs(20),
            );
            let note = format!(
                "stage: {}\nseed: {seed}\niteration: {}\nmutation: {}\nfixture: {}\nsize: {} bytes (minimised from {})",
                stage.name(),
                iters - 1,
                case.label,
                path.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                min.len(),
                case.bytes.len()
            );
            if let Some(p) = save_repro(&f, kind.ext(), &min, &note) {
                new_repros.push(p);
            }
        }
    }

    eprintln!(
        "fuzz_smoke: {} iterations ({formula_iters} formula) in {:?}, seed {seed}, {} distinct panic site(s), {slow} slow case(s); {opened}/{package_cases} mutants still opened as valid documents",
        iters - first_iter,
        start.elapsed(),
        tally.by_key.len()
    );
    for (key, (f, n)) in &tally.by_key {
        eprintln!("  {n:>6}x {key}: {}", f.message);
    }
    for p in &new_repros {
        eprintln!("  repro: {}", p.display());
    }
    let failing: Vec<&Finding> = tally
        .by_key
        .values()
        .map(|(f, _)| f)
        .filter(|f| strict || OWNED_HERE.contains(&f.owner.as_str()))
        .collect();
    assert!(
        failing.is_empty(),
        "panics in code owned here: {failing:#?}"
    );
}
