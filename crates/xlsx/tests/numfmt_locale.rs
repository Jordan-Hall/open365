//! Locale-dependent built-in number formats (ECMA-376 Part 1, 18.8.30).
//!
//! * The East Asian and Thai tables are the public specification's, checked against its own "unicode values"
//!   listing (written here with `\u` escapes, independently of the source).
//! * The other rows are **derived**: the rules measured under en-GB (see `numfmt_locale_oracle.rs`) applied to
//!   the Windows locale data. Excel can be measured only in the machine's own locale, so these rows are
//!   regression anchors for the derivation, not Excel measurements.

#![allow(clippy::unwrap_used)]

use open365_calc::numfmt::validate_format;
use open365_locale::Locale;
use open365_xlsx::{
    builtin_number_format_in, builtin_number_format_lcids, nls_pattern_to_excel,
    resolve_number_format_lcid,
};
use serde_json::Value;

const ZH_TW: u32 = 0x0404;
const ZH_CN: u32 = 0x0804;
const JA_JP: u32 = 0x0411;
const KO_KR: u32 = 0x0412;
const TH_TH: u32 = 0x041E;

fn code(id: u32, lcid: u32) -> String {
    builtin_number_format_in(id, lcid).unwrap_or_else(|| panic!("no code for {id} in {lcid:#x}"))
}

/// Glyphs of the specification's tables.
const NIAN: &str = "\u{5E74}"; // year
const YUE: &str = "\u{6708}"; // month
const RI: &str = "\u{65E5}"; // day
const SHI_TW: &str = "\u{6642}"; // hour (traditional)
const SHI_CN: &str = "\u{65F6}"; // hour (simplified)
const FEN: &str = "\u{5206}"; // minute
const MIAO: &str = "\u{79D2}"; // second
const AM_PM: &str = "\u{4E0A}\u{5348}/\u{4E0B}\u{5348}";

fn check(lcid: u32, tag: &str, expected: Vec<(u32, String)>) {
    for (id, want) in expected {
        assert_eq!(code(id, lcid), want, "{tag} id {id}");
    }
}

#[test]
fn zh_tw_table() {
    let ymd = format!("[$-404]e\"{NIAN}\"m\"{YUE}\"d\"{RI}\"");
    let hm = format!("{AM_PM}hh\"{SHI_TW}\"mm\"{FEN}\"");
    let hms = format!("{AM_PM}hh\"{SHI_TW}\"mm\"{FEN}\"ss\"{MIAO}\"");
    let expected: Vec<(u32, String)> = vec![
        (27, "[$-404]e/m/d".into()),
        (28, ymd.clone()),
        (29, ymd.clone()),
        (30, "m/d/yy".into()),
        (31, format!("yyyy\"{NIAN}\"m\"{YUE}\"d\"{RI}\"")),
        (32, format!("hh\"{SHI_TW}\"mm\"{FEN}\"")),
        (33, format!("hh\"{SHI_TW}\"mm\"{FEN}\"ss\"{MIAO}\"")),
        (34, hm.clone()),
        (35, hms.clone()),
        (36, "[$-404]e/m/d".into()),
        (50, "[$-404]e/m/d".into()),
        (51, ymd.clone()),
        (52, hm.clone()),
        (53, hms.clone()),
        (54, ymd.clone()),
        (55, hm),
        (56, hms),
        (57, "[$-404]e/m/d".into()),
        (58, ymd),
    ];
    check(ZH_TW, "zh-TW", expected.clone());
    check(0x0C04, "zh-HK", expected);
}

#[test]
fn zh_cn_table() {
    let ym = format!("yyyy\"{NIAN}\"m\"{YUE}\"");
    let md = format!("m\"{YUE}\"d\"{RI}\"");
    let hm = format!("{AM_PM}h\"{SHI_CN}\"mm\"{FEN}\"");
    let hms = format!("{AM_PM}h\"{SHI_CN}\"mm\"{FEN}\"ss\"{MIAO}\"");
    let expected: Vec<(u32, String)> = vec![
        (27, ym.clone()),
        (28, md.clone()),
        (29, md.clone()),
        (30, "m-d-yy".into()),
        (31, format!("yyyy\"{NIAN}\"m\"{YUE}\"d\"{RI}\"")),
        (32, format!("h\"{SHI_CN}\"mm\"{FEN}\"")),
        (33, format!("h\"{SHI_CN}\"mm\"{FEN}\"ss\"{MIAO}\"")),
        (34, hm.clone()),
        (35, hms.clone()),
        (36, ym.clone()),
        (50, ym.clone()),
        (51, md.clone()),
        (52, ym.clone()),
        (53, md.clone()),
        (54, md.clone()),
        (55, hm),
        (56, hms),
        (57, ym),
        (58, md),
    ];
    check(ZH_CN, "zh-CN", expected.clone());
    check(0x1004, "zh-SG", expected);
}

