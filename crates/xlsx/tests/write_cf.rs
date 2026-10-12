//! Conditional formatting: every rule type is written as Excel writes it and reads back the same.

mod write_support;

use open365_xlsx::{
    CellIsOp, CfKind, CfRuleSpec, CfSpec, Cfvo, CfvoKind, Color, ColorSource, DataBarAxis,
    DataBarDirection, DataBarExt, Dxf, DxfFill, DxfFont, FillPattern, Rgb, TimePeriod, XlsxError,
};
use write_support::{fixture, reopen, text_of};

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        source: ColorSource::Rgb(Rgb::new(r, g, b)),
        tint: 0.0,
    }
}

fn red_fill() -> Dxf {
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
        format: Some(red_fill()),
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

fn cf(rules: Vec<CfRuleSpec>) -> CfSpec {
    CfSpec {
        ranges: vec![(2, 1, 9, 2)],
        rules,
    }
}

#[test]
fn a_cell_value_rule_is_written_with_a_dxf_like_excel() {
    let mut wb = fixture("formulas-basic");
    wb.add_conditional_format(
        0,
        &cf(vec![rule(CfKind::CellIs {
            op: CellIsOp::GreaterThan,
            formulas: vec!["100".to_owned()],
        })]),
    )
    .expect("adds");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<conditionalFormatting sqref=\"A2:B9\"><cfRule type=\"cellIs\" dxfId=\"0\" priority=\"1\" operator=\"greaterThan\"><formula>100</formula></cfRule></conditionalFormatting>"), "{sheet}");
    let styles = text_of(&mut wb, "xl/styles.xml");
    assert!(styles.contains("<dxfs count=\"1\"><dxf><font><color rgb=\"FF9C0006\"/></font><fill><patternFill><bgColor rgb=\"FFFFC7CE\"/></patternFill></fill></dxf></dxfs>"), "{styles}");
    let back = reopen(&mut wb);
    let blocks = back.sheet(0).expect("sheet").conditional_formats();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].ranges, vec![(2, 1, 9, 2)]);
    assert!(
        matches!(&blocks[0].rules[0].kind, CfKind::CellIs { op: CellIsOp::GreaterThan, formulas } if formulas == &["100"])
    );
    assert!(
        back.styles()
            .dxf(0)
            .and_then(|d| d.fill.as_ref())
            .and_then(DxfFill::solid_color)
            .is_some()
    );
}

#[test]
fn text_blank_error_date_and_statistical_rules_round_trip() {
    let mut wb = fixture("formulas-basic");
    let kinds = vec![
        CfKind::Top10 {
            rank: 5,
            percent: true,
            bottom: true,
        },
        CfKind::AboveAverage {
            above: false,
            equal: true,
            std_dev: 2,
        },
        CfKind::DuplicateValues,
        CfKind::UniqueValues,
        CfKind::ContainsText("a\"b".to_owned()),
        CfKind::NotContainsText("x".to_owned()),
        CfKind::BeginsWith("pre".to_owned()),
        CfKind::EndsWith("suf".to_owned()),
        CfKind::ContainsBlanks,
        CfKind::NotContainsBlanks,
        CfKind::ContainsErrors,
        CfKind::NotContainsErrors,
        CfKind::TimePeriod(TimePeriod::LastWeek),
        CfKind::TimePeriod(TimePeriod::Today),
        CfKind::CellIs {
            op: CellIsOp::Between,
            formulas: vec!["1".to_owned(), "9".to_owned()],
        },
    ];
    let mut rules: Vec<CfRuleSpec> = kinds.iter().cloned().map(rule).collect();
    rules.push(CfRuleSpec {
        formula: Some("=MOD(ROW(),2)=0".to_owned()),
        kind: CfKind::Expression,
        ..rule(CfKind::Expression)
    });
    wb.add_conditional_format(0, &cf(rules)).expect("adds");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(
        sheet.contains(
            "type=\"top10\" dxfId=\"0\" priority=\"1\" percent=\"1\" bottom=\"1\" rank=\"5\""
        ),
        "{sheet}"
    );
    assert!(sheet.contains("type=\"aboveAverage\" dxfId=\"0\" priority=\"2\" aboveAverage=\"0\" equalAverage=\"1\" stdDev=\"2\""), "{sheet}");
    assert!(sheet.contains("operator=\"containsText\" text=\"a&quot;b\"><formula>NOT(ISERROR(SEARCH(\"a\"\"b\",A2)))</formula>"), "{sheet}");
    assert!(
        sheet.contains("<formula>LEFT(A2,LEN(\"pre\"))=\"pre\"</formula>"),
        "{sheet}"
    );
    assert!(
        sheet.contains("<formula>LEN(TRIM(A2))=0</formula>"),
        "{sheet}"
    );
    assert!(
        sheet.contains("timePeriod=\"lastWeek\"") && sheet.contains("FLOOR(A2,1)=TODAY()"),
        "{sheet}"
    );
    assert!(
        sheet.contains("<formula>MOD(ROW(),2)=0</formula>"),
        "{sheet}"
    );
    let back = reopen(&mut wb);
    let block = &back.sheet(0).expect("sheet").conditional_formats()[0];
    assert_eq!(block.rules.len(), kinds.len() + 1);
    for (i, k) in kinds.iter().enumerate() {
        let got = &block.rules[i].kind;
        match (k, got) {
            (CfKind::ContainsText(a), CfKind::ContainsText(b))
            | (CfKind::NotContainsText(a), CfKind::NotContainsText(b)) => assert_eq!(a, b),
            _ => assert_eq!(k, got, "rule {i}"),
        }
        assert_eq!(
            block.rules[i].priority,
            i64::try_from(i + 1).expect("small")
        );
    }
    assert_eq!(block.rules[15].kind, CfKind::Expression);
}

