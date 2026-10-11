//! Seeded mutations of the fixture corpus. Every mutated input must end as `Ok` or an error
//! value: no panic, no hang. Replay a failure with the printed fixture name and seed.

use open365_robustness::{Outcome, Rng, fixtures, mutate_package, run_in_process};

/// Mutations per fixture; raise with `ROBUST_CASES=200` for a longer soak.
fn cases() -> u64 {
    std::env::var("ROBUST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6)
}

fn soak(dir: &str, ext: &str, kind: &'static str) {
    let mut failures = Vec::new();
    let mut total = 0usize;
    for (name, bytes) in fixtures(dir, ext) {
        for seed in 0..cases() {
            let mut rng = Rng::new(seed ^ (name.len() as u64) << 32);
            let mutated = mutate_package(&mut rng, &bytes);
            total += 1;
            let outcome = run_in_process(kind, mutated);
            if !outcome.is_clean() {
                failures.push(format!("{name} seed {seed}: {outcome:?}"));
            }
        }
    }
    assert!(total > 0, "no fixtures found in {dir}");
    assert!(
        failures.is_empty(),
        "{} of {total} mutated inputs misbehaved:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn unmodified_fixtures_open() {
    for (dir, ext, kind) in [("docx", "docx", "docx"), ("xlsx", "xlsx", "xlsx")] {
        for (name, bytes) in fixtures(dir, ext) {
            let outcome = run_in_process(kind, bytes);
            assert!(
                matches!(outcome, Outcome::Accepted),
                "{name} should open: {outcome:?}"
            );
        }
    }
}

#[test]
fn mutated_docx_fixtures() {
    soak("docx", "docx", "docx");
}

#[test]
fn mutated_xlsx_fixtures() {
    soak("xlsx", "xlsx", "xlsx");
}

#[test]
fn raw_byte_flips_on_the_zip_container() {
    let (_, base) = fixtures("docx", "docx").remove(0);
    for seed in 0..200u64 {
        let mut rng = Rng::new(seed);
        let mut bytes = base.clone();
        for _ in 0..=rng.below(6) {
            let at = rng.below(bytes.len());
            bytes[at] ^= 1 << rng.below(8);
        }
        if rng.below(4) == 0 {
            bytes.truncate(rng.below(bytes.len()));
        }
        let outcome = run_in_process("package", bytes);
        assert!(outcome.is_clean(), "zip flip seed {seed}: {outcome:?}");
    }
}

#[test]
fn formula_mutations() {
    let seeds = [
        "SUM(A1:B2)*IF(A1>1,\"x\",2)&TEXT(A1,\"0.00\")",
        "INDEX(A1:C10,MATCH(5,A1:A10,0),2)",
        "SUMPRODUCT((A1:A10>3)*(B1:B10))",
        "LET(x,A1,x+1)",
        "{1,2;3,4}+A1",
        "'S'!A1+[1]S!B2+Table1[[#This Row],[a]]",
    ];
    let mut failures = Vec::new();
    for (i, seed) in seeds.iter().enumerate() {
        for n in 0..300u64 {
            let mut rng = Rng::new(n + 1000 * i as u64);
            let text = open365_robustness::mutate(&mut rng, seed.as_bytes());
            let outcome = run_in_process("formula", text.clone());
            if !outcome.is_clean() {
                failures.push(format!("{:?}: {outcome:?}", String::from_utf8_lossy(&text)));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
