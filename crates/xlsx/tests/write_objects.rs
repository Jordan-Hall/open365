//! Hyperlinks, notes, charts and pivot tables.

mod write_support;

use open365_xlsx::{
    CellInput, ChartAnchor, HyperlinkSpec, HyperlinkTarget, NoteSpec, PivotSpec, XlsxError,
};
use write_support::{fixture, has_part, reopen, text_of};

fn part_of(name: &str, part: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx")
        .join(format!("{name}.xlsx"));
    let pkg = open365_opc::Package::read(
        &std::fs::read(path).expect("fixture"),
        &open365_opc::Limits::default(),
    )
    .expect("package");
    pkg.part(part).expect("part").data().to_vec()
}

fn link(range: (u32, u32, u32, u32), target: HyperlinkTarget) -> HyperlinkSpec {
    HyperlinkSpec {
        range,
        target,
        display: None,
        tooltip: None,
    }
}

#[test]
fn the_three_kinds_of_hyperlink_are_written_the_way_excel_does() {
    let mut wb = fixture("formulas-basic");
    wb.set_hyperlink(
        0,
        &link(
            (1, 1, 1, 1),
            HyperlinkTarget::Url("https://example.com/a?x=1&y=2".to_owned()),
        ),
    )
    .expect("url");
    wb.set_hyperlink(
        0,
        &link(
            (2, 1, 2, 1),
            HyperlinkTarget::Location("Sheet1!B2".to_owned()),
        ),
    )
    .expect("place");
    wb.set_hyperlink(
        0,
        &HyperlinkSpec {
            range: (3, 1, 3, 1),
            target: HyperlinkTarget::Email {
                address: "a@b.com".to_owned(),
                subject: Some("Hello there".to_owned()),
            },
            display: Some("mail".to_owned()),
            tooltip: Some("tip".to_owned()),
        },
    )
    .expect("email");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    // A1 and A3 already hold values: Excel then writes the address as `display` for an address link.
    assert!(
        sheet.contains(
            "<hyperlink ref=\"A1\" r:id=\"rId1\" display=\"https://example.com/a?x=1&amp;y=2\"/>"
        ),
        "{sheet}"
    );
    assert!(
        sheet.contains("<hyperlink ref=\"A2\" location=\"Sheet1!B2\" display=\"Sheet1!B2\"/>"),
        "{sheet}"
    );
    assert!(
        sheet.contains("<hyperlink ref=\"A3\" r:id=\"rId2\" tooltip=\"tip\"/>"),
        "{sheet}"
    );
    let rels = text_of(&mut wb, "xl/worksheets/_rels/sheet1.xml.rels");
    assert!(
        rels.contains("Target=\"https://example.com/a?x=1&amp;y=2\" TargetMode=\"External\""),
        "{rels}"
    );
    assert!(
        rels.contains("Target=\"mailto:a@b.com?subject=Hello%20there\" TargetMode=\"External\""),
        "{rels}"
    );
    let back = reopen(&mut wb);
    let links = back.hyperlinks(0);
    assert_eq!(links.len(), 3);
    assert_eq!(
        links[0].target,
        HyperlinkTarget::Url("https://example.com/a?x=1&y=2".to_owned())
    );
    assert_eq!(
        links[2].target,
        HyperlinkTarget::Email {
            address: "a@b.com".to_owned(),
            subject: Some("Hello there".to_owned())
        }
    );
}

