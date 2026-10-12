//! Sheets Insert tab: every wired control runs through `Shell::run_command` and changes the sheet.

#![allow(clippy::unwrap_used)]

use open365_opc::Limits;
use open365_preview::dialogs::{insert_chart, insert_link, insert_symbol, insert_table};
use open365_preview::ribbon::Handled;
use open365_preview::sheet::SheetModel;
use open365_preview::sheet::sheet_ext::insert::Kind;
use open365_preview::sheet::sheet_ext::view::ViewMode;
use open365_preview::{AppKind, Preview};
use open365_xlsx::{HyperlinkTarget, Workbook};

mod dialogs_common;

/// The bare control opened its dialog; Enter is OK.
fn confirm(p: &mut Preview, title: &str) {
    assert!(p.shell.dialog_open(), "{title} is open");
    assert_eq!(dialogs_common::title(p).as_deref(), Some(title));
    dialogs_common::key(p, open365_ui_core::Key::Enter);
}

fn rig() -> Preview {
    Preview::new(AppKind::Sheets, true, 1600, 1000, 1.25)
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn sheet(p: &mut Preview) -> &mut SheetModel {
    p.shell.sheet_mut()
}

/// The workbook as saved: the sheet written out and read back.
fn saved(p: &mut Preview, name: &str) -> Workbook {
    let path = std::env::temp_dir().join(name);
    sheet(p).save(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(path);
    Workbook::open(&bytes, &Limits::default()).unwrap()
}

fn fill(p: &mut Preview, rows: &[&[&str]]) {
    let s = sheet(p);
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            s.enter(r as u32 + 1, c as u32 + 1, v).unwrap();
        }
    }
}

fn sales(p: &mut Preview) {
    fill(
        p,
        &[
            &["Month", "Sales", "Costs"],
            &["Jan", "10", "4"],
            &["Feb", "12", "5"],
            &["Mar", "9", "6"],
        ],
    );
    sheet(p).select_cell((2, 2));
}

#[test]
fn table_guesses_the_headers_of_the_current_region_and_is_one_undo_step() {
    let mut p = rig();
    sales(&mut p);
    run(&mut p, "insert.tables.table");
    confirm(&mut p, "Create Table");
    assert!(!p.shell.dialog_open());
    let wb = saved(&mut p, "open365-insert-table.xlsx");
    let (_, t) = wb.tables().next().expect("a table");
    assert_eq!(t.range_text, "A1:C4");
    assert_eq!(t.header_row_count, 1);
    assert!(sheet(&mut p).undo());
    assert!(
        saved(&mut p, "open365-insert-table2.xlsx")
            .tables()
            .next()
            .is_none()
    );
}

#[test]
fn table_honours_my_table_has_headers() {
    let mut p = rig();
    sales(&mut p);
    sheet(&mut p).select_cell((1, 1));
    sheet(&mut p).extend_to((4, 3));
    run(&mut p, "insert.tables.table=no-headers;name:Data");
    let wb = saved(&mut p, "open365-insert-table3.xlsx");
    let (_, t) = wb.tables().next().expect("a table");
    assert_eq!(t.name, "Data");
    assert_eq!(
        t.range_text, "A1:C5",
        "a header row is added above when the box is cleared"
    );
}

#[test]
fn create_table_dialog_guesses_headers_like_excel() {
    let kinds = [vec![Kind::Text, Kind::Text], vec![Kind::Text, Kind::Number]];
    let d = insert_table::CreateTableDialog::new((1, 1, 2, 2), &kinds);
    assert!(d.has_headers);
    assert_eq!(d.range_text(), "$A$1:$B$2");
    assert!(!insert_table::guess_headers(&[
        vec![Kind::Number],
        vec![Kind::Number]
    ]));
    assert!(!insert_table::guess_headers(&[
        vec![Kind::Text],
        vec![Kind::Text]
    ]));
}

#[test]
fn pivot_table_goes_to_a_new_sheet_that_becomes_active() {
    let mut p = rig();
    sales(&mut p);
    run(&mut p, "insert.tables.pivot-table");
    confirm(&mut p, "Create PivotTable");
    assert!(!p.shell.dialog_open());
    assert_eq!(sheet(&mut p).sheet, 1);
    assert_eq!(sheet(&mut p).active, (3, 1));
    let wb = saved(&mut p, "open365-insert-pivot.xlsx");
    let pivots = open365_pivot::sheet_pivots(&wb, 1);
    assert_eq!(pivots.len(), 1);
    assert_eq!(pivots[0].table.name, "PivotTable1");
    assert!(open365_pivot::sheet_pivots(&wb, 0).is_empty());
}

