//! Sheets Page Layout tab controls run through `Shell::run_command` and change the page setup.

#![allow(clippy::unwrap_used)]

use open365_preview::ribbon::Handled;
use open365_preview::sheet::sheet_ext::page_layout::{Margins, Paper};
use open365_preview::{AppKind, Preview};

mod dialogs_common;

fn rig() -> Preview {
    Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25)
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

#[test]
fn margins_orientation_and_size() {
    let mut p = rig();
    assert_eq!(p.shell.sheet().page().margins, Margins::NORMAL);
    run(&mut p, "page-layout.page-set-up.margins.narrow");
    assert_eq!(p.shell.sheet().page().margins, Margins::NARROW);
    run(&mut p, "page-layout.page-set-up.margins#1");
    assert_eq!(p.shell.sheet().page().margins, Margins::WIDE);
    run(&mut p, "page-layout.page-set-up.margins.custom=1,2,3,4");
    let m = p.shell.sheet().page().margins;
    assert_eq!((m.top, m.bottom, m.left, m.right), (1.0, 2.0, 3.0, 4.0));
    run(&mut p, "page-layout.page-set-up.orientation.landscape");
    assert!(p.shell.sheet().page().landscape);
    run(&mut p, "page-layout.page-set-up.orientation#0");
    assert!(!p.shell.sheet().page().landscape);
    run(&mut p, "page-layout.page-set-up.size.letter");
    assert_eq!(p.shell.sheet().page().paper, Paper::Letter);
    run(&mut p, "page-layout.page-set-up.size#5");
    assert_eq!(p.shell.sheet().page().paper, Paper::A3);
}

#[test]
fn print_area_and_breaks() {
    let mut p = rig();
    p.shell.sheet_mut().anchor = (2, 2);
    p.shell.sheet_mut().focus = (5, 4);
    p.shell.sheet_mut().active = (2, 2);
    run(&mut p, "page-layout.page-set-up.print-area.set");
    assert_eq!(p.shell.sheet().page().print_areas, vec![(2, 2, 5, 4)]);
    p.shell.sheet_mut().select_cell((8, 8));
    run(&mut p, "page-layout.page-set-up.print-area.add");
    assert_eq!(p.shell.sheet().page().print_areas.len(), 2);
    run(&mut p, "page-layout.page-set-up.print-area.clear");
    assert!(p.shell.sheet().page().print_areas.is_empty());

    p.shell.sheet_mut().select_cell((10, 3));
    run(&mut p, "page-layout.page-set-up.breaks.insert");
    let pg = p.shell.sheet().page();
    assert!(pg.row_breaks.contains(&10) && pg.col_breaks.contains(&3));
    assert!(p.shell.sheet().on_page_break());
    run(&mut p, "page-layout.page-set-up.breaks.remove");
    assert!(p.shell.sheet().page().row_breaks.is_empty());
    // A break at row 1 or column A does not exist.
    p.shell.sheet_mut().select_cell((1, 1));
    run(&mut p, "page-layout.page-set-up.breaks.insert");
    assert!(p.shell.sheet().page().row_breaks.is_empty());
    p.shell.sheet_mut().select_cell((4, 4));
    run(&mut p, "page-layout.page-set-up.breaks.insert");
    run(&mut p, "page-layout.page-set-up.breaks.reset");
    assert!(p.shell.sheet().page().col_breaks.is_empty());
}

#[test]
fn a_manual_break_starts_a_new_page() {
    let mut p = rig();
    for r in 1..=6 {
        p.shell.sheet_mut().enter(r, 1, "x").unwrap();
    }
    assert_eq!(p.shell.sheet().print_pages().len(), 1);
    p.shell.sheet_mut().select_cell((4, 1));
    run(&mut p, "page-layout.page-set-up.breaks.insert");
    let pages = p.shell.sheet().print_pages();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].rows, vec![4, 5, 6]);
    // Print titles repeat on the second page.
    run(&mut p, "page-layout.page-set-up.print-titles=$1:$1");
    assert_eq!(p.shell.sheet().print_pages()[1].rows, vec![1, 4, 5, 6]);
    run(&mut p, "page-layout.page-set-up.print-titles=clear");
    assert!(p.shell.sheet().page().title_rows.is_none());
}

#[test]
fn print_titles_come_from_the_selection() {
    let mut p = rig();
    p.shell.sheet_mut().select_cell((2, 2));
    // The bare button opens Page Setup (Sheet tab); Esc leaves the page untouched.
    run(&mut p, "page-layout.page-set-up.print-titles");
    assert!(p.shell.dialog_open());
    assert_eq!(dialogs_common::title(&p).as_deref(), Some("Page Setup"));
    dialogs_common::key(&mut p, open365_ui_core::Key::Escape);
    assert!(!p.shell.dialog_open());
    assert!(p.shell.sheet().page().title_rows.is_none());
    // The Rows to repeat box takes the selection: a cell is refused, whole rows are taken.
    let err = p
        .shell
        .sheet_mut()
        .set_print_titles_from_selection()
        .unwrap_err();
    assert!(err.contains("entire rows"));
    p.shell.sheet_mut().select_rows(2, 3, false);
    p.shell
        .sheet_mut()
        .set_print_titles_from_selection()
        .unwrap();
    assert_eq!(p.shell.sheet().page().title_rows, Some((2, 3)));
    run(&mut p, "page-layout.page-set-up.print-titles=$A:$B");
    assert_eq!(p.shell.sheet().page().title_cols, Some((1, 2)));
}

#[test]
fn background_sets_and_deletes() {
    let mut p = rig();
    run(&mut p, "page-layout.page-set-up.background=C:/pics/bg.png");
    assert!(p.shell.sheet().page().background.is_some());
    run(&mut p, "page-layout.page-set-up.background");
    assert!(p.shell.sheet().page().background.is_none());
}

#[test]
fn scale_to_fit_and_scale_exclude_each_other() {
    let mut p = rig();
    run(&mut p, "page-layout.scale-to-fit.width=2 pages");
    assert_eq!(p.shell.sheet().page().fit_width, 2);
    run(&mut p, "page-layout.scale-to-fit.height#1");
    assert_eq!(p.shell.sheet().page().fit_height, 1);
    // Setting the scale puts Width and Height back to Automatic.
    run(&mut p, "page-layout.scale-to-fit.scale=80%");
    let pg = p.shell.sheet().page();
    assert_eq!((pg.scale, pg.fit_width, pg.fit_height), (80, 0, 0));
    run(&mut p, "page-layout.scale-to-fit.width=Automatic");
    assert_eq!(p.shell.sheet().page().fit_width, 0);
    run(&mut p, "page-layout.scale-to-fit.scale=5000");
    assert_eq!(p.shell.sheet().page().scale, 400);
}

#[test]
fn sheet_options_and_theme() {
    let mut p = rig();
    let s = p.shell.sheet().sheet;
    run(&mut p, "page-layout.sheet-options.view");
    assert!(!p.shell.sheet().ext_view.gridlines(s));
    run(&mut p, "page-layout.sheet-options.print");
    assert!(p.shell.sheet().page().print_gridlines);
    run(&mut p, "page-layout.sheet-options.print-2");
    assert!(p.shell.sheet().page().print_headings);
    run(&mut p, "page-layout.sheet-options.print-2");
    assert!(!p.shell.sheet().page().print_headings);
    run(&mut p, "page-layout.themes.colours.blue-warm");
    assert_eq!(p.shell.sheet().ext_page.theme.colours, "blue-warm");
    assert_eq!(p.shell.sheet().ext_page.theme.fonts, "Office");
}
