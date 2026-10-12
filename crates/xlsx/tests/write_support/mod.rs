//! Helpers shared by the `write_*` tests: reopening a saved workbook and checking that the package
//! is the kind Excel opens without a repair prompt (every content type, relationship target and
//! `r:id` resolves).
#![allow(dead_code)]

use std::collections::HashSet;

use open365_opc::{Limits, Package, Relationships, TargetMode, rels_part_name, resolve_target};
use open365_xlsx::Workbook;

pub fn open(bytes: &[u8]) -> Workbook {
    Workbook::open(bytes, &Limits::default()).expect("opens")
}

pub fn fixture(name: &str) -> Workbook {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/xlsx");
    open(&std::fs::read(dir.join(format!("{name}.xlsx"))).expect("fixture"))
}

/// Saves, checks the package and opens it again.
pub fn reopen(wb: &mut Workbook) -> Workbook {
    let bytes = wb.save().expect("saves");
    check_package(&bytes);
    open(&bytes)
}

pub fn text_of(wb: &mut Workbook, part: &str) -> String {
    let bytes = wb.save().expect("saves");
    let pkg = Package::read(&bytes, &Limits::default()).expect("package");
    String::from_utf8(
        pkg.part(part)
            .unwrap_or_else(|| panic!("no part {part}"))
            .data()
            .to_vec(),
    )
    .expect("utf-8")
}

pub fn has_part(wb: &mut Workbook, part: &str) -> bool {
    let bytes = wb.save().expect("saves");
    Package::read(&bytes, &Limits::default())
        .expect("package")
        .part(part)
        .is_some()
}

/// Every part has a content type, every internal relationship resolves, and every `r:id`,
/// `r:embed` or `r:link` in an XML part names a relationship of that part.
pub fn check_package(bytes: &[u8]) {
    let pkg = Package::read(bytes, &Limits::default()).expect("package reads");
    let issues = pkg.check_integrity();
    assert!(issues.is_empty(), "package integrity: {issues:?}");
    let names: Vec<String> = pkg.parts().map(|p| p.name().to_string()).collect();
    for name in &names {
        if name.ends_with(".rels") {
            let rels =
                Relationships::parse(pkg.part(name).expect("part").data()).expect("rels parse");
            let source = {
                let stem = name.strip_suffix(".rels").expect("suffix");
                if name == "_rels/.rels" {
                    None
                } else {
                    let (dir, file) = stem.rsplit_once('/').expect("dir");
                    let parent = dir
                        .strip_suffix("_rels")
                        .expect("rels dir")
                        .trim_end_matches('/');
                    Some(if parent.is_empty() {
                        file.to_owned()
                    } else {
                        format!("{parent}/{file}")
                    })
                }
            };
            let source = source.map(|s| open365_opc::PartName::new(&s).expect("name"));
            let mut ids = HashSet::new();
            for r in rels.iter() {
                assert!(
                    ids.insert(r.id.clone()),
                    "duplicate relationship id {} in {name}",
                    r.id
                );
                if r.target_mode == TargetMode::Internal {
                    let t = resolve_target(source.as_ref(), &r.target).expect("target resolves");
                    assert!(
                        pkg.part(t.as_str()).is_some(),
                        "{name}: {} -> missing {t}",
                        r.id
                    );
                }
            }
            let _ = rels_part_name(source.as_ref());
            continue;
        }
        if !(name.ends_with(".xml") || name.ends_with(".vml")) || name == "[Content_Types].xml" {
            continue;
        }
        let text = String::from_utf8_lossy(pkg.part(name).expect("part").data()).into_owned();
        let part = open365_opc::PartName::new(name).expect("name");
        let rels_name = rels_part_name(Some(&part));
        let rel_ids: HashSet<String> = pkg
            .part(rels_name.as_str())
            .and_then(|p| Relationships::parse(p.data()).ok())
            .map(|r| r.iter().map(|x| x.id.clone()).collect())
            .unwrap_or_default();
        for attr in ["r:id=\"", "r:embed=\"", "r:link=\""] {
            let mut rest = text.as_str();
            while let Some(i) = rest.find(attr) {
                let after = &rest[i + attr.len()..];
                let id = after.split('"').next().unwrap_or_default();
                assert!(
                    rel_ids.contains(id),
                    "{name}: {attr}{id}\" has no relationship"
                );
                rest = after;
            }
        }
    }
}
