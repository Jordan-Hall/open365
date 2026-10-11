//! The document caret blinks like a Windows caret: shown at the start of a cycle, hidden for one blink time, restarted by
//! input, hidden while the window is inactive, and the document has the keyboard at start-up. Input time is set by the
//! tests, so no wall clock is involved.

use std::path::PathBuf;

use egui::{Key, vec2};
use egui_kittest::Harness;
use open365_commands::Target;
use open365_ui_egui::{DocView, blink_phase};

/// Half of the blink cycle used by these tests, seconds.
const HALF: f64 = 0.5;

struct State {
    view: DocView,
    target: Target,
}

fn harness() -> Harness<'static, State> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/docx/plain-paragraphs.docx");
    let bytes = std::fs::read(path).expect("fixture");
    let mut view = DocView::new();
    view.set_caret_style(1.0, Some(HALF));
    Harness::builder()
        .with_size(vec2(900.0, 600.0))
        .build_ui_state(
            |ui, s: &mut State| {
                s.view.show(ui, &mut s.target);
            },
            State {
                view,
                target: Target::open_docx(&bytes).expect("open"),
            },
        )
}

/// One frame at input time `t` seconds.
fn tick(h: &mut Harness<'_, State>, t: f64) {
    h.input_mut().time = Some(t);
    h.step();
}

/// The first frames at t = 0: the document asks for focus, gets it, and the caret cycle starts.
fn started() -> Harness<'static, State> {
    let mut h = harness();
    for _ in 0..10 {
        tick(&mut h, 0.0);
    }
    h
}

#[test]
fn the_document_has_the_keyboard_at_startup() {
    let h = started();
    assert!(h.state().view.is_focused());
}

#[test]
fn caret_is_visible_at_time_zero() {
    let h = started();
    assert!(h.state().view.caret_shown());
}

#[test]
fn caret_is_hidden_after_one_blink_time_and_back_after_two() {
    let mut h = started();
    tick(&mut h, 0.4);
    assert!(h.state().view.caret_shown(), "still in the first half");
    tick(&mut h, HALF + 0.1);
    assert!(!h.state().view.caret_shown(), "hidden at the blink time");
    tick(&mut h, 2.0 * HALF + 0.1);
    assert!(
        h.state().view.caret_shown(),
        "shown again after a full cycle"
    );
}

#[test]
fn a_keystroke_restarts_the_cycle() {
    let mut h = started();
    tick(&mut h, 0.6);
    assert!(!h.state().view.caret_shown());
    // F13 moves nothing: only the keystroke restarts the cycle.
    h.input_mut().time = Some(0.6);
    h.key_press(Key::F13);
    h.step();
    assert!(h.state().view.caret_shown(), "shown right after the key");
    tick(&mut h, 0.9);
    assert!(
        h.state().view.caret_shown(),
        "0.3 s after the key; the old cycle would hide it at 0.9 s"
    );
    tick(&mut h, 1.2);
    assert!(
        !h.state().view.caret_shown(),
        "hidden one blink time after the key"
    );
}

#[test]
fn caret_is_hidden_while_the_window_is_inactive() {
    let mut h = started();
    h.input_mut().focused = false;
    tick(&mut h, 0.1);
    assert!(!h.state().view.caret_shown());
    h.input_mut().focused = true;
    tick(&mut h, 0.2);
    assert!(h.state().view.caret_shown());
}

#[test]
fn a_caret_that_does_not_blink_stays_visible() {
    let mut h = started();
    h.state_mut().view.set_caret_style(1.0, None);
    tick(&mut h, 5.3);
    assert!(h.state().view.caret_shown());
    assert_eq!(blink_phase(5.3, None), (true, None));
}