/// Reference: `tools/office/make-xlsx-write-excel.ps1` (`hyperlinks.xlsx`, `hyperlinks-text.xlsx` in
/// fixtures/reference/xlsx/write-excel), Excel's own output for Hyperlinks.Add.
#[test]
fn a_link_is_written_and_styled_like_excel_does() {
    let mut wb = fixture("formulas-basic");
    let at = |r: u32| (r, 6, r, 6);
    let spec = |r, target, text: Option<&str>, tip: Option<&str>| HyperlinkSpec {
        range: at(r),
        target,
        display: text.map(str::to_owned),
        tooltip: tip.map(str::to_owned),
    };
    let url = |u: &str| HyperlinkTarget::Url(u.to_owned());
    let place = || HyperlinkTarget::Location("Sheet1!B2".to_owned());
    wb.set_hyperlink(0, &spec(1, url("https://example.com/a"), None, None))
        .expect("empty cell, address");
    wb.set_hyperlink(0, &spec(2, place(), Some("go"), None))
        .expect("place with text");
    wb.set_hyperlink(
        0,
        &spec(
            3,
            url("https://example.com/b"),
            Some("text b"),
            Some("tip b"),
        ),
    )
    .expect("address with text");
    wb.set_hyperlink(0, &spec(4, place(), None, Some("loc tip")))
        .expect("place");
    wb.set_cell_value(0, 5, 6, CellInput::Text("Apple".to_owned()))
        .expect("text");
    wb.set_hyperlink(0, &spec(5, url("https://example.com/c"), None, None))
        .expect("full cell, address");
    wb.set_cell_value(0, 6, 6, CellInput::Text("Plum".to_owned()))
        .expect("text");
    wb.set_hyperlink(0, &spec(6, place(), None, None))
        .expect("full cell, place");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    for expected in [
        "<hyperlink ref=\"F1\" r:id=\"rId1\"/>",
        "<hyperlink ref=\"F2\" location=\"Sheet1!B2\" display=\"go\"/>",
        "<hyperlink ref=\"F3\" r:id=\"rId2\" tooltip=\"tip b\"/>",
        "<hyperlink ref=\"F4\" location=\"Sheet1!B2\" tooltip=\"loc tip\" display=\"Sheet1!B2\"/>",
        "<hyperlink ref=\"F5\" r:id=\"rId3\" display=\"https://example.com/c\"/>",
        "<hyperlink ref=\"F6\" location=\"Sheet1!B2\" display=\"Sheet1!B2\"/>",
    ] {
        assert!(
            sheet.contains(expected),
            "{expected}
{sheet}"
        );
    }
    let back = reopen(&mut wb);
    let text = |r: u32| {
        back.sheet(0)
            .and_then(|s| s.cell(r, 6))
            .map(|c| format!("{:?}", c.value))
            .unwrap_or_default()
    };
    assert!(
        text(1).contains("https://example.com/a")
            && text(2).contains("go")
            && text(3).contains("text b")
    );
    assert!(text(4).contains("Sheet1!B2") && text(5).contains("Apple") && text(6).contains("Plum"));
    // Every linked cell has the Hyperlink cell style (xfId 1 is the style's own record).
    let styles = text_of(&mut wb, "xl/styles.xml");
    assert!(
        styles.contains("<font><u/><sz val=\"11\"/><color theme=\"10\"/>"),
        "{styles}"
    );
    assert!(
        styles.contains("<cellStyle name=\"Hyperlink\" xfId=\"1\" builtinId=\"8\"/>"),
        "{styles}"
    );
    for r in 1..=6 {
        assert_ne!(back.cell_xf(0, r, 6), 0, "F{r} is styled");
    }
    let xf = back.cell_xf(0, 1, 6);
    assert!(
        (1..=6).all(|r| back.cell_xf(0, r, 6) == xf),
        "one shared record"
    );
}

#[test]
fn a_linked_cell_with_a_format_of_its_own_keeps_it() {
    let mut wb = fixture("formulas-basic");
    // F1 has a cell style already; the link must not replace it with the plain Hyperlink record.
    wb.apply_cell_style(0, (1, 6, 1, 6), "Good")
        .expect("styles");
    let before = wb.cell_xf(0, 1, 6);
    assert_ne!(before, 0);
    wb.set_hyperlink(
        0,
        &link(
            (1, 6, 1, 6),
            HyperlinkTarget::Url("https://example.com/".to_owned()),
        ),
    )
    .expect("links");
    assert_eq!(reopen(&mut wb).cell_xf(0, 1, 6), before);
}

