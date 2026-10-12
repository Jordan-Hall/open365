//! Writes one small workbook per write operation, for the Excel oracle (`tools/office/office-oracle.ps1
//! -Mode verify`): every file must open in Excel with no repair prompt.
//!
//! `cargo run -p open365-xlsx --example write_ops [out_dir]` (default `fixtures/xlsx/write-ops`).
//! The base workbooks are the repository's own clean fixtures; nothing personal goes in.

use std::path::PathBuf;

use open365_opc::{Limits, Package, TargetMode};
use open365_xlsx::{
    CalcMode, CalcSettings, CellInput, CellIsOp, CfKind, CfRuleSpec, CfSpec, Cfvo, CfvoKind,
    ChartAnchor, Color, ColorSource, DataBarAxis, DataBarDirection, DataBarExt, Dxf, DxfFill,
    DxfFont, FillPattern, HeaderChoice, HyperlinkSpec, HyperlinkTarget, NameSource, NameSpec,
    NoteSpec, PivotSpec, Rgb, SheetState, ShiftDirection, TabColor, TableSpec, TimePeriod,
    Workbook, XlsxError,
};

type Op = (&'static str, fn() -> Result<Workbook, XlsxError>);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/xlsx")
}

fn load(name: &str) -> Workbook {
    let bytes = std::fs::read(fixtures().join(format!("{name}.xlsx"))).expect("fixture");
    Workbook::open(&bytes, &Limits::default()).expect("opens")
}

fn part_of(name: &str, part: &str) -> Vec<u8> {
    let bytes = std::fs::read(fixtures().join(format!("{name}.xlsx"))).expect("fixture");
    Package::read(&bytes, &Limits::default())
        .expect("package")
        .part(part)
        .expect("part")
        .data()
        .to_vec()
}

fn text(wb: &mut Workbook, r: u32, c: u32, t: &str) {
    wb.set_cell_value(0, r, c, CellInput::Text(t.to_owned()))
        .expect("text");
}

fn num(wb: &mut Workbook, r: u32, c: u32, n: f64) {
    wb.set_cell_value(0, r, c, CellInput::Number(n))
        .expect("number");
}

/// A small block at F1: optional header row Item/Qty/Price then three data rows (clean demo data).
fn block(wb: &mut Workbook, header: bool) {
    let top = u32::from(header);
    if header {
        for (i, h) in ["Item", "Qty", "Price"].iter().enumerate() {
            text(wb, 1, 6 + u32::try_from(i).expect("small"), h);
        }
    }
    for (i, (n, q, p)) in [
        ("Apple", 3.0, 1.5),
        ("Pear", 5.0, 2.25),
        ("Plum", 8.0, 0.75),
    ]
    .iter()
    .enumerate()
    {
        let r = 1 + top + u32::try_from(i).expect("small");
        text(wb, r, 6, n);
        num(wb, r, 7, *q);
        num(wb, r, 8, *p);
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        source: ColorSource::Rgb(Rgb::new(r, g, b)),
        tint: 0.0,
    }
}

fn dxf() -> Dxf {
    Dxf {
        font: DxfFont {
            color: Some(rgb(0x9C, 0, 0x06)),
            ..DxfFont::default()
        },
        fill: Some(DxfFill {
            pattern: FillPattern::Solid,
            fg: None,
            bg: Some(rgb(0xFF, 0xC7, 0xCE)),
        }),
        ..Dxf::default()
    }
}

fn rule(kind: CfKind) -> CfRuleSpec {
    CfRuleSpec {
        kind,
        format: Some(dxf()),
        stop_if_true: false,
        formula: None,
    }
}

fn point(kind: CfvoKind, val: Option<&str>) -> Cfvo {
    Cfvo {
        kind,
        val: val.map(str::to_owned),
        gte: true,
    }
}

fn cf_wb(kind: CfKind) -> Result<Workbook, XlsxError> {
    let mut wb = load("formulas-basic");
    block(&mut wb, true);
    wb.add_conditional_format(
        0,
        &CfSpec {
            ranges: vec![(2, 6, 4, 8)],
            rules: vec![rule(kind)],
        },
    )?;
    Ok(wb)
}

fn data_bar(ext: Option<DataBarExt>) -> CfKind {
    CfKind::DataBar {
        min: point(CfvoKind::Min, None),
        max: point(CfvoKind::Max, None),
        color: rgb(0x63, 0x8E, 0xC6),
        min_length: 10,
        max_length: 90,
        show_value: true,
        ext,
    }
}

fn icons(name: &str, n: usize) -> CfKind {
    let steps = ["0", "20", "40", "60", "80"];
    CfKind::IconSet {
        name: name.to_owned(),
        cfvos: (0..n)
            .map(|i| point(CfvoKind::Percent, Some(steps[i])))
            .collect(),
        reverse: false,
        show_value: true,
    }
}

