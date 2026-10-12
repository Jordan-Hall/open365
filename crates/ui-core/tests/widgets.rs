//! Widget behaviour through the headless harness: pointer, keyboard, focus, capture, timers,
//! text editing, IME, menus, key tips and the AccessKit tree.

use std::time::Duration;

use accesskit::{Role, Toggled};
use open365_ui_core::harness::{Harness, find_access};
use open365_ui_core::tree::{MenuState, SPINNER_BUTTON_W};
use open365_ui_core::{
    Emit, ImeInput, InputEvent, Key, MenuItem, Modifiers, Node, NodeId, Rect, ScrollState, TabItem,
    TextEdit, Tip, Widget,
};

fn node(h: &mut Harness, w: Widget, r: Rect, label: &str) -> NodeId {
    let root = h.ui.tree.root();
    h.ui.add(root, Node::new(w, r).with_label(label))
}

fn harness() -> Harness {
    Harness::new(400.0, 300.0, 1.0)
}

#[test]
fn button_click_needs_press_and_release_on_the_same_node() {
    let mut h = harness();
    let a = node(
        &mut h,
        Widget::Button { toggled: None },
        Rect::new(10.0, 10.0, 80.0, 30.0),
        "A",
    );
    let b = node(
        &mut h,
        Widget::Button { toggled: None },
        Rect::new(100.0, 10.0, 80.0, 30.0),
        "B",
    );
    h.press(20.0, 20.0);
    h.release(120.0, 20.0);
    assert!(
        h.ui.take_events().is_empty(),
        "released elsewhere: no click"
    );
    h.click(20.0, 20.0);
    assert_eq!(h.ui.take_events(), vec![Emit::Clicked(a)]);
    h.click_node(b);
    assert_eq!(h.ui.take_events(), vec![Emit::Clicked(b)]);
}

#[test]
fn hover_and_press_states_follow_the_pointer_and_capture_holds() {
    let mut h = harness();
    let a = node(
        &mut h,
        Widget::Button { toggled: None },
        Rect::new(10.0, 10.0, 80.0, 30.0),
        "A",
    );
    h.move_to(20.0, 20.0);
    assert_eq!(h.ui.hover(), Some(a));
    h.press(20.0, 20.0);
    assert_eq!((h.ui.pressed(), h.ui.captured()), (Some(a), Some(a)));
    h.move_to(300.0, 200.0);
    assert_eq!(
        h.ui.captured(),
        Some(a),
        "capture survives leaving the node"
    );
    h.release(300.0, 200.0);
    assert_eq!((h.ui.pressed(), h.ui.captured()), (None, None));
    h.ui.handle(open365_ui_core::InputEvent::PointerLeft);
    assert_eq!(h.ui.hover(), None);
}

#[test]
fn toggle_checkbox_and_radio_groups() {
    let mut h = harness();
    let t = node(
        &mut h,
        Widget::Button {
            toggled: Some(false),
        },
        Rect::new(0.0, 0.0, 40.0, 30.0),
        "Bold",
    );
    let c = node(
        &mut h,
        Widget::Checkbox { checked: false },
        Rect::new(0.0, 40.0, 100.0, 20.0),
        "Gridlines",
    );
    let r1 = node(
        &mut h,
        Widget::Radio {
            checked: true,
            group: 1,
        },
        Rect::new(0.0, 70.0, 100.0, 20.0),
        "One",
    );
    let r2 = node(
        &mut h,
        Widget::Radio {
            checked: false,
            group: 1,
        },
        Rect::new(0.0, 100.0, 100.0, 20.0),
        "Two",
    );
    h.click_node(t);
    h.click_node(c);
    h.click_node(r2);
    assert_eq!(
        h.ui.take_events(),
        vec![
            Emit::Toggled(t, true),
            Emit::Toggled(c, true),
            Emit::Toggled(r2, true),
            Emit::Toggled(r1, false)
        ]
    );
}