#[test]
fn pivot_table_needs_labelled_columns() {
    let mut p = rig();
    fill(&mut p, &[&["a", ""], &["1", "2"]]);
    run(&mut p, "insert.tables.pivot-table");
    confirm(&mut p, "Create PivotTable");
    assert!(p.shell.dialog_open(), "the error keeps the dialog open");
    assert!(
        sheet(&mut p)
            .status
            .starts_with("The PivotTable field name is not valid")
    );
    dialogs_common::key(&mut p, open365_ui_core::Key::Escape);
    assert_eq!(sheet(&mut p).sheet, 0);
}

fn chart_of(p: &mut Preview, name: &str) -> open365_chart::SheetChart {
    let wb = saved(p, name);
    let mut charts = open365_chart::sheet_charts(&wb, 0);
    assert_eq!(charts.len(), 1);
    charts.remove(0)
}

#[test]
fn recommended_charts_inserts_a_clustered_column_of_the_region() {
    let mut p = rig();
    sales(&mut p);
    run(&mut p, "insert.charts.recommended-charts");
    let c = chart_of(&mut p, "open365-insert-chart.xlsx");
    let xml = String::from_utf8(c.space.to_bytes()).unwrap();
    assert!(
        xml.contains("<c:barChart>") && xml.contains("barDir val=\"col\""),
        "{xml}"
    );
    assert!(
        xml.contains("Sheet1!$B$1") && xml.contains("Sheet1!$C$2:$C$4"),
        "{xml}"
    );
    assert!(
        xml.contains("Sheet1!$A$2:$A$4"),
        "categories come from the first column"
    );
    assert!(
        xml.contains("Chart Title"),
        "several series get the placeholder title"
    );
    assert!(xml.contains("<c:legend>"));
    assert!(sheet(&mut p).undo());
    assert!(
        open365_chart::sheet_charts(&saved(&mut p, "open365-insert-chart2.xlsx"), 0).is_empty()
    );
}

#[test]
fn a_chart_reads_an_offset_selection_and_names_a_single_series_after_it() {
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["", "", ""],
            &["", "Item", "Qty"],
            &["", "a", "3"],
            &["", "b", "5"],
        ],
    );
    sheet(&mut p).select_cell((2, 2));
    sheet(&mut p).extend_to((4, 3));
    run(
        &mut p,
        "insert.charts.insert-line-or-area-chart.line-with-markers",
    );
    let c = chart_of(&mut p, "open365-insert-chart3.xlsx");
    let xml = String::from_utf8(c.space.to_bytes()).unwrap();
    assert!(xml.contains("<c:lineChart>"), "{xml}");
    assert!(
        xml.contains("Sheet1!$C$2")
            && xml.contains("Sheet1!$C$3:$C$4")
            && xml.contains("Sheet1!$B$3:$B$4"),
        "{xml}"
    );
    assert!(!xml.contains("<c:legend>"), "a single series has no legend");
    assert!(xml.contains(">Qty<"), "the title is the series name");
}

#[test]
fn rows_become_series_when_the_selection_is_wider_than_tall() {
    let mut p = rig();
    fill(
        &mut p,
        &[
            &["", "Q1", "Q2", "Q3"],
            &["North", "1", "2", "3"],
            &["South", "4", "5", "6"],
        ],
    );
    sheet(&mut p).select_cell((1, 1));
    run(
        &mut p,
        "insert.charts.insert-column-or-bar-chart.stacked-column",
    );
    let c = chart_of(&mut p, "open365-insert-chart4.xlsx");
    let xml = String::from_utf8(c.space.to_bytes()).unwrap();
    assert!(
        xml.contains("Sheet1!$A$2") && xml.contains("Sheet1!$B$2:$D$2"),
        "{xml}"
    );
    assert!(
        xml.contains("Sheet1!$B$1:$D$1"),
        "categories are the header row"
    );
}

