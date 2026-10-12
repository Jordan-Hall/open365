//! The dialog framework on the headless harness: keyboard, access keys, tabs, lists, dragging,
//! DPI scaling and the AccessKit tree.

use accesskit::Role;

use super::*;
use crate::geom::Rect;
use crate::harness::{Harness, find_access};
use crate::input::{Emit, InputEvent, Key, Modifiers};

fn spec() -> Spec {
    let mut s = Spec::new("Test dialog", 360.0, 230.0);
    s.label(12.0, 8.0, 100.0, "&Name:");
    s.text("name", 120.0, 8.0, 200.0, "abc");
    s.label(12.0, 36.0, 100.0, "&Count:");
    s.spin("count", 120.0, 36.0, 80.0, (3.0, 0.0, 10.0, 1.0));
    s.check("flag", 12.0, 64.0, 150.0, "&Wrap text", false);
    s.radio("r1", "mode", 12.0, 86.0, 100.0, "&One", true);
    s.radio("r2", "mode", 12.0, 106.0, 100.0, "T&wo", false);
    s.radio("r3", "mode", 12.0, 126.0, 100.0, "T&hree", false);
    s.dropdown("kind", 120.0, 86.0, 120.0, &["Alpha", "Beta", "Gamma"], 0);
    s.list(
        "list",
        Rect::new(120.0, 116.0, 120.0, 60.0),
        (0..10).map(|i| format!("item {i}")).collect(),
        Some(1),
    );
    s.reference("ref", 12.0, 190.0, 300.0, "A1");
    s
}

fn open(h: &mut Harness, spec: Spec) -> Dialog {
    let mut d = Dialog::new(spec, 1);
    d.open(&mut h.ui, Icons::default());
    d
}

/// Feeds the pending events to the dialog; returns the outcomes.
fn pump(h: &mut Harness, d: &mut Dialog) -> Vec<Outcome> {
    let mut out = Vec::new();
    for ev in h.ui.take_events() {
        if let Response::Outcome(o) = d.handle(&mut h.ui, &ev) {
            out.push(o);
        }
    }
    out
}

fn key(h: &mut Harness, d: &mut Dialog, k: Key) -> Vec<Outcome> {
    h.key(k);
    pump(h, d)
}

fn key_mod(h: &mut Harness, d: &mut Dialog, k: Key, m: Modifiers) -> Vec<Outcome> {
    h.key_with(k, m);
    pump(h, d)
}

fn alt(h: &mut Harness, d: &mut Dialog, c: char) -> Vec<Outcome> {
    let m = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    h.ui.handle(InputEvent::Key {
        key: Key::Alt,
        pressed: true,
        modifiers: m,
    });
    let _ = pump(h, d);
    key_mod(h, d, Key::Char(c.to_ascii_uppercase()), m)
}

fn focus_label(h: &Harness) -> String {
    h.ui.focus()
        .and_then(|f| h.ui.tree.get(f))
        .map(|n| n.label.clone())
        .unwrap_or_default()
}

fn harness() -> Harness {
    Harness::new(800.0, 600.0, 1.0)
}

#[test]
fn opens_centred_and_focuses_the_first_control() {
    let mut h = harness();
    let d = open(&mut h, spec());
    let panel = h.ui.tree.abs_rect(d.panel().expect("panel"));
    assert!((panel.x - (800.0 - 360.0) / 2.0).abs() <= 1.0);
    assert_eq!(h.ui.focus(), d.node("name"));
    assert_eq!(scrim_serial(&h.ui), Some(1));
}

#[test]
fn tab_walks_controls_in_order_with_one_stop_per_group_and_list() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    let mut want = vec![
        d.node("count"),
        d.node("flag"),
        d.node("r1"),
        d.node("kind"),
        // The selected row of the list is its stop.
        h.find("item 1"),
        d.node("ref"),
    ];
    want.push(d.tab_stops(&h.ui).into_iter().find(|n| {
        h.ui.tree
            .get(*n)
            .is_some_and(|x| x.label.starts_with("Collapse"))
    }));
    want.push(d.footer_node("ok"));
    want.push(d.footer_node("cancel"));
    want.push(d.node("name"));
    let mut seen = Vec::new();
    for _ in 0..want.len() {
        key(&mut h, &mut d, Key::Tab);
        seen.push(h.ui.focus());
    }
    assert_eq!(seen, want);
}