#[test]
fn replacing_and_removing_a_link_cleans_its_relationship() {
    let mut wb = fixture("formulas-basic");
    wb.set_hyperlink(
        0,
        &link(
            (1, 1, 1, 1),
            HyperlinkTarget::Url("https://one.example/".to_owned()),
        ),
    )
    .expect("one");
    wb.set_hyperlink(
        0,
        &link(
            (1, 1, 1, 1),
            HyperlinkTarget::Url("https://two.example/".to_owned()),
        ),
    )
    .expect("two");
    let rels = text_of(&mut wb, "xl/worksheets/_rels/sheet1.xml.rels");
    assert!(
        !rels.contains("one.example") && rels.contains("two.example"),
        "{rels}"
    );
    assert!(wb.remove_hyperlink(0, 1, 1).expect("remove"));
    assert!(!wb.remove_hyperlink(0, 1, 1).expect("again"));
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("<hyperlinks"), "{sheet}");
    reopen(&mut wb);
}

#[test]
fn schemes_that_run_code_and_bad_addresses_are_refused() {
    let mut wb = fixture("formulas-basic");
    for bad in ["javascript:alert(1)", " VBScript:x", "data:text/html,x", ""] {
        let r = wb.set_hyperlink(0, &link((1, 1, 1, 1), HyperlinkTarget::Url(bad.to_owned())));
        assert!(matches!(r, Err(XlsxError::UnsupportedEdit(_))), "{bad:?}");
    }
    assert!(
        wb.set_hyperlink(
            0,
            &link(
                (1, 1, 1, 1),
                HyperlinkTarget::Email {
                    address: "nope".to_owned(),
                    subject: None
                }
            )
        )
        .is_err()
    );
    assert!(
        wb.set_hyperlink(
            0,
            &link((0, 1, 1, 1), HyperlinkTarget::Location("A1".to_owned()))
        )
        .is_err()
    );
}

fn note(row: u32, col: u32, text: &str) -> NoteSpec {
    NoteSpec {
        row,
        col,
        author: "Jordan".to_owned(),
        text: text.to_owned(),
        author_prefix: true,
    }
}

#[test]
fn the_first_note_creates_comments_vml_and_the_legacy_drawing() {
    let mut wb = fixture("formulas-basic");
    wb.set_note(0, &note(2, 3, "Check this")).expect("note");
    let comments = text_of(&mut wb, "xl/comments1.xml");
    assert!(
        comments.contains("<authors><author>Jordan</author></authors>"),
        "{comments}"
    );
    assert!(
        comments.contains("<comment ref=\"C2\" authorId=\"0\" shapeId=\"0\"><text>"),
        "{comments}"
    );
    assert!(
        comments.contains("<t>Jordan:</t>") && comments.contains("Check this"),
        "{comments}"
    );
    let vml = text_of(&mut wb, "xl/drawings/vmlDrawing1.vml");
    assert!(
        vml.contains("<o:idmap v:ext=\"edit\" data=\"1\"/>") && vml.contains("_x0000_s1025"),
        "{vml}"
    );
    assert!(
        vml.contains("<x:Row>1</x:Row><x:Column>2</x:Column>"),
        "{vml}"
    );
    assert!(vml.contains("ObjectType=\"Note\""), "{vml}");
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<legacyDrawing r:id=\"rId1\"/>"), "{sheet}");
    let ct = text_of(&mut wb, "[Content_Types].xml");
    assert!(
        ct.contains("Extension=\"vml\"") && ct.contains("PartName=\"/xl/comments1.xml\""),
        "{ct}"
    );
    let back = reopen(&mut wb);
    let notes = back.notes(0);
    assert_eq!(notes.len(), 1);
    assert_eq!(
        (
            notes[0].row,
            notes[0].col,
            notes[0].text.as_str(),
            notes[0].author_prefix
        ),
        (2, 3, "Check this", true)
    );
}

