//! Invariants over every Excel-made fixture in `fixtures/xlsx`.

mod common;

use std::time::Instant;

use open365_opc::{Limits, Package};
use open365_xlsx::{SheetKind, Workbook};

use common::{assert_infoset_eq, fixtures};

fn open(bytes: &[u8]) -> Workbook {
    Workbook::open(bytes, &Limits::default()).expect("fixture opens")
}

#[test]
fn fixtures_exist() {
    assert!(
        fixtures().len() >= 16,
        "expected the Excel-made fixture set"
    );
}

#[test]
fn open_succeeds_and_nothing_is_dropped() {
    for (name, bytes) in fixtures() {
        let wb = open(&bytes);
        assert!(wb.dropped().is_empty(), "{name}: {:?}", wb.dropped());
        assert!(!wb.sheets().is_empty(), "{name}: no sheets");
        assert!(!wb.is_dirty(), "{name}: dirty after open");
        assert!(wb.hygiene_log().is_empty(), "{name}: hygiene before edit");
    }
}

#[test]
fn unedited_save_is_byte_identical() {
    for (name, bytes) in fixtures() {
        let original = Package::read(&bytes, &Limits::default()).expect("package");
        let mut wb = open(&bytes);
        let saved = wb.save().expect("saves");
        let saved = Package::read(&saved, &Limits::default()).expect("saved package");
        let a: Vec<_> = original
            .parts()
            .map(|p| (p.name().to_string(), p.data().to_vec(), p.compression()))
            .collect();
        let b: Vec<_> = saved
            .parts()
            .map(|p| (p.name().to_string(), p.data().to_vec(), p.compression()))
            .collect();
        assert_eq!(a.len(), b.len(), "{name}: part count");
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.0, y.0, "{name}: part order");
            assert!(x.1 == y.1, "{name}: {} bytes differ", x.0);
            assert_eq!(x.2, y.2, "{name}: {} compression", x.0);
        }
    }
}

#[test]
fn reserialised_parts_are_infoset_equal() {
    for (name, bytes) in fixtures() {
        let wb = open(&bytes);
        let package = Package::read(&bytes, &Limits::default()).expect("package");
        let check = |label: &str, part: &str, rewritten: &[u8]| {
            let original = package.part(part).unwrap_or_else(|| panic!("{part}"));
            if let Err(diff) = assert_infoset_eq(original.data(), rewritten) {
                panic!("{name}: {label} {part}: {diff}");
            }
        };
        check(
            "workbook",
            wb.workbook_part().as_str(),
            &wb.serialize_workbook(),
        );
        for (i, info) in wb.sheets().iter().enumerate() {
            if info.kind == SheetKind::Worksheet {
                let part = wb.sheet_part(i).expect("part").to_string();
                check(
                    "sheet",
                    &part,
                    &wb.serialize_sheet(i).expect("worksheet serialises"),
                );
            }
        }
        if let Some(sst) = wb.serialize_shared_strings() {
            let part = package
                .parts()
                .find(|p| p.name().as_str().ends_with("sharedStrings.xml"))
                .expect("sst part");
            check("sst", part.name().as_str(), &sst);
        }
        if let Some(styles) = wb.serialize_styles() {
            check("styles", "xl/styles.xml", &styles);
        }
    }
}

#[test]
fn sheet_states_and_kinds_come_from_the_workbook() {
    let wb = open(&common::fixture("hidden-sheet"));
    let states: Vec<_> = wb
        .sheets()
        .iter()
        .map(|s| format!("{:?}", s.state))
        .collect();
    assert_eq!(states, ["Visible", "Hidden", "VeryHidden"]);
    assert!(wb.sheets().iter().all(|s| s.kind == SheetKind::Worksheet));
    assert!(open(&common::fixture("dates-1904")).date1904());
    assert!(!open(&common::fixture("dates-1900")).date1904());
}

#[test]
fn defined_names_and_number_formats_are_readable() {
    let wb = open(&common::fixture("defined-names"));
    assert!(!wb.defined_names().is_empty());
    for n in wb.defined_names() {
        assert!(!n.name.is_empty() && !n.formula.is_empty(), "{n:?}");
    }

    let wb = open(&common::fixture("number-formats"));
    let formats: Vec<&str> = (1..=7).map(|r| wb.number_format(0, r, 1)).collect();
    assert_eq!(
        formats,
        [
            "\"$\"#,##0.00",
            "0.0%",
            r"0.00;[Red]\-0.00",
            r"0.00;[Red]\-0.00",
            "mm-dd-yy",
            r"dddd\,\ mmmm\ d\,\ yyyy",
            "h:mm AM/PM",
        ]
    );
    // Cells that do not exist have the default style.
    assert_eq!(wb.number_format(0, 99, 99), "General");
}

#[test]
fn large_sheet_opens_fast_and_keeps_every_cell() {
    let bytes = common::fixture("large-sheet");
    let started = Instant::now();
    let wb = open(&bytes);
    let elapsed = started.elapsed();
    let cells = wb.iter_cells(0).count();
    assert_eq!(cells, 50_000 * 11);
    assert_eq!(
        wb.sheet(0).expect("sheet").used_range(),
        Some((1, 1, 50_000, 11))
    );
    eprintln!("large-sheet: opened {cells} cells in {elapsed:?}");
    assert!(
        elapsed.as_secs_f64() < 2.0,
        "open took {elapsed:?} (budget 2s in debug)"
    );
}
