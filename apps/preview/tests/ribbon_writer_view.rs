//! Writer View tab controls run through `Shell::run_command`.

#![allow(clippy::unwrap_used)]

use open365_preview::doc::WriterPane;
use open365_preview::ribbon::Handled;
use open365_preview::ribbon::writer::view::ViewMode;
use open365_preview::{AppKind, Preview};

fn rig() -> Preview {
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text("hello");
    p.shell.set_writer(&mut p.ui, pane);
    p
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn pane(p: &Preview) -> &WriterPane {
    p.shell.writer().unwrap()
}

#[test]
fn views_switch_mode() {
    let mut p = rig();
    for (id, mode) in [
        ("view.views.read-mode", ViewMode::Read),
        ("view.views.web-layout", ViewMode::Web),
        ("view.views.outline", ViewMode::Outline),
        ("view.views.draft", ViewMode::Draft),
        ("view.views.print-layout", ViewMode::PrintLayout),
    ] {
        run(&mut p, id);
        assert_eq!(pane(&p).view_state().mode, mode, "{id}");
    }
    run(&mut p, "view.immersive.reader");
    assert_eq!(pane(&p).view_state().mode, ViewMode::Read);
    run(&mut p, "view.immersive.reader");
    assert_eq!(pane(&p).view_state().mode, ViewMode::PrintLayout);
}

#[test]
fn show_options_toggle() {
    let mut p = rig();
    run(&mut p, "view.show.ruler");
    assert!(pane(&p).ruler());
    run(&mut p, "view.show.ruler");
    assert!(!pane(&p).ruler());
    run(&mut p, "view.show.gridlines");
    run(&mut p, "view.show.navigation-pane");
    assert!(pane(&p).view_state().gridlines && pane(&p).view_state().nav_pane);
    run(&mut p, "view.show.gridlines");
    assert!(!pane(&p).view_state().gridlines);
}

#[test]
fn zoom_presets() {
    let mut p = rig();
    run(&mut p, "view.zoom.zoom=150");
    assert_eq!(pane(&p).zoom_percent(), 150.0);
    run(&mut p, "view.zoom.100");
    assert_eq!(pane(&p).zoom_percent(), 100.0);
    run(&mut p, "view.zoom.page-width");
    let width = pane(&p).zoom_percent();
    run(&mut p, "view.zoom.one-page");
    let one = pane(&p).zoom_percent();
    run(&mut p, "view.zoom.multiple-pages");
    let many = pane(&p).zoom_percent();
    assert!(one <= width, "one page {one} <= page width {width}");
    assert!(many < width, "two pages across {many} < page width {width}");
    assert!(one >= 10.0 && many >= 10.0);
}

#[test]
fn window_and_macros_are_handled() {
    let mut p = rig();
    run(&mut p, "view.window.split");
    assert!(pane(&p).view_state().split);
    run(&mut p, "view.window.split");
    assert!(!pane(&p).view_state().split);
    run(&mut p, "view.window.synchronous-scrolling");
    assert!(!pane(&p).view_state().sync_scroll);
    run(&mut p, "view.macros.macros");
    assert!(pane(&p).status.as_deref().unwrap().contains("Macros"));
    for id in [
        "view.window.new-window",
        "view.window.arrange-all",
        "view.window.switch-windows",
        "view.window.view-side-by-side",
        "view.window.reset-window-position",
        "view.server.properties",
        "view.appearance.switch-theme",
        "view.immersive.focus",
        "view.page-movement.side-to-side",
        "view.page-movement.vertical",
    ] {
        run(&mut p, id);
    }
    assert!(!pane(&p).view_state().side_to_side);
}

#[test]
fn help_stubs_name_local_docs() {
    let mut p = rig();
    for id in [
        "help.help.help",
        "help.help.feedback",
        "help.help.show-training",
        "help.help.what-s-new",
        "help.mobile.get-mobile-app",
    ] {
        run(&mut p, id);
        assert!(pane(&p).status.is_some(), "{id}");
    }
}
