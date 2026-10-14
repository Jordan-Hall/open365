//! Page Layout tab: the page setup is written to the worksheet and read back.

#![allow(clippy::unwrap_used)]

use open365_preview::ribbon::Handled;
use open365_preview::sheet::SheetModel;
use open365_preview::sheet::sheet_ext::page_layout::{Margins, Paper};
use open365_preview::{AppKind, Preview};

fn rig() -> Preview {
    Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25)
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

#[test]
fn page_setup_is_saved_in_the_file_and_reopened() {
    let mut p = rig();
    run(&mut p, "page-layout.page-set-up.margins.narrow");
    run(&mut p, "page-layout.page-set-up.orientation.landscape");
    run(&mut p, "page-layout.page-set-up.size.letter");
    p.shell.sheet_mut().anchor = (2, 2);
    p.shell.sheet_mut().focus = (9, 5);
    p.shell.sheet_mut().active = (2, 2);
    run(&mut p, "page-layout.page-set-up.print-area.set");
    p.shell.sheet_mut().select_cell((12, 4));
    run(&mut p, "page-layout.page-set-up.breaks.insert");
    run(&mut p, "page-layout.page-set-up.print-titles=$1:$2");
    run(&mut p, "page-layout.page-set-up.print-titles=$A:$B");
    run(&mut p, "page-layout.sheet-options.print=on");
    run(&mut p, "page-layout.sheet-options.print-2=on");
    run(&mut p, "page-layout.scale-to-fit.width=2 page");

    let path = std::env::temp_dir().join("open365-page-setup.xlsx");
    p.shell.sheet_mut().save(&path).unwrap();
    let again = SheetModel::open(&path).unwrap();
    let _ = std::fs::remove_file(path);
    let page = again.page_from_file();
    assert_eq!(page.margins, Margins::NARROW);
    assert!(page.landscape);
    assert_eq!(page.paper, Paper::Letter);
    assert_eq!(page.print_areas, vec![(2, 2, 9, 5)]);
    assert!(page.row_breaks.contains(&12) && page.col_breaks.contains(&4));
    assert_eq!(
        (page.title_rows, page.title_cols),
        (Some((1, 2)), Some((1, 2)))
    );
    assert!(page.print_gridlines && page.print_headings);
    assert_eq!((page.fit_width, page.fit_height), (2, 0));
}

#[test]
fn each_change_is_one_undo_step_and_unchanged_commands_add_none() {
    let mut p = rig();
    run(&mut p, "page-layout.page-set-up.orientation.landscape");
    assert!(p.shell.sheet().page_file_setup(0).landscape);
    // Repeating the same choice changes nothing in the file, so it is not a second step.
    run(&mut p, "page-layout.page-set-up.orientation.landscape");
    assert!(p.shell.sheet_mut().undo());
    assert!(!p.shell.sheet().page_file_setup(0).landscape);
    assert!(!p.shell.sheet().can_undo());
}

#[test]
fn undo_restores_the_file_and_the_next_command_starts_from_it() {
    let mut p = rig();
    run(&mut p, "page-layout.page-set-up.margins.wide");
    run(&mut p, "page-layout.page-set-up.size.a5");
    assert!(p.shell.sheet_mut().undo());
    assert!(p.shell.sheet_mut().undo());
    run(&mut p, "page-layout.page-set-up.orientation.landscape");
    let page = p.shell.sheet().page();
    assert_eq!(page.margins, Margins::NORMAL);
    assert!(page.landscape);
    assert_eq!(page.paper, Paper::A4);
}