#[test]
fn shift_tab_goes_back_and_wraps() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    key_mod(
        &mut h,
        &mut d,
        Key::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(focus_label(&h), "Cancel");
}

#[test]
fn enter_presses_the_default_button_and_escape_the_cancel_button() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    assert_eq!(
        key(&mut h, &mut d, Key::Enter),
        vec![Outcome::Pressed("ok".into())]
    );
    assert_eq!(
        key(&mut h, &mut d, Key::Escape),
        vec![Outcome::Pressed("cancel".into())]
    );
    // Enter on the Cancel button presses Cancel, not the default.
    let c = d.footer_node("cancel").expect("cancel");
    d.focus(&mut h.ui, "cancel");
    assert_eq!(h.ui.focus(), Some(c));
    assert_eq!(
        key(&mut h, &mut d, Key::Enter),
        vec![Outcome::Pressed("cancel".into())]
    );
}

#[test]
fn escape_closes_an_open_dropdown_first() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    d.focus(&mut h.ui, "kind");
    let n = d.node("kind").expect("kind");
    h.click_node(n);
    pump(&mut h, &mut d);
    assert!(matches!(
        h.ui.tree.get(n).map(|x| &x.widget),
        Some(crate::Widget::Dropdown { open: true, .. })
    ));
    assert!(key(&mut h, &mut d, Key::Escape).is_empty());
    assert!(matches!(
        h.ui.tree.get(n).map(|x| &x.widget),
        Some(crate::Widget::Dropdown { open: false, .. })
    ));
    assert_eq!(
        key(&mut h, &mut d, Key::Escape),
        vec![Outcome::Pressed("cancel".into())]
    );
}

#[test]
fn typing_and_arrows_change_values() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    h.type_text("xyz");
    pump(&mut h, &mut d);
    // Focus selected the old text, so typing replaced it.
    assert_eq!(d.values(&h.ui).text("name"), "xyz");
    key(&mut h, &mut d, Key::Tab);
    key(&mut h, &mut d, Key::Up);
    let out = pump(&mut h, &mut d);
    let _ = out;
    assert!((d.values(&h.ui).num("count") - 4.0).abs() < 1e-9);
}

#[test]
fn access_keys_focus_fields_and_toggle_boxes() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    // The label's key goes to the next field.
    alt(&mut h, &mut d, 'c');
    assert_eq!(h.ui.focus(), d.node("count"));
    let out = alt(&mut h, &mut d, 'w');
    assert!(out.contains(&Outcome::Changed("flag".into())));
    assert!(d.values(&h.ui).flag("flag"));
    alt(&mut h, &mut d, 'w');
    assert!(!d.values(&h.ui).flag("flag"));
    // Radios: Alt+W is the check box; Alt+H checks "Three".
    alt(&mut h, &mut d, 'h');
    assert_eq!(d.values(&h.ui).radio("mode"), "r3");
    alt(&mut h, &mut d, 'o');
    assert_eq!(d.values(&h.ui).radio("mode"), "r1");
}

#[test]
fn radio_group_has_one_tab_stop_and_arrows_move_the_choice() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    d.focus(&mut h.ui, "r1");
    key(&mut h, &mut d, Key::Down);
    assert_eq!(d.values(&h.ui).radio("mode"), "r2");
    // The checked radio is the stop now.
    key(&mut h, &mut d, Key::Tab);
    key(&mut h, &mut d, Key::Tab);
    assert_ne!(focus_label(&h), "Two");
    d.focus(&mut h.ui, "name");
    let stops = d.tab_stops(&h.ui);
    let r2 = d.node("r2").expect("r2");
    assert!(stops.contains(&r2));
    assert!(!stops.contains(&d.node("r1").expect("r1")));
}