#[test]
fn ja_jp_table() {
    let wareki = format!("[$-411]ggge\"{NIAN}\"m\"{YUE}\"d\"{RI}\"");
    let ym = format!("yyyy\"{NIAN}\"m\"{YUE}\"");
    let md = format!("m\"{YUE}\"d\"{RI}\"");
    let expected: Vec<(u32, String)> = vec![
        (27, "[$-411]ge.m.d".into()),
        (28, wareki.clone()),
        (29, wareki.clone()),
        (30, "m/d/yy".into()),
        (31, format!("yyyy\"{NIAN}\"m\"{YUE}\"d\"{RI}\"")),
        (32, format!("h\"{SHI_TW}\"mm\"{FEN}\"")),
        (33, format!("h\"{SHI_TW}\"mm\"{FEN}\"ss\"{MIAO}\"")),
        (34, ym.clone()),
        (35, md.clone()),
        (36, "[$-411]ge.m.d".into()),
        (50, "[$-411]ge.m.d".into()),
        (51, wareki.clone()),
        (52, ym.clone()),
        (53, md.clone()),
        (54, wareki.clone()),
        (55, ym),
        (56, md),
        (57, "[$-411]ge.m.d".into()),
        (58, wareki),
    ];
    check(JA_JP, "ja-JP", expected);
}

#[test]
fn ko_kr_table() {
    let long = format!("yyyy\"{NIAN}\" mm\"{YUE}\" dd\"{RI}\"");
    let expected: Vec<(u32, String)> = vec![
        (27, long.clone()),
        (28, "mm-dd".into()),
        (29, "mm-dd".into()),
        (30, "mm-dd-yy".into()),
        (31, "yyyy\"\u{B144}\" mm\"\u{C6D4}\" dd\"\u{C77C}\"".into()),
        (32, "h\"\u{C2DC}\" mm\"\u{BD84}\"".into()),
        (33, "h\"\u{C2DC}\" mm\"\u{BD84}\" ss\"\u{CD08}\"".into()),
        (34, "yyyy-mm-dd".into()),
        (35, "yyyy-mm-dd".into()),
        (36, long.clone()),
        (50, long.clone()),
        (51, "mm-dd".into()),
        (52, "yyyy-mm-dd".into()),
        (53, "yyyy-mm-dd".into()),
        (54, "mm-dd".into()),
        (55, "yyyy-mm-dd".into()),
        (56, "yyyy-mm-dd".into()),
        (57, long),
        (58, "mm-dd".into()),
    ];
    check(KO_KR, "ko-KR", expected);
}

#[test]
fn th_th_table() {
    // Thai letters of the specification: wo waen U+0E27, do dek U+0E14, po pla U+0E1B, cho chang U+0E0A,
    // no nu U+0E19, tho thahan U+0E17.
    let (w, d, p, c, n, t) = (
        "\u{0E27}", "\u{0E14}", "\u{0E1B}", "\u{0E0A}", "\u{0E19}", "\u{0E17}",
    );
    let expected: Vec<(u32, String)> = vec![
        (59, "t0".into()),
        (60, "t0.00".into()),
        (61, "t#,##0".into()),
        (62, "t#,##0.00".into()),
        (67, "t0%".into()),
        (68, "t0.00%".into()),
        (69, "t# ?/?".into()),
        (70, "t# ??/??".into()),
        (71, format!("{w}/{d}/{p}{p}{p}{p}")),
        (72, format!("{w}-{d}{d}{d}-{p}{p}")),
        (73, format!("{w}-{d}{d}{d}")),
        (74, format!("{d}{d}{d}-{p}{p}")),
        (75, format!("{c}:{n}{n}")),
        (76, format!("{c}:{n}{n}:{t}{t}")),
        (77, format!("{w}/{d}/{p}{p}{p}{p} {c}:{n}{n}")),
        (78, format!("{n}{n}:{t}{t}")),
        (79, format!("[{c}]:{n}{n}:{t}{t}")),
        (80, format!("{n}{n}:{t}{t}.0")),
        (81, "d/m/bb".into()),
    ];
    check(TH_TH, "th-TH", expected);
    // The Thai table leaves 63 to 66 to the currency rule.
    assert_eq!(code(63, TH_TH), code(5, TH_TH));
}

