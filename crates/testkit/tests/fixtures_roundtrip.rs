//! Every real-world fixture must survive read -> write -> read losslessly.

use std::fs;
use std::path::{Path, PathBuf};

use open365_opc::MainPartKind;
use open365_testkit::roundtrip;

/// Any main-part type, minus Office lock files (`~$name`), which are not packages.
fn is_package(path: &Path) -> bool {
    let lock = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_none_or(|n| n.starts_with("~$"));
    !lock
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| MainPartKind::ALL.iter().any(|k| k.extension() == e))
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if is_package(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn read_retry(path: &Path) -> std::io::Result<Vec<u8>> {
    fs::read(path).or_else(|_| fs::read(path))
}

#[test]
fn all_fixtures_roundtrip_losslessly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut files = Vec::new();
    collect(&root, &mut files).expect("walk fixtures");
    files.sort();
    assert!(!files.is_empty(), "no fixtures found under {root:?}");

    let mut failures = Vec::new();
    for path in &files {
        let bytes = read_retry(path).expect("read fixture");
        match roundtrip(&bytes) {
            Ok(report) if report.is_lossless() => {}
            Ok(report) => failures.push(format!("{}: {}", path.display(), report.describe())),
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    assert!(
        failures.is_empty(),
        "lossy fixtures:\n{}",
        failures.join("\n")
    );
}