#[test]
fn tab_order_follows_the_tree_and_wraps() {
    let mut h = harness();
    let a = node(
        &mut h,
        Widget::Button { toggled: None },
        Rect::new(0.0, 0.0, 40.0, 20.0),
        "A",
    );
    let label = node(
        &mut h,
        Widget::Label,
        Rect::new(0.0, 30.0, 40.0, 20.0),
        "Static",
    );
    let b = node(
        &mut h,
        Widget::Checkbox { checked: false },
        Rect::new(0.0, 60.0, 40.0, 20.0),
        "B",
    );
    let c = node(
        &mut h,
        Widget::TextField(TextEdit::new("")),
        Rect::new(0.0, 90.0, 40.0, 20.0),
        "C",
    );
    h.ui.tree.get_mut(b).expect("b").enabled = false;
    h.key(Key::Tab);
    assert_eq!(h.ui.focus(), Some(a));
    h.key(Key::Tab);
    assert_eq!(
        h.ui.focus(),
        Some(c),
        "labels and disabled nodes are skipped"
    );
    h.key(Key::Tab);
    assert_eq!(h.ui.focus(), Some(a), "wraps");
    h.key_with(
        Key::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(h.ui.focus(), Some(c), "shift+tab goes back");
    assert!(h.ui.tree.contains(label));
}

#[test]
fn space_and_enter_activate_the_focused_button() {
    let mut h = harness();
    let a = node(
        &mut h,
        Widget::Button { toggled: None },
        Rect::new(0.0, 0.0, 40.0, 20.0),
        "A",
    );
    h.key(Key::Tab);
    h.key(Key::Space);
    h.key(Key::Enter);
    assert_eq!(h.ui.take_events(), vec![Emit::Clicked(a), Emit::Clicked(a)]);
}

#[test]
fn access_keys_activate_with_alt() {
    let mut h = harness();
    let a = node(
        &mut h,
        Widget::Button { toggled: None },
        Rect::new(0.0, 0.0, 40.0, 20.0),
        "Save",
    );
    h.ui.tree.get_mut(a).expect("a").access_key = Some('s');
    h.key_with(
        Key::Char('S'),
        Modifiers {
            alt: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(h.ui.take_events(), vec![Emit::Clicked(a)]);
}

#[test]
fn split_button_reports_the_main_part_and_the_arrow() {
    let mut h = harness();
    let s = node(
        &mut h,
        Widget::SplitButton { arrow_w: 20.0 },
        Rect::new(0.0, 0.0, 100.0, 30.0),
        "Paste",
    );
    h.click(30.0, 15.0);
    h.click(90.0, 15.0);
    assert_eq!(
        h.ui.take_events(),
        vec![Emit::Clicked(s), Emit::MenuRequested(s)]
    );
}

#[test]
fn dropdown_opens_selects_and_closes() {
    let mut h = harness();
    let items = vec![
        "Calibri".to_owned(),
        "Segoe UI".to_owned(),
        "Arial".to_owned(),
    ];
    let d = node(
        &mut h,
        Widget::Dropdown {
            items,
            selected: 0,
            open: false,
        },
        Rect::new(10.0, 10.0, 120.0, 24.0),
        "Font",
    );
    h.click_node(d);
    assert!(matches!(
        h.ui.tree.get(d).map(|n| &n.widget),
        Some(Widget::Dropdown { open: true, .. })
    ));
    // The list hangs below the field: row 1 is the second row.
    h.click(40.0, 10.0 + 24.0 + 4.0 + 28.0 + 10.0);
    assert!(h.ui.take_events().contains(&Emit::Selected(d, 1)));
    assert!(matches!(
        h.ui.tree.get(d).map(|n| &n.widget),
        Some(Widget::Dropdown {
            open: false,
            selected: 1,
            ..
        })
    ));
    h.key(Key::Down);
    assert!(h.ui.take_events().contains(&Emit::Selected(d, 2)));
}

#[test]
fn combo_edits_text_and_picks_from_the_list() {
    let mut h = harness();
    let c = node(
        &mut h,
        Widget::Combo {
            edit: TextEdit::new("11"),
            items: vec!["10".into(), "11".into(), "12".into()],
            open: false,
        },
        Rect::new(10.0, 10.0, 80.0, 24.0),
        "Size",
    );
    h.click(20.0, 20.0);
    assert_eq!(h.ui.focus(), Some(c));
    h.key(Key::End);
    h.key(Key::Backspace);
    h.type_text("4");
    assert!(
        h.ui.take_events()
            .contains(&Emit::TextChanged(c, "14".into()))
    );
    h.key(Key::Down);
    let ev = h.ui.take_events();
    assert!(ev.contains(&Emit::Selected(c, 0)), "{ev:?}");
    h.click(80.0, 20.0);
    assert!(matches!(
        h.ui.tree.get(c).map(|n| &n.widget),
        Some(Widget::Combo { open: true, .. })
    ));
}

#[test]
fn text_field_selection_clipboard_and_editing() {
    let mut h = harness();
    let t = node(
        &mut h,
        Widget::TextField(TextEdit::new("hello world")),
        Rect::new(10.0, 10.0, 200.0, 24.0),
        "Name",
    );
    h.click_node(t);
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    h.key_with(Key::Char('A'), ctrl);
    h.key_with(Key::Char('C'), ctrl);
    h.type_text("x");
    h.key_with(Key::Char('V'), ctrl);
    let text = match h.ui.tree.get(t).map(|n| n.widget.clone()) {
        Some(Widget::TextField(e)) => e.text,
        _ => String::new(),
    };
    assert_eq!(text, "xhello world");
    h.key(Key::Home);
    h.key_with(
        Key::Right,
        Modifiers {
            shift: true,
            ctrl: true,
            ..Modifiers::default()
        },
    );
    h.key(Key::Delete);
    let events = h.ui.take_events();
    assert!(
        events.contains(&Emit::TextChanged(t, " world".into()))
            || matches!(events.last(), Some(Emit::TextChanged(_, s)) if s.starts_with(" world") || s == "world"),
        "{events:?}"
    );
    h.key(Key::Enter);
    assert!(matches!(
        h.ui.take_events().last(),
        Some(Emit::TextCommitted(..))
    ));
}

#[test]
fn dragging_in_a_text_field_selects() {
    let mut h = harness();
    let t = node(
        &mut h,
        Widget::TextField(TextEdit::new("abcdefgh")),
        Rect::new(10.0, 10.0, 200.0, 24.0),
        "Name",
    );
    h.press(15.0, 20.0);
    h.move_to(100.0, 20.0);
    h.release(100.0, 20.0);
    match h.ui.tree.get(t).map(|n| &n.widget) {
        Some(Widget::TextField(e)) => assert!(!e.selected_text().is_empty(), "drag selects"),
        _ => panic!("text field"),
    }
}

#[test]
fn caret_blinks_with_the_os_rate_and_restarts_on_typing() {
    let mut h = harness();
    let t = node(
        &mut h,
        Widget::TextField(TextEdit::new("")),
        Rect::new(10.0, 10.0, 200.0, 24.0),
        "Name",
    );
    h.click_node(t);
    let blink = h.ui.settings.caret_blink.expect("default blink");
    let visible = |h: &Harness| match h.ui.tree.get(t).map(|n| &n.widget) {
        Some(Widget::TextField(e)) => e.caret_visible(h.now(), h.ui.settings.caret_blink),
        _ => false,
    };
    assert!(visible(&h));
    assert!(h.ui.next_wake().is_some(), "a blink timer is pending");
    h.advance(blink);
    assert!(!visible(&h));
    h.type_text("a");
    assert!(visible(&h), "typing restarts the blink");
}

#[test]
fn tooltip_appears_after_the_delay_and_hides_on_leave() {
    let mut h = harness();
    let a = node(
        &mut h,
        Widget::Button { toggled: None },
        Rect::new(10.0, 10.0, 80.0, 30.0),
        "A",
    );
    h.ui.tree.get_mut(a).expect("a").tooltip = Some("Bold (Ctrl+B)".into());
    h.move_to(20.0, 20.0);
    h.advance(Duration::from_millis(300));
    assert!(h.ui.tooltip().is_none());
    h.advance(Duration::from_millis(250));
    assert_eq!(
        h.ui.tooltip().map(|t| t.text.as_str()),
        Some("Bold (Ctrl+B)")
    );
    h.move_to(300.0, 250.0);
    assert!(h.ui.tooltip().is_none());
}

#[test]
fn scroll_bar_thumb_drag_page_click_and_wheel() {
    let mut h = harness();
    let s = node(
        &mut h,
        Widget::ScrollBar(ScrollState::new(true, 900.0, 100.0)),
        Rect::new(380.0, 0.0, 16.0, 200.0),
        "Vertical",
    );
    // Thumb is 20 px long at the top. Click the track below it: one page down.
    h.click(388.0, 100.0);
    assert!(
        matches!(h.ui.take_events().last(), Some(Emit::ScrollChanged(_, v)) if (*v - 100.0).abs() < 0.01)
    );
    // Drag the thumb to the bottom.
    let (off, _) = match h.ui.tree.get(s).map(|n| n.widget.clone()) {
        Some(Widget::ScrollBar(sc)) => sc.thumb(200.0),
        _ => (0.0, 0.0),
    };
    h.press(388.0, off + 5.0);
    h.move_to(388.0, 400.0);
    h.release(388.0, 400.0);
    assert!(
        matches!(h.ui.take_events().last(), Some(Emit::ScrollChanged(_, v)) if (*v - 900.0).abs() < 0.01)
    );
    h.move_to(388.0, 50.0);
    h.ui.handle(open365_ui_core::InputEvent::Wheel { delta_y: 1.0 });
    assert!(matches!(h.ui.take_events().last(), Some(Emit::ScrollChanged(_, v)) if *v < 900.0));
}

#[test]
fn spinner_steps_with_buttons_and_arrows_within_range() {
    let mut h = harness();
    let s = node(
        &mut h,
        Widget::Spinner {
            value: 11.0,
            min: 8.0,
            max: 12.0,
            step: 1.0,
            edit: TextEdit::new("11"),
        },
        Rect::new(10.0, 10.0, 70.0, 24.0),
        "Font size",
    );
    h.click(80.0 - SPINNER_BUTTON_W / 2.0, 14.0);
    h.click(80.0 - SPINNER_BUTTON_W / 2.0, 14.0);
    assert!(
        matches!(h.ui.take_events().last(), Some(Emit::ValueChanged(_, v)) if *v == 12.0),
        "clamped at max"
    );
    h.key(Key::Down);
    assert!(
        matches!(h.ui.take_events().last(), Some(Emit::ValueChanged(id, v)) if *id == s && *v == 11.0)
    );
}

#[test]
fn tab_strip_selects_by_click_and_arrows() {
    let mut h = harness();
    let tabs = vec![
        TabItem::new("Home", 60.0),
        TabItem::new("Insert", 60.0),
        TabItem::new("View", 60.0),
    ];
    let s = node(
        &mut h,
        Widget::TabStrip {
            tabs,
            selected: 0,
            gap: 1.0,
        },
        Rect::new(0.0, 0.0, 200.0, 30.0),
        "Tabs",
    );
    h.click(70.0, 15.0);
    h.key(Key::Right);
    h.key(Key::Home);
    assert_eq!(
        h.ui.take_events(),
        vec![
            Emit::TabSelected(s, 1),
            Emit::TabSelected(s, 2),
            Emit::TabSelected(s, 0)
        ]
    );
}

fn menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem::new("new", "New"),
        MenuItem::separator(),
        MenuItem::new("export", "Export").with_submenu(vec![
            MenuItem::new("pdf", "PDF"),
            MenuItem::new("csv", "CSV"),
        ]),
        MenuItem {
            enabled: false,
            ..MenuItem::new("print", "Print")
        },
    ]
}

#[test]
fn menu_keyboard_navigation_with_submenus() {
    let mut h = harness();
    let m = node(
        &mut h,
        Widget::Menu(MenuState::new(menu_items())),
        Rect::new(20.0, 20.0, 160.0, 0.0),
        "File",
    );
    h.ui.open_menu(m, open365_ui_core::Point::new(20.0, 20.0));
    h.key(Key::Down);
    h.key(Key::Down);
    h.key(Key::Right);
    h.key(Key::Down);
    h.key(Key::Enter);
    assert_eq!(h.ui.take_events(), vec![Emit::MenuCommand(m, "csv".into())]);
    assert!(matches!(h.ui.tree.get(m).map(|n| &n.widget), Some(Widget::Menu(s)) if !s.open));
}

#[test]
fn menu_pointer_hover_opens_submenu_and_click_commands() {
    let mut h = harness();
    let m = node(
        &mut h,
        Widget::Menu(MenuState::new(menu_items())),
        Rect::new(20.0, 20.0, 160.0, 0.0),
        "File",
    );
    h.ui.open_menu(m, open365_ui_core::Point::new(20.0, 20.0));
    // Row tops: New 4, separator 32 (9 px), Export 41.
    h.move_to(40.0, 20.0 + 4.0 + 28.0 + 9.0 + 10.0);
    assert!(
        matches!(h.ui.tree.get(m).map(|n| &n.widget), Some(Widget::Menu(s)) if s.path == vec![2])
    );
    // The submenu sits to the right at the height of its parent row; its first row is PDF.
    h.click(20.0 + 160.0 + 20.0, 20.0 + 41.0 + 4.0 + 10.0);
    assert_eq!(h.ui.take_events(), vec![Emit::MenuCommand(m, "pdf".into())]);
}

#[test]
fn clicking_outside_closes_an_open_menu_and_disabled_rows_do_nothing() {
    let mut h = harness();
    let m = node(
        &mut h,
        Widget::Menu(MenuState::new(menu_items())),
        Rect::new(20.0, 20.0, 160.0, 0.0),
        "File",
    );
    h.ui.open_menu(m, open365_ui_core::Point::new(20.0, 20.0));
    h.click(40.0, 20.0 + 4.0 + 28.0 + 9.0 + 28.0 + 10.0);
    assert!(h.ui.take_events().is_empty(), "disabled Print");
    h.click(350.0, 250.0);
    assert!(matches!(h.ui.tree.get(m).map(|n| &n.widget), Some(Widget::Menu(s)) if !s.open));
}

#[test]
fn ime_composition_shows_inline_and_commits() {
    let mut h = harness();
    let t = node(
        &mut h,
        Widget::TextField(TextEdit::new("ab")),
        Rect::new(10.0, 10.0, 200.0, 24.0),
        "Name",
    );
    h.click_node(t);
    assert!(h.ui.wants_ime());
    assert!(h.ui.ime_cursor_area().is_some());
    h.ime(ImeInput::Preedit {
        text: "\u{306b}\u{307b}".into(),
        cursor: Some((0, 3)),
    });
    match h.ui.tree.get(t).map(|n| &n.widget) {
        Some(Widget::TextField(e)) => {
            assert_eq!(
                e.composition.as_ref().map(|c| c.text.as_str()),
                Some("\u{306b}\u{307b}")
            );
            assert_eq!(e.text, "ab", "preedit is not committed text");
        }
        _ => panic!("text field"),
    }
    h.ime(ImeInput::Commit("\u{65e5}\u{672c}".into()));
    assert!(
        h.ui.take_events()
            .contains(&Emit::TextChanged(t, "ab\u{65e5}\u{672c}".into()))
    );
    h.ime(ImeInput::Disabled);
    assert!(h.ui.tree.get(t).is_some());
}

#[test]
fn alt_key_tips_walk_tab_then_command_level_and_escape_goes_up() {
    let mut h = Harness::new(600.0, 100.0, 1.0);
    let strip = node(
        &mut h,
        Widget::TabStrip {
            tabs: vec![
                TabItem {
                    key_tip: Some("H".into()),
                    ..TabItem::new("Home", 60.0)
                },
                TabItem {
                    key_tip: Some("N".into()),
                    ..TabItem::new("Insert", 60.0)
                },
            ],
            selected: 1,
            gap: 1.0,
        },
        Rect::new(50.0, 0.0, 200.0, 30.0),
        "Tabs",
    );
    let file = node(
        &mut h,
        Widget::Button { toggled: None },
        Rect::new(0.0, 0.0, 48.0, 30.0),
        "File",
    );
    h.ui.tree.get_mut(file).expect("file").key_tip = Some("F".into());
    h.ui.set_tip_children(
        "tab:0",
        vec![
            Tip {
                keys: "V".into(),
                target: "cmd.paste".into(),
            },
            Tip {
                keys: "FP".into(),
                target: "cmd.painter".into(),
            },
        ],
    );
    h.alt_tap();
    assert!(h.ui.keytips.is_showing());
    assert_eq!(h.ui.visible_tips().len(), 3, "H, N and F");
    // H descends to the Home tab: it is selected and its command tips show.
    h.key(Key::Char('H'));
    assert!(h.ui.take_events().contains(&Emit::TabSelected(strip, 0)));
    assert_eq!(h.ui.keytips.depth(), 1);
    let keys: Vec<String> = h.ui.keytips.visible().map(|t| t.keys.clone()).collect();
    assert_eq!(keys, ["V", "FP"]);
    // The command tips resolve to nodes registered for them and run those nodes.
    let paste = node(
        &mut h,
        Widget::Button { toggled: None },
        Rect::new(10.0, 40.0, 40.0, 40.0),
        "Paste",
    );
    h.ui.set_tip_nodes([("cmd.paste".to_string(), paste)].into());
    assert_eq!(h.ui.visible_tips().len(), 1, "only V has a node to badge");
    h.ui.take_events();
    h.key(Key::Char('V'));
    let ev = h.ui.take_events();
    assert!(ev.contains(&Emit::Clicked(paste)) && ev.contains(&Emit::KeyTip(paste)));
    h.alt_tap();
    h.key(Key::Char('H'));
    h.ui.take_events();
    h.key(Key::Escape);
    assert_eq!(
        (h.ui.keytips.depth(), h.ui.visible_tips().len()),
        (0, 3),
        "back to the tab tips"
    );
    h.key(Key::Escape);
    assert!(!h.ui.keytips.is_showing());
    // F activates the File button.
    h.alt_tap();
    h.key(Key::Char('F'));
    assert!(h.ui.take_events().contains(&Emit::KeyTip(file)));
}

#[test]
fn accesskit_tree_reflects_roles_names_and_state() {
    let mut h = harness();
    let c = node(
        &mut h,
        Widget::Checkbox { checked: true },
        Rect::new(0.0, 0.0, 100.0, 20.0),
        "Gridlines",
    );
    let b = node(
        &mut h,
        Widget::Button {
            toggled: Some(false),
        },
        Rect::new(0.0, 30.0, 50.0, 20.0),
        "Bold",
    );
    let t = node(
        &mut h,
        Widget::TextField(TextEdit::new("hi")),
        Rect::new(0.0, 60.0, 100.0, 20.0),
        "Name",
    );
    let _tabs = node(
        &mut h,
        Widget::TabStrip {
            tabs: vec![TabItem::new("Home", 50.0)],
            selected: 0,
            gap: 0.0,
        },
        Rect::new(0.0, 90.0, 60.0, 20.0),
        "Tabs",
    );
    h.ui.tree.get_mut(b).expect("b").enabled = false;
    h.ui.set_focus(Some(c));
    let u = h.access();
    let (_, cb) = find_access(&u, Role::CheckBox, "Gridlines").expect("checkbox");
    assert_eq!(cb.toggled(), Some(Toggled::True));
    let (_, tb) = find_access(&u, Role::Button, "Bold").expect("toggle");
    assert!(tb.is_disabled());
    assert_eq!(tb.toggled(), Some(Toggled::False));
    let (_, tf) = find_access(&u, Role::TextInput, "Name").expect("text input");
    assert_eq!(tf.value(), Some("hi"));
    let (_, tab) = find_access(&u, Role::Tab, "Home").expect("tab");
    assert_eq!(tab.is_selected(), Some(true));
    assert_eq!(u.focus, open365_ui_core::access::ak_id(c));
    let root = u
        .nodes
        .iter()
        .find(|(id, _)| *id == open365_ui_core::access::WINDOW_ID)
        .expect("window");
    assert_eq!(root.1.role(), Role::Window);
    assert!(
        root.1
            .children()
            .contains(&open365_ui_core::access::ak_id(t))
    );
    assert!(u.tree.is_some());
}

#[test]
fn removing_a_node_while_hovered_does_not_leave_dangling_state() {
    let mut h = harness();
    let a = node(
        &mut h,
        Widget::Button { toggled: None },
        Rect::new(10.0, 10.0, 80.0, 30.0),
        "A",
    );
    h.move_to(20.0, 20.0);
    h.press(20.0, 20.0);
    h.ui.tree.remove(a);
    h.release(20.0, 20.0);
    assert!(h.ui.take_events().is_empty());
    h.move_to(30.0, 30.0);
    assert_eq!(h.ui.hover(), None);
}

#[test]
fn a_focused_canvas_gets_alt_chords() {
    let mut h = harness();
    let c = node(
        &mut h,
        Widget::Canvas {
            caret: Rect::new(0.0, 0.0, 1.0, 1.0),
        },
        Rect::new(0.0, 0.0, 400.0, 300.0),
        "Grid",
    );
    h.ui.set_focus(Some(c));
    h.ui.take_events();
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    // Alt+= is AutoSum; Alt+Down opens the validation list.
    h.key_with(Key::Char('='), alt);
    assert!(
        h.ui.take_events()
            .contains(&Emit::KeyPressed(c, Key::Char('='), alt))
    );
    h.key_with(Key::Down, alt);
    assert!(
        h.ui.take_events()
            .contains(&Emit::KeyPressed(c, Key::Down, alt))
    );
    // A plain letter still arrives as text, not as a key.
    h.key(Key::Char('a'));
    assert!(
        !h.ui
            .take_events()
            .iter()
            .any(|e| matches!(e, Emit::KeyPressed(_, Key::Char('a'), _)))
    );
}

fn blink_field() -> (Harness, NodeId, Duration) {
    let mut h = harness();
    let t = node(
        &mut h,
        Widget::TextField(TextEdit::new("")),
        Rect::new(10.0, 10.0, 200.0, 24.0),
        "Name",
    );
    h.click_node(t);
    let blink = h.ui.settings.caret_blink.expect("default blink");
    (h, t, blink)
}

fn field_visible(h: &Harness, t: NodeId) -> bool {
    match h.ui.tree.get(t).map(|n| &n.widget) {
        Some(Widget::TextField(e)) => e.caret_visible(h.now(), h.ui.settings.caret_blink),
        _ => false,
    }
}

#[test]
fn blink_wakes_only_at_the_next_edge_and_a_keystroke_moves_it() {
    let (mut h, t, blink) = blink_field();
    let t0 = h.now();
    assert!(field_visible(&h, t));
    assert_eq!(
        h.ui.next_wake(),
        Some(t0 + blink),
        "first edge: caret hides"
    );
    h.advance(blink);
    assert!(!field_visible(&h, t));
    assert!(h.ui.take_redraw() || h.ui.next_wake().is_some());
    assert_eq!(h.ui.next_wake(), Some(t0 + blink * 2), "next edge only");
    h.advance(blink / 2);
    h.type_text("a");
    assert!(field_visible(&h, t), "visible right after a keystroke");
    assert_eq!(
        h.ui.next_wake(),
        Some(h.now() + blink),
        "the keystroke restarted the cycle"
    );
}

#[test]
fn no_blink_timer_without_window_focus_and_caret_returns_solid() {
    let (mut h, t, blink) = blink_field();
    h.advance(blink);
    h.ui.handle(InputEvent::FocusChanged(false));
    assert_eq!(h.ui.next_wake(), None, "an inactive window does not wake");
    h.advance(blink * 5);
    h.ui.handle(InputEvent::FocusChanged(true));
    assert!(field_visible(&h, t), "solid when focus returns");
    assert_eq!(h.ui.next_wake(), Some(h.now() + blink));
}

#[test]
fn a_steady_os_caret_never_wakes_the_loop() {
    let mut h = harness();
    let mut s = h.ui.settings.clone();
    s.caret_blink = None;
    h.ui.set_settings(s);
    let t = node(
        &mut h,
        Widget::TextField(TextEdit::new("")),
        Rect::new(10.0, 10.0, 200.0, 24.0),
        "Name",
    );
    h.click_node(t);
    h.type_text("a");
    assert_eq!(h.ui.next_wake(), None);
    h.advance(Duration::from_secs(3));
    assert!(field_visible(&h, t));
}

#[test]
fn a_canvas_caret_follows_the_blink_clock_and_its_epoch() {
    let mut h = harness();
    let c = node(
        &mut h,
        Widget::Canvas {
            caret: Rect::new(0.0, 0.0, 1.0, 16.0),
        },
        Rect::new(0.0, 50.0, 300.0, 200.0),
        "Page",
    );
    h.click_node(c);
    let blink = h.ui.settings.caret_blink.expect("default blink");
    let t0 = h.now();
    assert!(h.ui.canvas_caret_visible(t0, t0), "visible at t=0");
    assert_eq!(h.ui.next_wake(), Some(t0 + blink));
    h.advance(blink);
    assert!(
        !h.ui.canvas_caret_visible(t0, h.now()),
        "hidden after one interval"
    );
    h.advance(blink / 2);
    let epoch = h.now();
    assert!(
        h.ui.canvas_caret_visible(epoch, h.now()),
        "visible after a move"
    );
    assert_eq!(h.ui.next_wake(), Some(epoch + blink));
    h.ui.handle(InputEvent::FocusChanged(false));
    assert!(
        !h.ui.canvas_caret_visible(epoch, h.now()),
        "no caret without focus"
    );
    assert_eq!(h.ui.next_wake(), None);
}

#[test]
fn a_canvas_that_draws_no_caret_never_blinks() {
    let mut h = harness();
    let c = node(
        &mut h,
        Widget::Canvas {
            caret: Rect::new(0.0, 0.0, 1.0, 16.0),
        },
        Rect::new(0.0, 50.0, 300.0, 200.0),
        "Grid",
    );
    h.click_node(c);
    assert_eq!(
        h.ui.next_wake(),
        None,
        "Sheets outside edit mode has no caret"
    );
}

#[test]
fn a_masked_field_keeps_its_text_from_events_access_and_the_clipboard() {
    let mut h = harness();
    let f = node(
        &mut h,
        Widget::TextField(TextEdit::masked("")),
        Rect::new(10.0, 10.0, 200.0, 24.0),
        "Password",
    );
    h.ui.set_focus(Some(f));
    h.type_text("secret");
    let events = h.ui.take_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Emit::TextChanged(n, t) if *n == f && t.is_empty())),
        "{events:?}"
    );
    assert!(
        !events.iter().any(|e| format!("{e:?}").contains("secret")),
        "{events:?}"
    );
    let u = h.access();
    let (_, node) = find_access(&u, Role::PasswordInput, "Password").expect("password input");
    assert_eq!(node.value(), None);
    // Select all and cut: a password is neither copied nor cut.
    h.key_with(
        Key::Char('A'),
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    );
    h.key_with(
        Key::Char('X'),
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    );
    let Widget::TextField(e) = &h.ui.tree.get(f).expect("field").widget else {
        unreachable!()
    };
    assert_eq!(e.text, "secret");
    // It paints a bullet per character.
    assert_eq!(e.display().text, "\u{2022}".repeat(6));
}
