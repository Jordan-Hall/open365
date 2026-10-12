//! Sheets View tab and File tab controls run through `Shell::run_command` and change the model.

#![allow(clippy::unwrap_used)]

use open365_preview::ribbon::Handled;
use open365_preview::sheet::sheet_ext::view::ViewMode;
use open365_preview::{AppKind, Preview};

fn rig() -> Preview {
    Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25)
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

#[test]
fn workbook_views_switch() {
    let mut p = rig();
    assert_eq!(p.shell.sheet().ext_view.mode, ViewMode::Normal);
    run(&mut p, "view.workbook-views.page-break-preview");
    assert_eq!(p.shell.sheet().ext_view.mode, ViewMode::PageBreakPreview);
    run(&mut p, "view.workbook-views.page-layout");
    assert_eq!(p.shell.sheet().ext_view.mode, ViewMode::PageLayout);
    run(&mut p, "view.workbook-views.normal");
    assert_eq!(p.shell.sheet().ext_view.mode, ViewMode::Normal);
}

#[test]
fn show_options_toggle() {
    let mut p = rig();
    let s = p.shell.sheet().sheet;
    run(&mut p, "view.show.gridlines");
    assert!(!p.shell.sheet().ext_view.gridlines(s));
    run(&mut p, "view.show.gridlines");
    assert!(p.shell.sheet().ext_view.gridlines(s));
    run(&mut p, "view.show.headings=off");
    assert!(!p.shell.sheet().ext_view.headings(s));
    run(&mut p, "view.show.formula-bar");
    assert!(!p.shell.sheet().ext_view.formula_bar);
    // Page Layout > Sheet Options > View is the same flag.
    run(&mut p, "page-layout.sheet-options.view-2");
    assert!(p.shell.sheet().ext_view.headings(s));
}

#[test]
fn zoom_is_clamped_and_resets() {
    let mut p = rig();
    run(&mut p, "view.zoom.zoom=150%");
    assert_eq!(p.shell.sheet().ext_view.zoom, 150);
    run(&mut p, "view.zoom.zoom=9999");
    assert_eq!(p.shell.sheet().ext_view.zoom, 400);
    run(&mut p, "view.zoom.zoom=1");
    assert_eq!(p.shell.sheet().ext_view.zoom, 10);
    run(&mut p, "view.zoom.100");
    assert_eq!(p.shell.sheet().ext_view.zoom, 100);
}

#[test]
fn zoom_to_selection_fits_the_range() {
    let mut p = rig();
    // One cell zooms to the maximum, a big block to something smaller.
    run(&mut p, "view.zoom.zoom-to-selection");
    assert_eq!(p.shell.sheet().ext_view.zoom, 400);
    p.shell.sheet_mut().anchor = (1, 1);
    p.shell.sheet_mut().focus = (30, 20);
    run(&mut p, "view.zoom.zoom-to-selection");
    let z = p.shell.sheet().ext_view.zoom;
    assert!((10..100).contains(&z), "{z}");
}

#[test]
fn freeze_panes_drive_the_grid_geometry() {
    let mut p = rig();
    p.shell.sheet_mut().select_cell((4, 3));
    run(&mut p, "view.window.freeze-panes.freeze-panes");
    assert_eq!(p.shell.sheet().frozen(), (3, 2));
    assert_eq!(p.shell.sheet().top_left, (4, 3));
    // The same item now unfreezes.
    run(&mut p, "view.window.freeze-panes.freeze-panes");
    assert_eq!(p.shell.sheet().frozen(), (0, 0));
    run(&mut p, "view.window.freeze-panes.freeze-top-row");
    assert_eq!(p.shell.sheet().frozen(), (1, 0));
    run(&mut p, "view.window.freeze-panes.freeze-first-column");
    assert_eq!(p.shell.sheet().frozen(), (0, 1));
    run(&mut p, "view.window.freeze-panes#1");
    assert_eq!(p.shell.sheet().frozen(), (1, 0));
    run(&mut p, "view.window.freeze-panes.unfreeze");
    assert_eq!(p.shell.sheet().frozen(), (0, 0));
}

#[test]
fn split_and_window_commands() {
    let mut p = rig();
    p.shell.sheet_mut().select_cell((5, 4));
    let s = p.shell.sheet().sheet;
    run(&mut p, "view.window.split");
    assert_eq!(p.shell.sheet().ext_view.split(s), Some((5, 4)));
    run(&mut p, "view.window.split");
    assert_eq!(p.shell.sheet().ext_view.split(s), None);
    run(&mut p, "view.window.new-window");
    assert_eq!(p.shell.sheet().ext_view.windows, 2);
    run(&mut p, "view.window.hide");
    assert!(p.shell.sheet().ext_view.hidden);
    run(&mut p, "view.window.unhide");
    assert!(!p.shell.sheet().ext_view.hidden);
}

#[test]
fn file_save_as_open_new_and_close() {
    let dir = std::env::temp_dir().join(format!("o365-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("book.xlsx");
    let mut p = rig();
    p.shell.sheet_mut().enter(1, 1, "42").unwrap();
    // A dirty workbook is never dropped by New or Open.
    run(&mut p, "file.new");
    assert!(p.shell.sheet().is_dirty());
    assert!(p.shell.sheet().status.contains("Save"));
    run(&mut p, &format!("file.save-as={}", path.display()));
    assert!(path.exists());
    assert!(!p.shell.sheet().is_dirty());
    p.shell.sheet_mut().enter(1, 1, "43").unwrap();
    run(&mut p, "file.save");
    assert!(!p.shell.sheet().is_dirty());
    run(&mut p, "file.new");
    assert_eq!(p.shell.sheet().display(1, 1), "");
    run(&mut p, &format!("file.open={}", path.display()));
    assert_eq!(p.shell.sheet().display(1, 1), "43");
    assert_eq!(p.shell.sheet().path().as_deref(), Some(path.as_path()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn export_pdf_and_print_use_the_page_setup() {
    let dir = std::env::temp_dir().join(format!("o365-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = rig();
    // Nothing to print on an empty sheet.
    run(&mut p, "file.print");
    assert!(p.shell.sheet().status.contains("anything to print"));
    for r in 1..=120 {
        p.shell
            .sheet_mut()
            .enter(r, 1, &format!("row {r}"))
            .unwrap();
        p.shell.sheet_mut().enter(r, 2, &r.to_string()).unwrap();
    }
    let portrait = p.shell.sheet().print_pages().len();
    assert!(portrait >= 2, "{portrait}");
    run(&mut p, "page-layout.scale-to-fit.height=1 page");
    assert_eq!(p.shell.sheet().print_pages().len(), 1);
    run(&mut p, "file.print");
    assert!(p.shell.sheet().status.starts_with("1 page"));
    let pdf = dir.join("out.pdf");
    run(&mut p, &format!("file.export-pdf={}", pdf.display()));
    let bytes = std::fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Count 1"));
    assert!(text.contains("(row 120)"));
    std::fs::remove_dir_all(&dir).ok();
}
