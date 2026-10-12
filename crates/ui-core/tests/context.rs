//! Pointer buttons, the context menu popup and the animation frame source.

use std::time::Duration;

use open365_ui_core::geom::Point;
use open365_ui_core::harness::Harness;
use open365_ui_core::{
    Emit, FRAME_INTERVAL, InputEvent, Key, MenuItem, Modifiers, Node, PointerButton, Rect, Widget,
};

fn canvas(h: &mut Harness) -> open365_ui_core::NodeId {
    let root = h.ui.tree.root();
    let id = h.ui.add(
        root,
        Node::new(
            Widget::Canvas {
                caret: Rect::new(0.0, 0.0, 10.0, 10.0),
            },
            Rect::new(0.0, 0.0, 400.0, 300.0),
        )
        .with_label("canvas"),
    );
    h.ui.set_focus(Some(id));
    id
}

#[test]
fn secondary_and_middle_presses_carry_the_button() {
    let mut h = Harness::new(400.0, 300.0, 1.0);
    let c = canvas(&mut h);
    h.ui.take_events();
    h.right_click(50.0, 60.0);
    h.click_with(PointerButton::Middle, 70.0, 80.0);
    let ev = h.ui.take_events();
    assert_eq!(
        ev,
        vec![
            Emit::CanvasButton {
                node: c,
                at: Point::new(50.0, 60.0),
                button: PointerButton::Secondary,
                clicks: 1,
                modifiers: Modifiers::default(),
            },
            Emit::CanvasButton {
                node: c,
                at: Point::new(70.0, 80.0),
                button: PointerButton::Middle,
                clicks: 1,
                modifiers: Modifiers::default(),
            },
        ]
    );
}

#[test]
fn drag_and_release_carry_the_modifiers() {
    let mut h = Harness::new(400.0, 300.0, 1.0);
    let c = canvas(&mut h);
    h.ui.take_events();
    h.press(10.0, 10.0);
    h.ui.handle(InputEvent::Key {
        key: Key::Ctrl,
        pressed: true,
        modifiers: Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    });
    h.move_to(30.0, 30.0);
    h.release(30.0, 30.0);
    let ev = h.ui.take_events();
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    assert!(ev.contains(&Emit::CanvasDrag {
        node: c,
        at: Point::new(30.0, 30.0),
        button: PointerButton::Primary,
        modifiers: ctrl,
    }));
    assert!(ev.contains(&Emit::CanvasRelease {
        node: c,
        at: Point::new(30.0, 30.0),
        button: PointerButton::Primary,
        modifiers: ctrl,
    }));
}

#[test]
fn the_context_menu_navigates_by_keyboard_and_flips_inside_the_window() {
    let mut h = Harness::new(400.0, 300.0, 1.0);
    let _ = canvas(&mut h);
    let items = vec![
        MenuItem::new("a", "Alpha"),
        MenuItem::separator(),
        MenuItem::new("b", "Beta"),
    ];
    let id = h.ui.open_context_menu(items, Point::new(390.0, 295.0));
    let r = h.ui.tree.abs_rect(id);
    assert!(r.right() <= 400.0 && r.bottom() <= 300.0, "{r:?}");
    h.ui.take_events();
    h.key(Key::Down);
    h.key(Key::Down);
    h.key(Key::Enter);
    assert!(
        h.ui.take_events()
            .contains(&Emit::MenuCommand(id, "b".into()))
    );
    // Closed after the pick: a press where it was reaches the canvas again.
    h.click(r.x + 5.0, r.y + 5.0);
    assert!(
        h.ui.take_events()
            .iter()
            .any(|e| matches!(e, Emit::CanvasPointer { .. }))
    );
}

#[test]
fn the_frame_source_ticks_only_while_animating() {
    let mut h = Harness::new(400.0, 300.0, 1.0);
    assert_eq!(h.ui.next_wake(), None);
    h.ui.set_animating(true);
    assert!(h.ui.next_wake().is_some());
    let f0 = h.ui.anim_frame();
    h.advance(FRAME_INTERVAL);
    h.advance(FRAME_INTERVAL);
    assert_eq!(h.ui.anim_frame(), f0 + 2);
    assert!(h.ui.take_redraw());
    h.ui.set_animating(false);
    assert_eq!(h.ui.next_wake(), None);
    h.advance(Duration::from_secs(1));
    assert_eq!(h.ui.anim_frame(), f0 + 2);
}