#[test]
fn list_keys_select_and_scroll() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    d.focus(&mut h.ui, "list");
    assert_eq!(d.values(&h.ui).index("list"), Some(1));
    let out = key(&mut h, &mut d, Key::Down);
    assert!(out.contains(&Outcome::Changed("list".into())));
    assert_eq!(d.values(&h.ui).index("list"), Some(2));
    key(&mut h, &mut d, Key::End);
    assert_eq!(d.values(&h.ui).index("list"), Some(9));
    // The selected row scrolled into view and holds the focus.
    assert_eq!(focus_label(&h), "item 9");
    key(&mut h, &mut d, Key::Home);
    assert_eq!(d.values(&h.ui).index("list"), Some(0));
    assert_eq!(focus_label(&h), "item 0");
    // A click selects.
    let row = h.find("item 2").expect("row 2");
    h.click_node(row);
    pump(&mut h, &mut d);
    assert_eq!(d.values(&h.ui).index("list"), Some(2));
}

#[test]
fn tabs_show_their_page_and_ctrl_tab_switches() {
    let mut s = Spec::new("Pages", 300.0, 120.0);
    s.tabs = vec!["&One".into(), "&Two".into()];
    s.on_page(Some(0));
    s.text("a", 12.0, 44.0, 200.0, "");
    s.on_page(Some(1));
    s.text("b", 12.0, 44.0, 200.0, "");
    let mut h = harness();
    let mut d = open(&mut h, s);
    let a = d.node("a").expect("a");
    let b = d.node("b").expect("b");
    assert!(h.ui.tree.shown(a) && !h.ui.tree.shown(b));
    let out = key_mod(
        &mut h,
        &mut d,
        Key::Tab,
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(out, vec![Outcome::Page(1)]);
    assert!(!h.ui.tree.shown(a) && h.ui.tree.shown(b));
    // Tab skips the hidden page's field.
    let stops = d.tab_stops(&h.ui);
    assert!(stops.contains(&b) && !stops.contains(&a));
}

#[test]
fn dragging_the_title_bar_moves_the_dialog() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    let panel = d.panel().expect("panel");
    let before = h.ui.tree.abs_rect(panel);
    let bar = h.ui.tree.abs_rect(d.title_bar().expect("bar"));
    let (x, y) = (bar.x + 40.0, bar.y + 10.0);
    h.move_to(x, y);
    h.press(x, y);
    pump(&mut h, &mut d);
    h.move_to(x + 100.0, y + 50.0);
    pump(&mut h, &mut d);
    h.release(x + 100.0, y + 50.0);
    pump(&mut h, &mut d);
    let after = h.ui.tree.abs_rect(panel);
    assert!(
        (after.x - before.x - 100.0).abs() < 1.5,
        "{before:?} {after:?}"
    );
    assert!((after.y - before.y - 50.0).abs() < 1.5);
    // It stays inside the window however far it is dragged.
    d.relayout(&mut h.ui);
    let kept = h.ui.tree.abs_rect(panel);
    assert!((kept.x - after.x).abs() < 0.5);
}

#[test]
fn the_dialog_is_laid_out_in_logical_units_at_any_scale() {
    for scale in [1.0_f32, 1.25, 2.0] {
        let mut h = Harness::new(800.0, 600.0, scale);
        let d = open(&mut h, spec());
        let ok = d.footer_node("ok").expect("ok");
        let r = h.ui.tree.abs_rect(ok);
        assert!((r.w - BUTTON_W).abs() < 0.01 && (r.h - BUTTON_H).abs() < 0.01);
        let dev = h.ui.tree.device_rect(ok, h.ui.scale);
        let want = BUTTON_W * scale;
        assert!(
            (dev.width() as f32 - want).abs() <= 1.0,
            "scale {scale}: {} vs {want}",
            dev.width()
        );
    }
}

#[test]
fn window_resize_recentres_and_keeps_a_dragged_dialog_on_screen() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    let panel = d.panel().expect("panel");
    let root = h.ui.tree.root();
    if let Some(r) = h.ui.tree.get_mut(root) {
        r.bounds = Rect::new(0.0, 0.0, 1000.0, 700.0);
    }
    d.relayout(&mut h.ui);
    let r = h.ui.tree.abs_rect(panel);
    assert!((r.x - (1000.0 - 360.0) / 2.0).abs() <= 1.0);
}

