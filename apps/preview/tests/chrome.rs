//! The preview chrome as widgets: placement equals the shared layout, and the ribbon is
//! interactive (tabs, hover, press, menus, key tips, Backstage, responsive collapse).

use open365_preview::{AppKind, Preview};
use open365_ui_core::{Key, NodeId, Widget};

fn preview(kind: AppKind) -> Preview {
    Preview::new(kind, true, 1600, 1000, 1.25)
}

fn rects(p: &Preview) -> Vec<(NodeId, open365_ui_core::DeviceRect)> {
    p.ui.tree
        .paint_order()
        .into_iter()
        .map(|id| (id, p.ui.tree.device_rect(id, p.ui.scale)))
        .collect()
}

fn find(p: &Preview, label: &str) -> NodeId {
    p.ui.tree
        .paint_order()
        .into_iter()
        .find(|id| p.ui.tree.get(*id).is_some_and(|n| n.label == label))
        .unwrap_or_else(|| panic!("no node labelled {label}"))
}

fn centre(p: &Preview, id: NodeId) -> (f32, f32) {
    let r = p.ui.tree.abs_rect(id).center();
    (r.x * p.scale(), r.y * p.scale())
}

#[test]
fn every_placed_control_is_a_node_on_exactly_the_layout_rectangle() {
    for kind in [AppKind::Sheets, AppKind::Writer] {
        let mut p = preview(kind);
        for tab in 0..p.shell.spec().tabs.len() {
            p.shell.select_tab(&mut p.ui, tab);
            let nodes = rects(&p);
            for (id, r) in p.shell.placed_controls() {
                assert!(
                    nodes
                        .iter()
                        .any(|(_, d)| (d.x0, d.y0, d.x1, d.y1) == (r.x, r.y, r.right(), r.bottom())),
                    "{kind:?} tab {tab}: control {id} at {r:?} has no node"
                );
            }
        }
    }
}

#[test]
fn tabs_switch_by_click_and_by_key_tip_and_hover_press_follow_the_pointer() {
    let mut p = preview(AppKind::Sheets);
    let strip = find(&p, "Ribbon tabs");
    let r = p.ui.tree.abs_rect(strip);
    let s = p.scale();
    // Click the second tab (Insert).
    p.click((r.x + 90.0) * s, (r.y + r.h / 2.0) * s);
    assert_eq!(p.shell.tab(), 1);
    assert!(
        p.ui.tree.paint_order().iter().any(|id| p
            .ui
            .tree
            .get(*id)
            .is_some_and(|n| n.label == "Tables"))
    );
    // Hover then press a ribbon button.
    p.shell.select_tab(&mut p.ui, 0);
    let bold = find(&p, "Bold");
    let (x, y) = centre(&p, bold);
    p.move_to(x, y);
    assert_eq!(p.ui.hover(), Some(bold));
    p.press(x, y);
    assert_eq!(p.ui.pressed(), Some(bold));
    p.release(x, y);
    assert!(p.shell.title().contains("bold"), "{}", p.shell.title());
    // Alt, then the tab tip N, shows the Insert tab and its command tips.
    p.alt_tap();
    assert!(p.ui.keytips.is_showing());
    p.key(Key::Char('N'));
    assert_eq!(p.shell.tab(), 1);
    assert_eq!(p.ui.keytips.depth(), 1);
    assert!(!p.ui.visible_tips().is_empty(), "command tips are badged");
}

#[test]
fn split_arrow_opens_a_menu_and_a_menu_item_runs() {
    let mut p = preview(AppKind::Sheets);
    let paste = find(&p, "Paste");
    let a = p.ui.tree.abs_rect(paste);
    let s = p.scale();
    p.click((a.x + a.w / 2.0) * s, (a.bottom() - 6.0) * s);
    let open = p.ui.tree.paint_order().into_iter().any(|id| {
        p.ui.tree
            .get(id)
            .is_some_and(|n| matches!(&n.widget, Widget::Menu(m) if m.open))
    });
    assert!(
        open,
        "the arrow part of the large split button opens its menu"
    );
}

#[test]
fn backstage_opens_from_file_and_closes_with_back() {
    let mut p = preview(AppKind::Writer);
    let file = find(&p, "File");
    let (x, y) = centre(&p, file);
    p.click(x, y);
    assert!(p.shell.backstage_open());
    let back = find(&p, "Back");
    let (x, y) = centre(&p, back);
    p.click(x, y);
    assert!(!p.shell.backstage_open());
}

#[test]
fn narrow_windows_collapse_groups_and_the_popup_shows_the_full_group() {
    let mut p = preview(AppKind::Sheets);
    p.resize(700, 700);
    let groups: Vec<NodeId> =
        p.ui.tree
            .paint_order()
            .into_iter()
            .filter(|id| {
                p.ui.tree
                    .get(*id)
                    .is_some_and(|n| n.label == "Clipboard" || n.label == "Font")
            })
            .collect();
    assert!(!groups.is_empty());
    // Every node stays inside the window.
    for (_, r) in rects(&p) {
        assert!(r.x0 >= 0 && r.y0 >= 0 && r.y1 <= 700, "{r:?}");
    }
    // Open a collapsed group's popup and find its full contents.
    let collapsed = p.ui.tree.paint_order().into_iter().find(|id| {
        p.ui.tree.get(*id).is_some_and(|n| {
            matches!(n.widget, Widget::Button { .. }) && n.look.chevron && n.label == "Font"
        })
    });
    if let Some(c) = collapsed {
        let (x, y) = centre(&p, c);
        p.click(x, y);
        assert!(
            p.ui.tree.paint_order().iter().any(|id| p
                .ui
                .tree
                .get(*id)
                .is_some_and(|n| n.label == "Bold"))
        );
    }
}

#[test]
fn chrome_renders_with_the_expected_surfaces() {
    let mut p = preview(AppKind::Sheets);
    let Some(img) = p.render() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    // Dark frame at the very top, the ribbon card below the tabs, the white grid in the middle.
    assert_eq!(&img.pixel(900, 3)[..3], &[0x09, 0x09, 0x09]);
    assert_eq!(&img.pixel(900, 200)[..3], &[0x29, 0x29, 0x29]);
    assert_eq!(&img.pixel(900, 600)[..3], &[255, 255, 255]);
}
