//! Benchmark-style budgets for package round trips (see `docs/testing/performance.md`).
//!
//! The budgets are deliberately loose (roughly 5-10x the numbers measured on the reference
//! machine) so they catch accidental quadratic behaviour, not noise. Debug builds are
//! ~10-20x slower, so the budgets apply to release builds only; debug builds still run the
//! workload and check the result, just without the clock.
//!
//! `OPEN365_PERF_DUMP=<path>` writes the generated package there (for profiling the CLI).

use std::io::{Cursor, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use open365_opc::{Limits, Package};
use open365_testkit::{inspect, roundtrip};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Entry count of the stress package: just under `Limits::max_parts` (20 000).
const ENTRIES: usize = 19_000;

fn big_package(entries: usize) -> Vec<u8> {
    use std::fmt::Write as _;
    let media = entries - 4;
    let mut types = String::from(
        "<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"png\" ContentType=\"image/png\"/>\
         <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>",
    );
    let mut rels = String::from(
        "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for i in 0..media {
        let _ = write!(
            types,
            "<Override PartName=\"/word/media/image{i}.png\" ContentType=\"image/png\"/>"
        );
        let _ = write!(
            rels,
            "<Relationship Id=\"rId{i}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/image{i}.png\"/>"
        );
    }
    types.push_str("</Types>");
    rels.push_str("</Relationships>");
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, data: &[u8], method: CompressionMethod| {
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(method),
            )
            .expect("perf setup");
        writer.write_all(data).expect("perf setup");
    };
    add(
        "[Content_Types].xml",
        types.as_bytes(),
        CompressionMethod::Deflated,
    );
    add(
        "_rels/.rels",
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>",
        CompressionMethod::Deflated,
    );
    add(
        "word/document.xml",
        b"<w:document/>",
        CompressionMethod::Deflated,
    );
    add(
        "word/_rels/document.xml.rels",
        rels.as_bytes(),
        CompressionMethod::Deflated,
    );
    for i in 0..media {
        add(
            &format!("word/media/image{i}.png"),
            &(i as u64).to_le_bytes(),
            CompressionMethod::Stored,
        );
    }
    writer.finish().expect("perf setup").into_inner()
}

fn timed<T>(label: &str, f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    let took = start.elapsed();
    println!("perf: {label}: {took:?}");
    (out, took)
}

fn within(label: &str, took: Duration, budget: Duration) {
    if !cfg!(debug_assertions) {
        assert!(took <= budget, "{label} took {took:?}, budget {budget:?}");
    }
}

#[test]
fn nineteen_thousand_entries_roundtrip_within_budget() {
    let bytes = big_package(ENTRIES);
    if let Ok(path) = std::env::var("OPEN365_PERF_DUMP") {
        std::fs::write(path, &bytes).expect("perf setup");
    }

    let (package, read) = timed("read 19000 entries", || {
        Package::read(&bytes, &Limits::default()).expect("perf setup")
    });
    assert_eq!(package.parts().count(), ENTRIES);
    let (issues, integrity) = timed("check_integrity", || package.check_integrity());
    assert!(issues.is_empty(), "{issues:?}");
    let (written, write) = timed("write", || package.write().expect("perf setup"));
    assert_eq!(written.len(), bytes.len());
    let (lookup_hits, lookups) = timed("19000 case-insensitive lookups", || {
        (0..ENTRIES - 4)
            .filter(|i| package.part(&format!("WORD/Media/Image{i}.PNG")).is_some())
            .count()
    });
    assert_eq!(lookup_hits, ENTRIES - 4);
    let (report, total) = timed("roundtrip", || roundtrip(&bytes).expect("perf setup"));
    assert!(report.is_lossless(), "{}", report.describe());
    assert_eq!(report.part_count, ENTRIES);
    let (info, inspected) = timed("inspect", || inspect(&bytes).expect("perf setup"));
    assert_eq!(info.parts.len(), ENTRIES);

    within("read", read, Duration::from_millis(500));
    within("check_integrity", integrity, Duration::from_millis(500));
    within("write", write, Duration::from_millis(500));
    within("lookups", lookups, Duration::from_millis(100));
    within("roundtrip", total, Duration::from_secs(1));
    within("inspect", inspected, Duration::from_secs(1));
}

#[test]
fn large_sheet_fixture_roundtrips_within_budget() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/xlsx/large-sheet.xlsx");
    let bytes = std::fs::read(path).expect("perf setup");
    let (report, took) = timed("roundtrip large-sheet.xlsx", || {
        roundtrip(&bytes).expect("perf setup")
    });
    assert!(report.is_lossless(), "{}", report.describe());
    within("large-sheet roundtrip", took, Duration::from_secs(3));
}