/// (locale, tag, id, code): derived rows for the Western, Cyrillic and other locales.
#[test]
fn derived_per_locale_table() {
    let rows: &[(u32, &str, u32, &str)] = &[
        // en-US: the codes Excel writes for the US (they equal the specification's table where it has one).
        (0x0409, "en-US", 5, "\"$\"#,##0_);\\(\"$\"#,##0\\)"),
        (
            0x0409,
            "en-US",
            8,
            "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)",
        ),
        (0x0409, "en-US", 14, "m/d/yyyy"),
        (0x0409, "en-US", 15, "d-mmm-yy"),
        (0x0409, "en-US", 20, "h:mm"),
        (0x0409, "en-US", 22, "m/d/yyyy h:mm"),
        (0x0409, "en-US", 37, "#,##0_);(#,##0)"),
        (0x0409, "en-US", 40, "#,##0.00_);[Red](#,##0.00)"),
        (
            0x0409,
            "en-US",
            41,
            "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)",
        ),
        (
            0x0409,
            "en-US",
            44,
            "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)",
        ),
        // en-GB
        (0x0809, "en-GB", 7, "\u{a3}#,##0.00;-\u{a3}#,##0.00"),
        (0x0809, "en-GB", 14, "dd/mm/yyyy"),
        (0x0809, "en-GB", 22, "dd/mm/yyyy hh:mm"),
        // de-DE: suffix currency, minus sign, day first with dots
        (0x0407, "de-DE", 5, "#,##0\\ \u{20ac};-#,##0\\ \u{20ac}"),
        (0x0407, "de-DE", 14, "dd.mm.yyyy"),
        (0x0407, "de-DE", 21, "hh:mm:ss"),
        (
            0x0407,
            "de-DE",
            44,
            "_-* #,##0.00\\ \u{20ac}_-;-* #,##0.00\\ \u{20ac}_-;_-* \"-\"??\\ \u{20ac}_-;_-@_-",
        ),
        // nl-NL: prefix currency with a space
        (0x0413, "nl-NL", 14, "d-m-yyyy"),
        (0x0413, "nl-NL", 5, "\u{20ac}\\ #,##0;\u{20ac}\\ -#,##0"),
        // ja-JP: the system short date is year first; ids outside the East Asian table follow the rules
        (0x0411, "ja-JP", 14, "yyyy/mm/dd"),
        (0x0411, "ja-JP", 71, "yyyy/mm/dd"),
        (0x0411, "ja-JP", 5, "\u{a5}#,##0;-\u{a5}#,##0"),
        (0x0804, "zh-CN", 14, "yyyy/m/d"),
        (0x0412, "ko-KR", 14, "yyyy-mm-dd"),
        (0x0419, "ru-RU", 14, "dd.mm.yyyy"),
        (0x0419, "ru-RU", 20, "h:mm"),
        // Locales without an East Asian table: 27 to 31, 36, 50 to 58 are the short date, 32 to 35 the time
        (0x0407, "de-DE", 27, "dd.mm.yyyy"),
        (0x0407, "de-DE", 34, "hh:mm:ss"),
        (0x0407, "de-DE", 58, "dd.mm.yyyy"),
        (0x0409, "en-US", 34, "h:mm:ss AM/PM"),
    ];
    for (lcid, tag, id, want) in rows {
        assert_eq!(code(*id, *lcid), *want, "{tag} id {id}");
    }
}

#[test]
fn ids_that_are_not_built_in() {
    for lcid in [0x0409, 0x0407, JA_JP, TH_TH, ZH_CN] {
        for id in (23..=26).chain(82..=200) {
            assert_eq!(builtin_number_format_in(id, lcid), None, "id {id}");
        }
    }
}

#[test]
fn every_locale_of_the_lcid_table_resolves_every_id_to_a_valid_code() {
    let mut checked = 0;
    for (lcid, tag) in Locale::all_lcids() {
        for id in (0..=22).chain(27..=81) {
            let code = builtin_number_format_in(id, lcid)
                .unwrap_or_else(|| panic!("{tag} ({lcid:#x}) has no code for id {id}"));
            // The formula engine has no Thai digit or Thai date letters yet (reported, not fixed here).
            if lcid & 0x3FF == 0x1E && id >= 59 && id != 63 && !(64..=66).contains(&id) {
                continue;
            }
            assert!(
                validate_format(&code).is_ok(),
                "{tag} ({lcid:#x}) id {id}: {code:?} does not parse"
            );
        }
        checked += 1;
    }
    assert!(checked > 400, "{checked} locales");
}

#[test]
fn locales_with_their_own_data_cover_the_lcid_table() {
    let own: Vec<u32> = builtin_number_format_lcids().collect();
    assert!(own.len() >= 400, "{} data rows", own.len());
    let missing: Vec<String> = Locale::all_lcids()
        .filter(|(lcid, _)| !own.contains(lcid))
        .map(|(lcid, tag)| format!("{tag} ({lcid:#x})"))
        .collect();
    // The few LCIDs this Windows version does not ship fall back to their language.
    assert!(missing.len() <= 5, "no data row for {missing:?}");
    for (lcid, _) in Locale::all_lcids() {
        assert!(own.contains(&resolve_number_format_lcid(lcid)));
    }
}

#[test]
fn the_rust_table_matches_the_committed_windows_data() {
    let path = format!(
        "{}/../../fixtures/reference/xlsx/numfmt-locale/windows-nls.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let json: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(json["label"], "derived");
    let rows = json["locales"].as_array().unwrap();
    let own: Vec<u32> = builtin_number_format_lcids().collect();
    assert_eq!(rows.len(), own.len());
    for row in rows {
        let lcid = u32::try_from(row["lcid"].as_u64().unwrap()).unwrap();
        assert_eq!(resolve_number_format_lcid(lcid), lcid);
        let short_date = nls_pattern_to_excel(row["shortDate"].as_str().unwrap());
        if !matches!(lcid & 0x3FF, 0x04 | 0x11 | 0x12 | 0x1E) {
            assert_eq!(code(14, lcid), short_date, "{}", row["tag"]);
        }
    }
}
