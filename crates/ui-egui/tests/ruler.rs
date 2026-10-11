//! The document area with and without the rulers: the page moves down by the 30 u band and right by half the 30 u
//! vertical band (Word, `word-page.json`), and the ruler describes the caret's paragraph.

use std::path::PathBuf;

use egui::vec2;
use egui_kittest::Harness;
use open365_commands::Target;
use open365_ui_egui::ruler::{RulerEdit, RulerTab};
use open365_ui_egui::{DocView, RulerColors};

struct State {
    view: DocView,
    target: Target,
}

fn harness(ruler: bool) -> Harness<'static, State> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/docx/plain-paragraphs.docx");
    let bytes = std::fs::read(path).expect("fixture");
    let mut view = DocView::new();
    view.set_zoom(Some(1.0));
    view.set_ruler(ruler);
    view.set_ruler_colors(RulerColors::default());
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 800.0))
        .build_ui_state(
            |ui, s: &mut State| {
                s.view.show(ui, &mut s.target);
            },
            State {
                view,
                target: Target::open_docx(&bytes).expect("open"),
            },
        );
    h.run();
    h
}

#[test]
fn page_starts_21_u_under_the_canvas() {
    let h = harness(false);
    let top = h.state().view.page_point_to_screen(0, 0.0, 0.0);
    // 21 u = 16.8 points under the canvas; the harness panel adds an 8 point margin.
    assert!((top.y - 8.0 - 16.8).abs() < 0.2, "{top:?}");
}

#[test]
fn ruler_moves_the_page_30_u_down_and_15_u_right() {
    let off = harness(false);
    let on = harness(true);
    let a = off.state().view.page_point_to_screen(0, 0.0, 0.0);
    let b = on.state().view.page_point_to_screen(0, 0.0, 0.0);
    assert!((b.y - a.y - 24.0).abs() < 0.2, "down {a:?} {b:?}");
    assert!((b.x - a.x - 12.0).abs() < 0.2, "right {a:?} {b:?}");
}

#[test]
fn ruler_info_reports_page_and_paragraph() {
    let h = harness(true);
    let s = h.state();
    let info = s.view.ruler_info(&s.target).expect("info");
    assert_eq!(info.para, 0);
    assert!(info.page_width_pt > 400.0 && info.margin_left_pt > 0.0);
    assert!(s.view.ruler());
    assert_eq!(s.view.ruler_tab_kind(), RulerTab::Left);
}

#[test]
fn ruler_edits_run_through_commands_and_undo() {
    let mut h = harness(true);
    let s = h.state_mut();
    let indents = RulerEdit::SetIndents {
        para: 0,
        left_pt: 36.0,
        first_line_pt: -18.0,
        right_pt: 9.0,
    };
    assert!(s.view.apply_ruler_edit(&mut s.target, &indents));
    let add = RulerEdit::AddTab {
        para: 0,
        pos_pt: 144.0,
        kind: RulerTab::Right,
    };
    assert!(s.view.apply_ruler_edit(&mut s.target, &add));
    let info = s.view.ruler_info(&s.target).expect("info");
    assert_eq!(
        (
            info.indent_left_pt,
            info.first_line_pt,
            info.indent_right_pt
        ),
        (36.0, -18.0, 9.0)
    );
    assert_eq!(info.tabs.len(), 1);
    assert_eq!(info.tabs[0].kind, RulerTab::Right);
    assert!((info.tabs[0].pos_pt - 144.0).abs() < 1e-9);

    let moved = RulerEdit::MoveTab {
        para: 0,
        from_pt: 144.0,
        to_pt: 216.0,
    };
    assert!(s.view.apply_ruler_edit(&mut s.target, &moved));
    let info = s.view.ruler_info(&s.target).expect("info");
    assert!((info.tabs[0].pos_pt - 216.0).abs() < 1e-9);
    let clear = RulerEdit::ClearTab {
        para: 0,
        pos_pt: 216.0,
    };
    assert!(s.view.apply_ruler_edit(&mut s.target, &clear));
    assert!(s.view.ruler_info(&s.target).expect("info").tabs.is_empty());
    // A stop that is not there fails and says why.
    assert!(!s.view.apply_ruler_edit(&mut s.target, &clear));
    assert!(s.view.status.is_some());

    // Undo (clear, move, add) puts the paragraph back; the view follows after a frame.
    for _ in 0..3 {
        s.view.undo(&mut s.target).expect("undo");
    }
    h.run();
    let s = h.state();
    let info = s.view.ruler_info(&s.target).expect("info");
    assert!(info.tabs.is_empty());
    assert_eq!(
        (
            info.indent_left_pt,
            info.first_line_pt,
            info.indent_right_pt
        ),
        (36.0, -18.0, 9.0)
    );
    let s = h.state_mut();
    s.view.undo(&mut s.target).expect("undo indents");
    s.view.redo(&mut s.target).expect("redo indents");
    h.run();
    let s = h.state();
    assert_eq!(
        s.view.ruler_info(&s.target).expect("info").indent_left_pt,
        36.0
    );
}

/// Word applies a ruler edit to every paragraph of the selection, not only the caret's.
#[test]
fn ruler_edits_apply_to_every_selected_paragraph() {
    let mut h = harness(true);
    let s = h.state_mut();
    let pos = |para| open365_ui_egui::DocPos { para, offset: 0 };
    s.view.select(pos(0), pos(1));
    let edit = RulerEdit::SetIndents {
        para: 0,
        left_pt: 36.0,
        first_line_pt: 0.0,
        right_pt: 0.0,
    };
    assert!(s.view.apply_ruler_edit(&mut s.target, &edit));
    let add = RulerEdit::AddTab {
        para: 1,
        pos_pt: 144.0,
        kind: RulerTab::Left,
    };
    assert!(s.view.apply_ruler_edit(&mut s.target, &add));
    // The caret sits in paragraph 1, which got the indent and the tab as well.
    let info = s.view.ruler_info(&s.target).expect("info");
    assert_eq!(info.para, 1);
    assert_eq!(info.indent_left_pt, 36.0);
    assert_eq!(info.tabs.len(), 1);
    // A paragraph outside the selection is left alone.
    s.view.select(pos(1), pos(1));
    let outside = RulerEdit::SetIndents {
        para: 0,
        left_pt: 72.0,
        first_line_pt: 0.0,
        right_pt: 0.0,
    };
    assert!(s.view.apply_ruler_edit(&mut s.target, &outside));
    assert_eq!(
        s.view.ruler_info(&s.target).expect("info").indent_left_pt,
        36.0
    );
}
