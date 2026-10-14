//! Page setup: margins, orientation, paper, scale, fit to pages, print options, breaks, print
//! area and print titles are written to the file and read back.

mod common;
mod write_support;

use open365_xlsx::{PageMargins, PageSetup, Workbook};
use write_support::{open, reopen, text_of};

fn blank() -> Workbook {
    open(&common::build(
        &common::sheet_xml("", "<sheetData/>", ""),
        &[],
    ))
}

fn full() -> PageSetup {
    PageSetup {
        margins: PageMargins {
            left: 0.25,
            right: 0.5,
            top: 1.0,
            bottom: 0.75,
            header: 0.4,
            footer: 0.3,
        },
        landscape: true,
        paper_size: Some(9),
        scale: 80,
        print_gridlines: true,
        print_headings: true,
        center_horizontally: true,
        first_page_number: Some(3),
        row_breaks: vec![11, 21],
        col_breaks: vec![4],
        print_area: vec![(1, 1, 20, 6), (30, 2, 35, 3)],
        title_rows: Some((1, 2)),
        title_cols: Some((1, 1)),
        ..PageSetup::default()
    }
}

#[test]
fn page_setup_round_trips_through_a_save() {
    let mut wb = blank();
    assert_eq!(wb.page_setup(0), PageSetup::default());
    wb.set_page_setup(0, &full()).expect("sets");
    assert_eq!(wb.page_setup(0), full());
    let mut again = reopen(&mut wb);
    assert_eq!(again.page_setup(0), full());
    let xml = text_of(&mut again, "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains("<brk id=\"10\" max=\"16383\" man=\"1\""),
        "{xml}"
    );
    assert!(
        xml.contains("<brk id=\"3\" max=\"1048575\" man=\"1\""),
        "{xml}"
    );
    assert!(xml.contains("orientation=\"landscape\""));
    assert!(xml.contains("paperSize=\"9\""));
    let wbxml = text_of(&mut again, "xl/workbook.xml");
    assert!(wbxml.contains("_xlnm.Print_Area"), "{wbxml}");
    assert!(wbxml.contains("$A$1:$F$20,Sheet1!$B$30:$C$35"), "{wbxml}");
    assert!(wbxml.contains("Sheet1!$A:$A,Sheet1!$1:$2"), "{wbxml}");
}

#[test]
fn fit_to_pages_uses_the_automatic_zero_and_clearing_removes_the_elements() {
    let mut wb = blank();
    let p = PageSetup {
        fit_to_page: true,
        fit_width: 1,
        fit_height: 0,
        ..PageSetup::default()
    };
    wb.set_page_setup(0, &p).expect("sets");
    let mut again = reopen(&mut wb);
    assert_eq!(again.page_setup(0), p);
    let xml = text_of(&mut again, "xl/worksheets/sheet1.xml");
    assert!(xml.contains("<pageSetUpPr fitToPage=\"1\""), "{xml}");
    assert!(xml.contains("fitToHeight=\"0\""), "{xml}");
    assert!(!xml.contains("fitToWidth"), "{xml}");
    again
        .set_page_setup(0, &PageSetup::default())
        .expect("clears");
    let xml = text_of(&mut again, "xl/worksheets/sheet1.xml");
    assert!(!xml.contains("pageSetUpPr"), "{xml}");
    assert!(!xml.contains("rowBreaks"), "{xml}");
    assert!(!xml.contains("printOptions"), "{xml}");
    let wbxml = text_of(&mut again, "xl/workbook.xml");
    assert!(!wbxml.contains("Print_"), "{wbxml}");
}
