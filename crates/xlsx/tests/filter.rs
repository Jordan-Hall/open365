//! AutoFilter against Excel: the `autoFilter` element Excel saved for each scenario of
//! `fixtures/reference/xlsx/sortfilter-filter/scenarios.json` is read and evaluated on the "before" sheet of
//! `fixtures/xlsx/sortfilter-filter.xlsx`; the rows it hides must be the rows Excel hid.

use std::path::PathBuf;

use open365_locale::Locale;
use open365_opc::Limits;
use open365_xlsx::{
    AutoFilter, ColorKey, CustomFilter, CustomFilters, FilterColumn, FilterContext, FilterCriteria,
    FilterOperator, Rgb, Workbook, parse_xml,
};
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load() -> (Workbook, Value) {
    let bytes =
        std::fs::read(root().join("fixtures/xlsx/sortfilter-filter.xlsx")).expect("fixture");
    let wb = Workbook::open(&bytes, &Limits::default()).expect("opens");
    let json = std::fs::read_to_string(
        root().join("fixtures/reference/xlsx/sortfilter-filter/scenarios.json"),
    )
    .expect("reference");
    (
        wb,
        serde_json::from_str(json.trim_start_matches('\u{feff}')).expect("json"),
    )
}

fn wrap(snippet: &str) -> Vec<u8> {
    format!(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
         xmlns:xr=\"http://schemas.microsoft.com/office/spreadsheetml/2014/revision\">{snippet}</worksheet>"
    )
    .into_bytes()
}

fn read_filter(snippet: &str) -> AutoFilter {
    let parsed = parse_xml(&wrap(snippet)).expect("parses");
    let el = parsed.root.child_main("autoFilter").expect("autoFilter");
    AutoFilter::from_element(el).expect("filter")
}

/// The colour each `dxf` of Excel's `dxfs` element stands for.
fn dxf_colors(dxfs: &str) -> Vec<ColorKey> {
    let parsed = parse_xml(&wrap(dxfs)).expect("dxfs");
    let list = parsed.root.child_main("dxfs").expect("dxfs");
    list.elements()
        .map(|dxf| {
            let fill = dxf
                .child_main("fill")
                .and_then(|f| f.child_main("patternFill"));
            let Some(fill) = fill else {
                return ColorKey::Automatic;
            };
            let fg = fill.child_main("fgColor");
            match (
                fill.head.attr("patternType"),
                fg.and_then(|c| c.head.attr("rgb")),
            ) {
                (Some("none"), _) | (_, None) => ColorKey::Automatic,
                (_, Some(rgb)) => {
                    let n = u32::from_str_radix(&rgb[2..], 16).expect("hex");
                    ColorKey::Rgb(Rgb::new((n >> 16) as u8, (n >> 8) as u8, n as u8))
                }
            }
        })
        .collect()
}

#[test]
fn filters_hide_the_rows_excel_hid() {
    let (wb, json) = load();
    let locale = Locale::from_bcp47("en-GB").expect("locale");
    let colors = dxf_colors(json["dxfs"].as_str().expect("dxfs"));
    let lookup = |id: u32, _font: bool| colors.get(id as usize).copied().unwrap_or_default();
    let scenarios = json["scenarios"].as_array().expect("scenarios");
    let (mut matched, mut failures) = (0, Vec::new());
    for sc in scenarios {
        let id = sc["id"].as_str().expect("id");
        let sheet = wb.sheet_index(id).expect("sheet");
        let filter = read_filter(sc["auto_filter"].as_str().expect("filter"));
        let mut ctx =
            FilterContext::new(wb.styles(), &locale, sc["today"].as_f64().expect("today"));
        ctx.dxf_color = Some(&lookup);
        let outcome = filter
            .evaluate(wb.sheet(sheet).expect("sheet"), &ctx)
            .expect(id);
        let (r1, _, r2, _) = filter.range;
        let expected: Vec<u32> = sc["hidden_rows"]
            .as_array()
            .expect("hidden")
            .iter()
            .map(|r| r.as_u64().expect("row") as u32)
            .filter(|r| (r1 + 1..=r2).contains(r))
            .collect();
        if outcome.hidden == expected {
            matched += 1;
        } else {
            eprintln!("{id}: excel {expected:?}, got {:?}", outcome.hidden);
            failures.push(id.to_owned());
        }
    }
    eprintln!(
        "filter: {matched}/{} scenarios match Excel",
        scenarios.len()
    );
    assert!(failures.is_empty(), "differs from Excel: {failures:?}");
}

#[test]
fn every_excel_filter_element_round_trips() {
    let (_, json) = load();
    for sc in json["scenarios"].as_array().expect("scenarios") {
        let snippet = sc["auto_filter"].as_str().expect("filter");
        let parsed = parse_xml(&wrap(snippet)).expect("parses");
        let el = parsed.root.child_main("autoFilter").expect("autoFilter");
        let filter = AutoFilter::from_element(el).expect("filter");
        let written = filter.to_element(&el.head);
        assert_eq!(
            AutoFilter::from_element(&written).expect("again"),
            filter,
            "{}",
            sc["id"]
        );
        assert_eq!(&written, el, "{}", sc["id"]);
    }
}

#[test]
fn a_filter_built_in_code_hides_rows_and_survives_a_save() {
    let (mut wb, json) = load();
    let locale = Locale::from_bcp47("en-GB").expect("locale");
    let sheet = wb.sheet_index("custom-gt").expect("sheet");
    let mut filter = AutoFilter::new((1, 1, 21, 5));
    filter.columns.push(FilterColumn::new(
        1,
        FilterCriteria::Custom(CustomFilters {
            and: false,
            filters: vec![CustomFilter {
                operator: FilterOperator::GreaterThan,
                value: "20".to_owned(),
            }],
        }),
    ));
    wb.set_auto_filter(sheet, Some(&filter)).expect("sets");
    let today = json["today"].as_f64().expect("today");
    let outcome = wb
        .apply_auto_filter(sheet, today, &locale)
        .expect("applies");
    assert_eq!(outcome.hidden.len(), 12);
    let bytes = wb.save().expect("saves");
    let wb2 = Workbook::open(&bytes, &Limits::default()).expect("reopens");
    assert_eq!(wb2.sheet(sheet).expect("sheet").auto_filter(), Some(filter));
    let hidden = wb2
        .sheet(sheet)
        .expect("sheet")
        .rows()
        .iter()
        .filter(|r| r.attrs().iter().any(|a| a.name == "hidden"))
        .count();
    assert_eq!(hidden, 12);
}
