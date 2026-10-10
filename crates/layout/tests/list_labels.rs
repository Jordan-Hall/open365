//! List labels against Word: `fixtures/reference/layout/<name>.lists.json` holds
//! `Range.ListFormat.ListString` of every paragraph, written by
//! `tools/office/make-layout-fixtures.ps1` while it builds the fixture.

use std::fs;
use std::path::Path;

use open365_docx::Document;
use open365_layout::list_labels;
use open365_opc::Limits;
use serde_json::Value;

const FIXTURES: [&str; 5] = [
    "layout-lists-style",
    "layout-lists-formats",
    "layout-lists-multilevel",
    "layout-lists-suff",
    "numbering",
];

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn list_labels_match_word() {
    let mut failures = Vec::new();
    for name in FIXTURES {
        let ref_path = root().join(format!("fixtures/reference/layout/{name}.lists.json"));
        let Ok(text) = fs::read_to_string(&ref_path) else {
            // Fixtures made before the oracle dumped labels have no reference.
            assert_eq!(name, "numbering", "missing {ref_path:?}");
            continue;
        };
        let reference: Value = serde_json::from_str(&text).expect("parse lists json");
        let bytes = fs::read(root().join(format!("fixtures/docx/{name}.docx"))).expect("docx");
        let doc = Document::open(&bytes, &Limits::default()).expect("open docx");
        let ours = list_labels(&doc);
        for row in reference["paragraphs"].as_array().expect("paragraphs") {
            let index = row["paragraph"].as_u64().expect("paragraph") as usize;
            let want = row["label"].as_str().unwrap_or_default();
            let got = ours.get(index).and_then(Option::as_deref).unwrap_or("");
            if got != want {
                failures.push(format!(
                    "{name} paragraph {index}: want {want:?}, got {got:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