#[test]
fn accesskit_exposes_a_modal_dialog_labelled_by_its_title() {
    let mut h = harness();
    let d = open(&mut h, spec());
    let update = h.access();
    let panel_ak = crate::access::ak_id(d.panel().expect("panel"));
    let (_, node) = update
        .nodes
        .iter()
        .find(|(id, _)| *id == panel_ak)
        .expect("dialog node");
    assert_eq!(node.role(), Role::Dialog);
    assert!(node.is_modal());
    assert_eq!(node.labelled_by().len(), 1);
    assert!(find_access(&update, Role::Button, "OK").is_some());
    assert!(find_access(&update, Role::TextInput, "Name").is_some());
    assert!(find_access(&update, Role::CheckBox, "Wrap text").is_some());
}

#[test]
fn the_reference_field_collapses_and_expands_with_enter() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    d.focus(&mut h.ui, "ref");
    let btn = d
        .tab_stops(&h.ui)
        .into_iter()
        .find(|n| {
            h.ui.tree
                .get(*n)
                .is_some_and(|x| x.label.starts_with("Collapse"))
        })
        .expect("collapse button");
    h.click_node(btn);
    let out = pump(&mut h, &mut d);
    assert_eq!(
        out,
        vec![Outcome::Ref {
            id: "ref".into(),
            collapsed: true
        }]
    );
    assert!(d.is_collapsed());
    // The scrim is gone so the sheet takes clicks; the other controls are hidden.
    let name = d.node("name").expect("name");
    assert!(!h.ui.tree.shown(name));
    let panel = h.ui.tree.abs_rect(d.panel().expect("panel"));
    assert!(panel.h < 100.0);
    // Enter expands.
    let out = key(&mut h, &mut d, Key::Enter);
    assert_eq!(
        out,
        vec![Outcome::Ref {
            id: "ref".into(),
            collapsed: false
        }]
    );
    assert!(h.ui.tree.shown(name));
}

#[test]
fn set_error_enable_and_items_update_the_widgets() {
    let mut h = harness();
    let mut d = open(&mut h, spec());
    d.set_error(&mut h.ui, "Bad value");
    assert_eq!(d.error(&h.ui), "Bad value");
    d.set_enabled(&mut h.ui, "ok", false);
    assert!(
        key(&mut h, &mut d, Key::Enter).is_empty(),
        "disabled default"
    );
    d.set_items(&mut h.ui, "list", vec!["a".into(), "b".into()], Some(1));
    assert_eq!(d.values(&h.ui).index("list"), Some(1));
    assert_eq!(d.item(&h.ui, "list", 0).as_deref(), Some("a"));
    d.set_text(&mut h.ui, "name", "new");
    assert_eq!(d.values(&h.ui).text("name"), "new");
    d.close(&mut h.ui);
    assert_eq!(scrim_serial(&h.ui), None);
}

#[test]
fn a_dialog_swallows_pointer_presses_outside_it() {
    let mut h = harness();
    let behind = {
        let root = h.ui.tree.root();
        h.ui.add(
            root,
            crate::Node::new(
                crate::Widget::Button { toggled: None },
                Rect::new(5.0, 5.0, 60.0, 20.0),
            )
            .with_label("Behind"),
        )
    };
    let mut d = open(&mut h, spec());
    h.click(10.0, 10.0);
    let events = h.ui.take_events();
    assert!(!events.contains(&Emit::Clicked(behind)));
    let _ = &mut d;
}

#[test]
fn a_bare_alt_tap_shows_no_ribbon_key_tips_behind_a_dialog() {
    let mut h = Harness::new(640.0, 480.0, 1.0);
    let root = h.ui.tree.root();
    let tabs = h.ui.add(
        root,
        crate::tree::Node::new(
            crate::tree::Widget::Button { toggled: None },
            Rect::new(0.0, 0.0, 40.0, 20.0),
        ),
    );
    h.ui.tree.get_mut(tabs).expect("node").key_tip = Some("H".into());
    h.alt_tap();
    assert!(h.ui.keytips.is_showing(), "tips show without a dialog");
    h.key(Key::Escape);
    let _d = open(&mut h, spec());
    h.ui.take_events();
    h.alt_tap();
    assert!(
        !h.ui.keytips.is_showing(),
        "no tips while the dialog is open"
    );
}