#[test]
fn notes_are_sorted_replaced_and_deleted_with_their_shapes() {
    let mut wb = fixture("formulas-basic");
    wb.set_note(0, &note(5, 1, "later")).expect("a");
    wb.set_note(0, &note(1, 1, "first")).expect("b");
    wb.set_note(
        0,
        &NoteSpec {
            author: "Sam".to_owned(),
            author_prefix: false,
            ..note(5, 1, "edited")
        },
    )
    .expect("c");
    let comments = text_of(&mut wb, "xl/comments1.xml");
    assert!(
        comments.find("ref=\"A1\"").expect("A1") < comments.find("ref=\"A5\"").expect("A5"),
        "{comments}"
    );
    assert!(
        comments.contains("<author>Jordan</author><author>Sam</author>"),
        "{comments}"
    );
    let vml = text_of(&mut wb, "xl/drawings/vmlDrawing1.vml");
    assert_eq!(vml.matches("<v:shape ").count(), 2, "{vml}");
    assert!(wb.delete_note(0, 1, 1).expect("delete"));
    let vml = text_of(&mut wb, "xl/drawings/vmlDrawing1.vml");
    assert_eq!(vml.matches("<v:shape ").count(), 1);
    assert!(!vml.contains("<x:Row>0</x:Row>"));
    assert!(wb.delete_note(0, 5, 1).expect("last"));
    assert!(
        !has_part(&mut wb, "xl/comments1.xml") && !has_part(&mut wb, "xl/drawings/vmlDrawing1.vml")
    );
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("legacyDrawing"), "{sheet}");
    assert!(!wb.delete_note(0, 5, 1).expect("none left"));
    reopen(&mut wb);
}

#[test]
fn a_note_is_added_beside_the_ones_excel_wrote() {
    let mut wb = fixture("comments-notes");
    assert_eq!(wb.notes(0).len(), 1);
    wb.set_note(0, &note(3, 2, "second")).expect("adds");
    let vml = text_of(&mut wb, "xl/drawings/vmlDrawing1.vml");
    assert!(
        vml.contains("_x0000_s1025") && vml.contains("_x0000_s1026"),
        "{vml}"
    );
    let back = reopen(&mut wb);
    assert_eq!(back.notes(0).len(), 2);
}

#[test]
fn a_chart_gets_a_drawing_an_anchor_and_a_relationship() {
    let chart = part_of("chart", "xl/charts/chart1.xml");
    let mut wb = fixture("formulas-basic");
    let made = wb
        .insert_chart(
            0,
            &chart,
            &ChartAnchor {
                range: (2, 4, 16, 10),
                name: None,
            },
        )
        .expect("inserts");
    assert_eq!(
        (
            made.chart_part.as_str(),
            made.drawing_part.as_str(),
            made.shape_id,
            made.name.as_str()
        ),
        (
            "xl/charts/chart1.xml",
            "xl/drawings/drawing1.xml",
            2,
            "Chart 1"
        )
    );
    let drawing = text_of(&mut wb, "xl/drawings/drawing1.xml");
    assert!(drawing.contains("<xdr:from><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>10</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>16</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>"), "{drawing}");
    assert!(
        drawing.contains("<xdr:cNvPr id=\"2\" name=\"Chart 1\"/>"),
        "{drawing}"
    );
    let rels = text_of(&mut wb, "xl/drawings/_rels/drawing1.xml.rels");
    assert!(
        rels.contains("Id=\"rId1\"") && rels.contains("Target=\"../charts/chart1.xml\""),
        "{rels}"
    );
    let sheet = text_of(&mut wb, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("r:id=\"rId"), "{sheet}");
    let ct = text_of(&mut wb, "[Content_Types].xml");
    assert!(ct.contains("PartName=\"/xl/charts/chart1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawingml.chart+xml\""), "{ct}");
    assert!(ct.contains("PartName=\"/xl/drawings/drawing1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawing+xml\""), "{ct}");
    let back = reopen(&mut wb);
    assert_eq!(back.sheet_charts(0).len(), 1);
}