#[test]
fn pie_uses_the_first_series_only_and_unknown_types_are_refused() {
    let mut p = rig();
    sales(&mut p);
    run(&mut p, "insert.charts.insert-pie-or-doughnut-chart");
    let c = chart_of(&mut p, "open365-insert-chart5.xlsx");
    let xml = String::from_utf8(c.space.to_bytes()).unwrap();
    assert!(
        xml.contains("<c:pieChart>") && !xml.contains("$C$2"),
        "{xml}"
    );
    run(
        &mut p,
        "insert.charts.insert-waterfall-funnel-stock-surface-or-radar-chart.waterfall",
    );
    assert!(sheet(&mut p).status.contains("not available"));
}

#[test]
fn chart_kinds_and_plan() {
    assert!(insert_chart::kind_of("insert-column-or-bar-chart", "clustered-bar").is_some());
    assert!(insert_chart::kind_of("insert-column-or-bar-chart", "3d").is_none());
    let k = [
        vec![Kind::Empty, Kind::Text],
        vec![Kind::Text, Kind::Number],
    ];
    let plan = insert_chart::plan(&k, false).unwrap();
    assert!(plan.cat_line == Some(0) && plan.header_k == Some(0));
    assert!(insert_chart::plan(&[vec![Kind::Text]], false).is_none());
}

#[test]
fn link_sets_a_hyperlink_and_remove_clears_it() {
    let mut p = rig();
    sheet(&mut p).select_cell((2, 2));
    run(&mut p, "insert.links.link=www.example.com|Example|The site");
    assert_eq!(sheet(&mut p).source(2, 2), "Example");
    let wb = saved(&mut p, "open365-insert-link.xlsx");
    let links = wb.hyperlinks(0);
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].target,
        HyperlinkTarget::Url("http://www.example.com".to_owned())
    );
    assert_eq!(links[0].tooltip.as_deref(), Some("The site"));
    run(&mut p, "insert.links.link.remove");
    assert!(
        saved(&mut p, "open365-insert-link2.xlsx")
            .hyperlinks(0)
            .is_empty()
    );
}

#[test]
fn link_to_a_place_shows_its_address_in_an_empty_cell() {
    let mut p = rig();
    run(&mut p, "insert.links.link=#Sheet1!B5");
    assert_eq!(sheet(&mut p).source(1, 1), "Sheet1!B5");
    run(&mut p, "insert.links.link");
    confirm(&mut p, "Insert Hyperlink");
    assert!(p.shell.dialog_open(), "no address: the dialog stays open");
    assert!(
        dialogs_common::dump(&p)
            .iter()
            .any(|l| l.contains("address"))
    );
    dialogs_common::key(&mut p, open365_ui_core::Key::Escape);
}

#[test]
fn link_targets_parse_like_the_dialog() {
    use open365_commands::objects_ops::LinkKind;
    assert_eq!(
        insert_link::parse_target("mailto:a@b.co?subject=Hi").unwrap(),
        LinkKind::Email {
            address: "a@b.co".into(),
            subject: Some("Hi".into())
        }
    );
    assert!(matches!(
        insert_link::parse_target("https://x.org").unwrap(),
        LinkKind::Url { .. }
    ));
    assert!(insert_link::parse_target("  ").is_err());
}

#[test]
fn symbol_is_appended_to_the_active_cell() {
    let mut p = rig();
    fill(&mut p, &[&["Price "]]);
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "insert.symbols.symbol=U+00A3");
    assert_eq!(sheet(&mut p).source(1, 1), "Price \u{a3}");
    assert!(sheet(&mut p).undo());
    assert_eq!(sheet(&mut p).source(1, 1), "Price ");
    assert_eq!(
        insert_symbol::parse_symbol("0x20AC").as_deref(),
        Some("\u{20ac}")
    );
    assert!(insert_symbol::subset_chars("Arrows").contains(&'\u{2192}'));
}

#[test]
fn comment_adds_a_threaded_comment() {
    let mut p = rig();
    sheet(&mut p).select_cell((2, 3));
    run(&mut p, "insert.comments.comment=Check this");
    let c = sheet(&mut p).review_comment_at(2, 3).unwrap();
    assert!(format!("{c:?}").contains("Check this"));
}

#[test]
fn header_and_footer_switches_to_page_layout_view() {
    let mut p = rig();
    run(&mut p, "insert.text.text.header-footer");
    assert_eq!(sheet(&mut p).ext_view.mode, ViewMode::PageLayout);
    run(&mut p, "insert.text.text.text-box");
    assert!(sheet(&mut p).status.contains("not available"));
}
