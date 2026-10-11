//! Built-in `numFmtId` codes per LCID: `FormatLocale::builtin_format_code` against the XLSX reader's table.
//!
//! `fixtures/calc/numfmt-builtin/xlsx-builtin-all-lcids.tsv` is `open365_xlsx::builtin_number_format_in(id, lcid)` for ids 0
//! to 100 and every LCID of `open365_locale::Locale::all_lcids()` (`lcid<TAB>id<TAB>code`, `\u{2205}` for "not built in").
//! calc must not depend on xlsx, so the xlsx side is a recorded fixture. Regenerate it with a throwaway binary that calls
//! `open365_xlsx::builtin_number_format_in`; after xlsx re-exports `open365_locale`'s function the two cannot differ.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use open365_calc::FormatLocale;
use open365_locale::Locale;

const GOLDEN: &str =
    include_str!("../../../fixtures/calc/numfmt-builtin/xlsx-builtin-all-lcids.tsv");

fn golden() -> BTreeMap<(u32, u32), String> {
    GOLDEN
        .lines()
        .map(|line| {
            let mut f = line.splitn(3, '\t');
            let lcid = u32::from_str_radix(f.next().unwrap(), 16).unwrap();
            let id: u32 = f.next().unwrap().parse().unwrap();
            (
                (lcid, id),
                f.next()
                    .unwrap_or_default()
                    .trim_end_matches('\r')
                    .to_owned(),
            )
        })
        .collect()
}

#[test]
fn calc_agrees_with_xlsx_for_every_lcid_and_id() {
    let golden = golden();
    let mut lcids: Vec<u32> = Locale::all_lcids().map(|(l, _)| l).collect();
    lcids.sort_unstable();
    lcids.dedup();
    assert!(lcids.len() >= 417, "{} LCIDs", lcids.len());
    let mut compared = 0;
    let mut bad = Vec::new();
    for lcid in &lcids {
        let locale =
            FormatLocale::for_lcid(*lcid).unwrap_or_else(|| panic!("no locale {lcid:04X}"));
        assert_eq!(locale.lcid, Some(*lcid));
        for id in 0..=100u32 {
            let want = &golden[&(*lcid, id)];
            let got = locale
                .builtin_format_code(id)
                .unwrap_or_else(|| "\u{2205}".to_owned());
            compared += 1;
            if &got != want {
                bad.push(format!("{lcid:04X} id {id}: calc {got:?}, xlsx {want:?}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {compared} differ:\n{}",
        bad.len(),
        bad[..bad.len().min(20)].join("\n")
    );
    assert_eq!(compared, lcids.len() * 101);
}

#[test]
fn spot_checks() {
    let code = |lcid: u32, id: u32| {
        FormatLocale::for_lcid(lcid)
            .unwrap()
            .builtin_format_code(id)
    };
    assert_eq!(code(0x0409, 14).as_deref(), Some("m/d/yyyy"));
    assert_eq!(code(0x0809, 14).as_deref(), Some("dd/mm/yyyy"));
    assert_eq!(code(0x0809, 15).as_deref(), Some("dd-mmm-yy"));
    assert_eq!(code(0x0411, 27).as_deref(), Some("[$-411]ge.m.d"));
    assert_eq!(code(0x041E, 59).as_deref(), Some("t0"));
    assert_eq!(code(0x041E, 81).as_deref(), Some("d/m/bb"));
    assert_eq!(
        code(0x0804, 31).as_deref(),
        Some("yyyy\"年\"m\"月\"d\"日\"")
    );
    assert_eq!(code(0x0809, 23), None);
    assert_eq!(code(0x0809, 82), None);
}