#[test]
fn a_second_chart_shares_the_drawing() {
    let chart = part_of("chart", "xl/charts/chart1.xml");
    let mut wb = fixture("chart");
    let before = wb.sheet_charts(0).len();
    let made = wb
        .insert_chart(
            0,
            &chart,
            &ChartAnchor {
                range: (20, 1, 30, 5),
                name: Some("Sales".to_owned()),
            },
        )
        .expect("inserts");
    assert!(!made.drawing_created);
    assert_eq!(made.shape_id, 3);
    assert_eq!(made.chart_part, "xl/charts/chart2.xml");
    let back = reopen(&mut wb);
    assert_eq!(back.sheet_charts(0).len(), before + 1);
    assert!(
        wb.insert_chart(
            0,
            b"<notachart/>",
            &ChartAnchor {
                range: (1, 1, 2, 2),
                name: None
            }
        )
        .is_err()
    );
}

fn pivot_spec() -> PivotSpec {
    PivotSpec {
        cache_definition: part_of("pivot-values", "xl/pivotCache/pivotCacheDefinition1.xml"),
        cache_records: Some(part_of(
            "pivot-values",
            "xl/pivotCache/pivotCacheRecords1.xml",
        )),
        table_definition: part_of("pivot-values", "xl/pivotTables/pivotTable1.xml"),
    }
}

#[test]
fn a_pivot_table_is_wired_to_its_cache_and_the_workbook() {
    let mut wb = fixture("formulas-basic");
    let made = wb.add_pivot_table(0, &pivot_spec()).expect("adds");
    assert_eq!(made.cache_id, 1);
    assert_eq!(made.pivot_table_part, "xl/pivotTables/pivotTable1.xml");
    let book = text_of(&mut wb, "xl/workbook.xml");
    assert!(
        book.contains("<pivotCaches><pivotCache cacheId=\"1\" r:id=\"rId"),
        "{book}"
    );
    let table = text_of(&mut wb, "xl/pivotTables/pivotTable1.xml");
    assert!(table.contains("cacheId=\"1\""), "{table}");
    let t_rels = text_of(&mut wb, "xl/pivotTables/_rels/pivotTable1.xml.rels");
    assert!(
        t_rels.contains("Target=\"../pivotCache/pivotCacheDefinition1.xml\""),
        "{t_rels}"
    );
    let c_rels = text_of(
        &mut wb,
        "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels",
    );
    assert!(
        c_rels.contains("Id=\"rId1\"") && c_rels.contains("Target=\"pivotCacheRecords1.xml\""),
        "{c_rels}"
    );
    let s_rels = text_of(&mut wb, "xl/worksheets/_rels/sheet1.xml.rels");
    assert!(
        s_rels.contains("relationships/pivotTable\" Target=\"../pivotTables/pivotTable1.xml\""),
        "{s_rels}"
    );
    let ct = text_of(&mut wb, "[Content_Types].xml");
    for part in [
        "pivotTables/pivotTable1",
        "pivotCache/pivotCacheDefinition1",
        "pivotCache/pivotCacheRecords1",
    ] {
        assert!(
            ct.contains(&format!("PartName=\"/xl/{part}.xml\"")),
            "{part}: {ct}"
        );
    }
    let second = wb.add_pivot_table(0, &pivot_spec()).expect("second");
    assert_eq!(
        (second.cache_id, second.pivot_table_part.as_str()),
        (2, "xl/pivotTables/pivotTable2.xml")
    );
    reopen(&mut wb);
    assert!(
        wb.add_pivot_table(
            0,
            &PivotSpec {
                table_definition: b"<x/>".to_vec(),
                ..pivot_spec()
            }
        )
        .is_err()
    );
}