#[test]
fn colour_scales_and_icon_sets_have_no_dxf() {
    let mut wb = fixture("formulas-basic");
    wb.add_conditional_format(
        0,
        &cf(vec![
            rule(CfKind::ColorScale {
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
            }),
            rule(CfKind::IconSet {
                name: "3Arrows".to_owned(),
                cfvos: vec![
                    point(CfvoKind::Percent, Some("0")),
                    point(CfvoKind::Percent, Some("33")),
                    point(CfvoKind::Percent, Some("67")),
                ],
                reverse: true,
                show_value: false,
            }),
            rule(CfKind::IconSet {
                name: "3TrafficLights1".to_owned(),
                cfvos: vec![
                    point(CfvoKind::Percent, Some("0")),
                    point(CfvoKind::Num, Some("5")),
                    point(CfvoKind::Formula, Some("$A$1")),
                ],
                reverse: false,
                show_value: true,
            }),
        ]),
    )
    .expect("adds");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<cfRule type=\"colorScale\" priority=\"1\"><colorScale><cfvo type=\"min\"/><cfvo type=\"percentile\" val=\"50\"/><cfvo type=\"max\"/><color rgb=\"FFF8696B\"/><color rgb=\"FFFFEB84\"/><color rgb=\"FF63BE7B\"/></colorScale></cfRule>"), "{sheet}");
    assert!(
        sheet.contains("<iconSet iconSet=\"3Arrows\" showValue=\"0\" reverse=\"1\">"),
        "{sheet}"
    );
    assert!(
        sheet.contains("<iconSet><cfvo type=\"percent\" val=\"0\"/>"),
        "{sheet}"
    );
    assert!(
        !text_of(&mut wb, "xl/styles.xml").contains("<dxf>"),
        "no dxf for scales and icons"
    );
    let back = reopen(&mut wb);
    let rules = &back.sheet(0).expect("sheet").conditional_formats()[0].rules;
    assert!(
        matches!(&rules[0].kind, CfKind::ColorScale { cfvos, colors } if cfvos.len() == 3 && colors.len() == 3)
    );
    assert!(
        matches!(&rules[1].kind, CfKind::IconSet { name, reverse: true, show_value: false, .. } if name == "3Arrows")
    );
    assert!(matches!(&rules[2].kind, CfKind::IconSet { name, .. } if name == "3TrafficLights1"));
    let bad = wb.add_conditional_format(
        0,
        &cf(vec![rule(CfKind::IconSet {
            name: "3Stars".to_owned(),
            cfvos: vec![],
            reverse: false,
            show_value: true,
        })]),
    );
    assert!(matches!(bad, Err(XlsxError::UnsupportedEdit(_))));
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

#[test]
fn a_data_bar_gets_its_excel_2010_extension() {
    let mut wb = fixture("formulas-basic");
    let ext = DataBarExt {
        min_length: 0,
        max_length: 100,
        gradient: false,
        border: true,
        border_color: None,
        negative_same_as_positive: false,
        negative_fill: Some(rgb(255, 0, 0)),
        negative_border_same_as_positive: true,
        negative_border_color: None,
        axis: DataBarAxis::Midpoint,
        axis_color: None,
        direction: DataBarDirection::RightToLeft,
        min: Some(point(CfvoKind::AutoMin, None)),
        max: Some(point(CfvoKind::Num, Some("50"))),
    };
    wb.add_conditional_format(0, &cf(vec![rule(data_bar(Some(ext)))]))
        .expect("adds");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<dataBar><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FF638EC6\"/></dataBar><extLst><ext uri=\"{B025F937-C7B1-47D3-B67F-A62EFF666E3E}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><x14:id>{"), "{sheet}");
    assert!(
        sheet.contains("<ext uri=\"{78C0D931-6437-407d-A8EE-F0AAD7539E65}\""),
        "{sheet}"
    );
    assert!(sheet.contains("<x14:dataBar minLength=\"0\" maxLength=\"100\" gradient=\"0\" border=\"1\" axisPosition=\"middle\" direction=\"rightToLeft\"><x14:cfvo type=\"autoMin\"/><x14:cfvo type=\"num\"><xm:f>50</xm:f></x14:cfvo>"), "{sheet}");
    assert!(sheet.contains("<xm:sqref>A2:B9</xm:sqref>"), "{sheet}");
    let back = reopen(&mut wb);
    let rule = &back.sheet(0).expect("sheet").conditional_formats()[0].rules[0];
    let CfKind::DataBar { ext: Some(e), .. } = &rule.kind else {
        panic!("data bar with extension")
    };
    assert!(!e.gradient && e.border);
    assert_eq!(e.axis, DataBarAxis::Midpoint);
    assert_eq!(e.direction, DataBarDirection::RightToLeft);
    assert_eq!(e.max.as_ref().and_then(|m| m.val.as_deref()), Some("50"));
}

