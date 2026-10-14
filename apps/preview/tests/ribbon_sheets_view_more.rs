//! View tab: Custom Views, and the window commands the shell cannot host.

#![allow(clippy::unwrap_used)]

use open365_preview::ribbon::Handled;
use open365_preview::sheet::SheetModel;
use open365_preview::sheet::sheet_ext::view::ViewMode;
use open365_preview::{AppKind, Preview};

fn rig() -> Preview {
    Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25)
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

#[test]
fn custom_views_save_show_delete_and_survive_a_save() {
    let mut p = rig();
    let s = p.shell.sheet().sheet;
    run(&mut p, "view.zoom.zoom=150");
    run(&mut p, "view.show.gridlines=off");
    run(&mut p, "view.workbook-views.page-break-preview");
    p.shell.sheet_mut().select_cell((4, 3));
    run(&mut p, "view.window.freeze-panes.freeze-panes");
    run(&mut p, "view.workbook-views.custom-views=add:Review mode");

    // Change everything, then show the view again.
    run(&mut p, "view.zoom.100");
    run(&mut p, "view.show.gridlines=on");
    run(&mut p, "view.workbook-views.normal");
    run(&mut p, "view.window.freeze-panes.unfreeze");
    p.shell.sheet_mut().select_cell((9, 9));
    run(&mut p, "view.workbook-views.custom-views=show:review MODE");
    let m = p.shell.sheet();
    assert_eq!(m.ext_view.zoom, 150);
    assert!(!m.ext_view.gridlines(s));
    assert_eq!(m.ext_view.mode, ViewMode::PageBreakPreview);
    assert_eq!(m.frozen(), (3, 2));
    assert_eq!(m.active, (4, 3));

    let views = p.shell.sheet().view_custom_views();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].name, "Review mode");

    let path = std::env::temp_dir().join("open365-custom-views.xlsx");
    p.shell.sheet_mut().save(&path).unwrap();
    let again = SheetModel::open(&path).unwrap();
    let _ = std::fs::remove_file(path);
    assert_eq!(again.view_custom_views(), views);

    // Adding the same name replaces it; Delete removes it, and undo brings it back.
    run(&mut p, "view.zoom.zoom=80");
    run(&mut p, "view.workbook-views.custom-views=add:Review mode");
    let v = p.shell.sheet().view_custom_views();
    assert_eq!((v.len(), v[0].sheets[0].zoom), (1, 80));
    run(
        &mut p,
        "view.workbook-views.custom-views=delete:Review mode",
    );
    assert!(p.shell.sheet().view_custom_views().is_empty());
    assert!(p.shell.sheet_mut().undo());
    assert_eq!(p.shell.sheet().view_custom_views().len(), 1);

    run(&mut p, "view.workbook-views.custom-views=show:Nothing");
    assert!(
        p.shell
            .sheet()
            .status
            .contains("There is no view with that name.")
    );
}

#[test]
fn multi_window_and_macro_controls_stay_unhandled() {
    let mut p = rig();
    for id in [
        "view.window.new-window",
        "view.window.arrange-all",
        "view.window.switch-windows",
        "view.window.reset-window-position",
        "view.window.view-side-by-side",
        "view.window.synchronous-scrolling",
        "view.macros.macros",
    ] {
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::No, "{id}");
    }
}

#[test]
fn zoom_to_selection_fills_the_window() {
    let mut p = rig();
    p.shell.sheet_mut().anchor = (2, 2);
    p.shell.sheet_mut().focus = (6, 5);
    p.shell.sheet_mut().active = (2, 2);
    run(&mut p, "view.zoom.zoom-to-selection");
    let z = p.shell.sheet().ext_view.zoom;
    assert!(z > 100 && z <= 400, "{z}");
    assert_eq!(p.shell.sheet().top_left, (2, 2));
}
