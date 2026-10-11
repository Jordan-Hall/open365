//! The bridge feeding a real `Ui`: a bare Alt press and release shows the key tips, and the
//! auto-repeat of a held Alt does not restart them.

use open365_ui_core::geom::Rect;
use open365_ui_core::harness::Harness;
use open365_ui_core::{InputEvent, Key, Modifiers, Node, TabItem, Widget};
use open365_ui_host::InputBridge;
use open365_ui_host::winit::event::WindowEvent;
use open365_ui_host::winit::keyboard::ModifiersState;

fn feed(h: &mut Harness, events: Vec<InputEvent>) {
    for e in events {
        h.ui.handle(e);
    }
}

fn harness_with_tabs() -> Harness {
    let mut h = Harness::new(300.0, 100.0, 1.0);
    let root = h.ui.tree.root();
    let tabs = vec![TabItem {
        key_tip: Some("H".into()),
        ..TabItem::new("Home", 60.0)
    }];
    let strip = Node::new(
        Widget::TabStrip {
            tabs,
            selected: 0,
            gap: 1.0,
        },
        Rect::new(0.0, 0.0, 100.0, 30.0),
    );
    h.ui.add(root, strip);
    h
}

#[test]
fn a_bare_alt_tap_through_the_bridge_shows_tips_and_a_held_alt_does_not_toggle_them() {
    let mut h = harness_with_tabs();
    let mut b = InputBridge::new(1.0);
    let mut out = Vec::new();
    // Alt goes down: winit reports the modifier change first, then the key (and its repeats).
    let mut ev = Vec::new();
    b.translate(
        &WindowEvent::ModifiersChanged(ModifiersState::ALT.into()),
        &mut ev,
    );
    b.key_event(Some(Key::Alt), true, false, None, &mut out);
    for _ in 0..5 {
        b.key_event(Some(Key::Alt), true, true, None, &mut out);
    }
    feed(&mut h, out);
    assert!(
        !h.ui.keytips.is_showing(),
        "nothing shows while Alt is held"
    );
    // Release.
    let mut out = Vec::new();
    b.translate(
        &WindowEvent::ModifiersChanged(ModifiersState::empty().into()),
        &mut ev,
    );
    b.key_event(Some(Key::Alt), false, false, None, &mut out);
    feed(&mut h, out);
    assert!(
        h.ui.keytips.is_showing(),
        "Alt press and release shows the tips"
    );
    assert_eq!(b.modifiers(), Modifiers::default());
}

#[test]
fn alt_used_as_a_modifier_never_shows_the_tips() {
    let mut h = harness_with_tabs();
    let mut b = InputBridge::new(1.0);
    let mut ev = Vec::new();
    b.translate(
        &WindowEvent::ModifiersChanged(ModifiersState::ALT.into()),
        &mut ev,
    );
    let mut out = Vec::new();
    b.key_event(Some(Key::Alt), true, false, None, &mut out);
    b.key_event(Some(Key::Char('S')), true, false, Some("s"), &mut out);
    b.key_event(Some(Key::Char('S')), false, false, None, &mut out);
    b.key_event(Some(Key::Alt), false, false, None, &mut out);
    feed(&mut h, out);
    assert!(!h.ui.keytips.is_showing());
}