fn hidden(state: SheetState) -> Result<Workbook, XlsxError> {
    let mut wb = load("cross-sheet");
    wb.set_sheet_state(1, state)?;
    Ok(wb)
}

fn link(r: u32, target: HyperlinkTarget, display: Option<&str>) -> HyperlinkSpec {
    HyperlinkSpec {
        range: (r, 6, r, 6),
        target,
        display: display.map(str::to_owned),
        tooltip: None,
    }
}

fn name(name: &str, formula: &str) -> NameSpec {
    NameSpec {
        name: name.into(),
        formula: formula.into(),
        ..NameSpec::default()
    }
}

fn ops() -> Vec<Op> {
    vec![
        ("add-part-relationship", || {
            let mut wb = load("formulas-basic");
            wb.add_part(
                "xl/custom/data.xml",
                "application/vnd.example.data+xml",
                b"<d/>".to_vec(),
            )?;
            wb.add_relationship(
                None,
                "http://example.com/rel/data",
                "xl/custom/data.xml",
                TargetMode::Internal,
            )?;
            Ok(wb)
        }),
        ("table-with-header", || {
            let mut wb = load("formulas-basic");
            block(&mut wb, true);
            wb.create_table(
                0,
                &TableSpec {
                    range: (1, 6, 4, 8),
                    ..TableSpec::default()
                },
            )?;
            Ok(wb)
        }),
        ("table-without-header", || {
            let mut wb = load("formulas-basic");
            block(&mut wb, false);
            wb.create_table(
                0,
                &TableSpec {
                    range: (1, 6, 3, 8),
                    headers: HeaderChoice::No,
                    ..TableSpec::default()
                },
            )?;
            Ok(wb)
        }),
        ("name-define", || {
            let mut wb = load("formulas-basic");
            wb.define_name(&name("Total", "Sheet1!$A$1"))?;
            wb.define_name(&NameSpec {
                scope: Some(0),
                comment: Some("local".into()),
                ..name("Local", "Sheet1!$A$1:$A$3")
            })?;
            Ok(wb)
        }),
        ("name-edit", || {
            let mut wb = load("formulas-basic");
            wb.define_name(&name("Total", "Sheet1!$A$1"))?;
            wb.edit_defined_name("Total", None, &name("Grand", "Sheet1!$A$1:$A$2"))?;
            Ok(wb)
        }),
        ("name-delete", || {
            let mut wb = load("defined-names");
            let first = wb
                .defined_names()
                .iter()
                .find(|n| n.local_sheet_id.is_none())
                .map(|n| n.name.clone());
            wb.delete_defined_name(&first.expect("a workbook name"), None)?;
            Ok(wb)
        }),
        ("name-from-selection", || {
            let mut wb = load("formulas-basic");
            block(&mut wb, true);
            let top = NameSource {
                top: true,
                ..NameSource::default()
            };
            wb.create_names_from_selection(0, (1, 6, 4, 8), top, false)?;
            Ok(wb)
        }),
        ("hyperlinks", || {
            let mut wb = load("formulas-basic");
            wb.set_hyperlink(
                0,
                &link(
                    1,
                    HyperlinkTarget::Url("https://example.com/a?x=1&y=2".into()),
                    None,
                ),
            )?;
            wb.set_hyperlink(
                0,
                &link(2, HyperlinkTarget::Location("Sheet1!B2".into()), None),
            )?;
            let mail = HyperlinkTarget::Email {
                address: "a@example.com".into(),
                subject: Some("Hi there".into()),
            };
            wb.set_hyperlink(0, &link(3, mail, Some("mail")))?;
            Ok(wb)
        }),
        ("notes", || {
            let mut wb = load("formulas-basic");
            for (r, t) in [(2, "First"), (5, "Second")] {
                wb.set_note(
                    0,
                    &NoteSpec {
                        row: r,
                        col: 3,
                        author: "Author".into(),
                        text: t.into(),
                        author_prefix: true,
                    },
                )?;
            }
            Ok(wb)
        }),
        ("chart", || {
            let mut wb = load("formulas-basic");
            wb.insert_chart(
                0,
                &part_of("chart", "xl/charts/chart1.xml"),
                &ChartAnchor {
                    range: (2, 12, 16, 18),
                    name: None,
                },
            )?;
            Ok(wb)
        }),
        ("pivot", || {
            let mut wb = load("pivot-values");
            let last = wb.sheets().len() - 1;
            wb.add_pivot_table(
                last,
                &PivotSpec {
                    cache_definition: part_of(
                        "pivot-values",
                        "xl/pivotCache/pivotCacheDefinition1.xml",
                    ),
                    cache_records: Some(part_of(
                        "pivot-values",
                        "xl/pivotCache/pivotCacheRecords1.xml",
                    )),
                    table_definition: part_of("pivot-values", "xl/pivotTables/pivotTable1.xml"),
                },
            )?;
            Ok(wb)
        }),
        ("calc-settings", || {
            let mut wb = load("formulas-basic");
            wb.set_calc_settings(&CalcSettings {
                mode: CalcMode::Manual,
                full_calc_on_load: true,
                calc_on_save: false,
                iterate: true,
                iterate_count: 50,
                iterate_delta: 0.01,
                full_precision: false,
            })?;
            Ok(wb)
        }),
        ("cf-cell-is", || {
            cf_wb(CfKind::CellIs {
                op: CellIsOp::Between,
                formulas: vec!["1".into(), "5".into()],
            })
        }),
        ("cf-expression", || {
            let mut wb = load("formulas-basic");
            block(&mut wb, true);
            let r = CfRuleSpec {
                formula: Some("MOD(ROW(),2)=0".into()),
                ..rule(CfKind::Expression)
            };
            wb.add_conditional_format(
                0,
                &CfSpec {
                    ranges: vec![(2, 6, 4, 8)],
                    rules: vec![r],
                },
            )?;
            Ok(wb)
        }),
        ("cf-top10", || {
            cf_wb(CfKind::Top10 {
                rank: 2,
                percent: false,
                bottom: false,
            })
        }),
        ("cf-top10-percent-bottom", || {
            cf_wb(CfKind::Top10 {
                rank: 10,
                percent: true,
                bottom: true,
            })
        }),
        ("cf-above-average", || {
            cf_wb(CfKind::AboveAverage {
                above: true,
                equal: true,
                std_dev: 1,
            })
        }),
        ("cf-duplicates", || cf_wb(CfKind::DuplicateValues)),
        ("cf-unique", || cf_wb(CfKind::UniqueValues)),
        ("cf-contains-text", || {
            cf_wb(CfKind::ContainsText("pl".into()))
        }),
        ("cf-not-contains-text", || {
            cf_wb(CfKind::NotContainsText("pl".into()))
        }),
        ("cf-begins-with", || cf_wb(CfKind::BeginsWith("A".into()))),
        ("cf-ends-with", || cf_wb(CfKind::EndsWith("m".into()))),
        ("cf-blanks", || cf_wb(CfKind::ContainsBlanks)),
        ("cf-not-blanks", || cf_wb(CfKind::NotContainsBlanks)),
        ("cf-errors", || cf_wb(CfKind::ContainsErrors)),
        ("cf-not-errors", || cf_wb(CfKind::NotContainsErrors)),
        ("cf-time-today", || {
            cf_wb(CfKind::TimePeriod(TimePeriod::Today))
        }),
        ("cf-time-yesterday", || {
            cf_wb(CfKind::TimePeriod(TimePeriod::Yesterday))
        }),
        ("cf-time-tomorrow", || {
            cf_wb(CfKind::TimePeriod(TimePeriod::Tomorrow))
        }),
        ("cf-time-last7", || {
            cf_wb(CfKind::TimePeriod(TimePeriod::Last7Days))
        }),
        ("cf-time-thismonth", || {
            cf_wb(CfKind::TimePeriod(TimePeriod::ThisMonth))
        }),
        ("cf-time-lastmonth", || {
            cf_wb(CfKind::TimePeriod(TimePeriod::LastMonth))
        }),
        ("cf-time-nextmonth", || {
            cf_wb(CfKind::TimePeriod(TimePeriod::NextMonth))
        }),
        ("cf-time-thisweek", || {
            cf_wb(CfKind::TimePeriod(TimePeriod::ThisWeek))
        }),
        ("cf-time-lastweek", || {
            cf_wb(CfKind::TimePeriod(TimePeriod::LastWeek))
        }),
        ("cf-time-nextweek", || {
            cf_wb(CfKind::TimePeriod(TimePeriod::NextWeek))
        }),
        ("cf-color-scale", || {
            cf_wb(CfKind::ColorScale {
                cfvos: vec![
                    point(CfvoKind::Min, None),
                    point(CfvoKind::Percentile, Some("50")),
                    point(CfvoKind::Max, None),
                ],
                colors: vec![
                    rgb(0xF8, 0x69, 0x6B),
                    rgb(0xFF, 0xEB, 0x84),
                    rgb(0x63, 0xBE, 0x7B),
                ],
            })
        }),
        ("cf-data-bar-2007", || cf_wb(data_bar(None))),
        ("cf-data-bar-x14", || {
            cf_wb(data_bar(Some(DataBarExt {
                min_length: 0,
                max_length: 100,
                gradient: false,
                border: true,
                border_color: Some(rgb(0x63, 0x8E, 0xC6)),
                negative_same_as_positive: false,
                negative_fill: Some(rgb(0xFF, 0, 0)),
                negative_border_same_as_positive: false,
                negative_border_color: Some(rgb(0xFF, 0, 0)),
                axis: DataBarAxis::Automatic,
                axis_color: Some(rgb(0, 0, 0)),
                direction: DataBarDirection::Context,
                min: Some(point(CfvoKind::AutoMin, None)),
                max: Some(point(CfvoKind::AutoMax, None)),
            })))
        }),
        ("cf-icons-3arrows", || cf_wb(icons("3Arrows", 3))),
        ("cf-icons-4rating", || cf_wb(icons("4Rating", 4))),
        ("cf-icons-5quarters", || cf_wb(icons("5Quarters", 5))),
        ("sheet-add", || {
            let mut wb = load("cross-sheet");
            wb.add_sheet(1, None)?;
            wb.add_sheet(0, Some("Extra data"))?;
            Ok(wb)
        }),
        ("sheet-delete", || {
            let mut wb = load("cross-sheet");
            wb.delete_sheet(1)?;
            Ok(wb)
        }),
        ("sheet-rename", || {
            let mut wb = load("cross-sheet");
            wb.rename_sheet(0, "Base data")?;
            Ok(wb)
        }),
        ("sheet-move", || {
            let mut wb = load("cross-sheet");
            wb.move_sheet(0, 2)?;
            Ok(wb)
        }),
        ("sheet-copy", || {
            let mut wb = load("cross-sheet");
            wb.copy_sheet(0, 1, None)?;
            Ok(wb)
        }),
        ("sheet-copy-objects", || {
            let mut wb = load("formulas-basic");
            block(&mut wb, true);
            wb.create_table(
                0,
                &TableSpec {
                    range: (1, 6, 4, 8),
                    ..TableSpec::default()
                },
            )?;
            wb.set_note(
                0,
                &NoteSpec {
                    row: 2,
                    col: 3,
                    author: "Author".into(),
                    text: "n".into(),
                    author_prefix: true,
                },
            )?;
            let url = HyperlinkTarget::Url("https://example.com/".into());
            wb.set_hyperlink(
                0,
                &HyperlinkSpec {
                    range: (8, 1, 8, 1),
                    target: url,
                    display: None,
                    tooltip: None,
                },
            )?;
            wb.insert_chart(
                0,
                &part_of("chart", "xl/charts/chart1.xml"),
                &ChartAnchor {
                    range: (6, 12, 20, 18),
                    name: None,
                },
            )?;
            wb.copy_sheet(0, 1, None)?;
            Ok(wb)
        }),
        ("sheet-tab-color", || {
            let mut wb = load("cross-sheet");
            wb.set_tab_color(0, TabColor::Rgb(Rgb::new(0xFF, 0, 0)))?;
            wb.set_tab_color(
                1,
                TabColor::Theme {
                    index: 5,
                    tint: 0.4,
                },
            )?;
            Ok(wb)
        }),
        ("sheet-hide", || hidden(SheetState::Hidden)),
        ("sheet-very-hide", || hidden(SheetState::VeryHidden)),
        ("cells-insert-down", || {
            let mut wb = load("formulas-basic");
            wb.shift_cells(0, (2, 1, 3, 2), ShiftDirection::Down)?;
            Ok(wb)
        }),
        ("cells-insert-right", || {
            let mut wb = load("formulas-basic");
            wb.shift_cells(0, (2, 1, 3, 2), ShiftDirection::Right)?;
            Ok(wb)
        }),
        ("cells-delete-up", || {
            let mut wb = load("formulas-basic");
            wb.shift_cells(0, (2, 1, 3, 2), ShiftDirection::Up)?;
            Ok(wb)
        }),
        ("cells-delete-left", || {
            let mut wb = load("formulas-basic");
            wb.shift_cells(0, (2, 1, 3, 2), ShiftDirection::Left)?;
            Ok(wb)
        }),
    ]
}

fn main() {
    let out: PathBuf = std::env::args_os()
        .nth(1)
        .map_or_else(|| fixtures().join("write-ops"), PathBuf::from);
    std::fs::create_dir_all(&out).expect("out dir");
    let mut failed = 0;
    for (name, build) in ops() {
        match build().and_then(|mut wb| wb.save()) {
            Ok(bytes) => {
                std::fs::write(out.join(format!("{name}.xlsx")), bytes).expect("writes");
                println!("wrote {name}");
            }
            Err(e) => {
                failed += 1;
                eprintln!("FAILED {name}: {e}");
            }
        }
    }
    std::process::exit(i32::from(failed > 0));
}