#[test]
fn new_rules_take_priority_one_and_push_the_old_ones_down() {
    let mut wb = fixture("formulas-basic");
    wb.add_conditional_format(0, &cf(vec![rule(CfKind::DuplicateValues)]))
        .expect("first");
    wb.add_conditional_format(
        0,
        &CfSpec {
            ranges: vec![(1, 4, 3, 4)],
            rules: vec![rule(CfKind::UniqueValues), rule(CfKind::ContainsBlanks)],
        },
    )
    .expect("second");
    let blocks = wb.sheet(0).expect("sheet").conditional_formats();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].rules[0].priority, 3);
    assert_eq!(
        blocks[1]
            .rules
            .iter()
            .map(|r| r.priority)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    // The dxf is shared by equal formats.
    assert!(text_of(&mut wb, "xl/styles.xml").contains("<dxfs count=\"1\">"));
}

#[test]
fn editing_deleting_and_clearing_keep_the_sheet_consistent() {
    let mut wb = fixture("formulas-basic");
    wb.add_conditional_format(0, &cf(vec![rule(data_bar(None))]))
        .expect("bar");
    wb.add_conditional_format(
        0,
        &CfSpec {
            ranges: vec![(1, 4, 3, 4)],
            rules: vec![rule(CfKind::UniqueValues)],
        },
    )
    .expect("unique");
    // Replace block 0 (the data bar) with a duplicate rule: its extension goes.
    wb.replace_conditional_format(0, 0, &cf(vec![rule(CfKind::DuplicateValues)]))
        .expect("replace");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(
        !sheet.contains("x14:dataBar") && !sheet.contains("<extLst>"),
        "{sheet}"
    );
    // Clear the middle of the first block's range: it splits.
    wb.add_conditional_format(0, &cf(vec![rule(data_bar(None))]))
        .expect("bar again");
    let removed = wb
        .clear_conditional_formats_in(0, (4, 1, 5, 2))
        .expect("clear");
    assert!(removed >= 1);
    let blocks = wb.sheet(0).expect("sheet").conditional_formats();
    let bar = blocks
        .iter()
        .find(|b| {
            b.rules
                .iter()
                .any(|r| matches!(r.kind, CfKind::DataBar { .. }))
        })
        .expect("bar block");
    assert_eq!(bar.ranges.len(), 2, "{:?}", bar.ranges);
    // Deleting the data bar block removes its extension.
    let at = blocks
        .iter()
        .position(|b| std::ptr::eq(b, bar))
        .expect("index");
    wb.delete_conditional_format(0, at).expect("delete");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("x14:dataBar"), "{sheet}");
    assert!(wb.delete_conditional_format(0, 99).is_err());
    assert!(
        wb.add_conditional_format(
            0,
            &CfSpec {
                ranges: vec![],
                rules: vec![]
            }
        )
        .is_err()
    );
    reopen(&mut wb);
}

/// Reference: Excel writes no `priority` on a data bar's `x14:cfRule` (fixtures/xlsx/grid-databar.xlsx, and
/// `databar-default.xlsx` from tools/office/make-xlsx-write-excel.ps1); the rule in the sheet carries it.
#[test]
fn a_data_bar_x14_rule_has_no_priority_like_excel() {
    let mut wb = fixture("formulas-basic");
    let bar = CfKind::DataBar {
        min: point(CfvoKind::Min, None),
        max: point(CfvoKind::Max, None),
        color: rgb(0x63, 0x8E, 0xC6),
        min_length: 10,
        max_length: 90,
        show_value: true,
        ext: None,
    };
    wb.add_conditional_format(0, &cf(vec![rule(bar.clone()), rule(bar)]))
        .expect("adds");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    let heads: Vec<&str> = sheet
        .split("<x14:cfRule ")
        .skip(1)
        .filter_map(|r| r.split('>').next())
        .collect();
    assert_eq!(heads.len(), 2, "{sheet}");
    assert!(heads.iter().all(|h| !h.contains("priority")), "{heads:?}");
}
