//! Helpers shared by the `grid_draw_*` tests (included with `mod grid_draw_common;`).

#![allow(clippy::unwrap_used, dead_code)]

use open365_gfx::DrawList;
use open365_preview::sheet::{MemoryClipboard, SheetModel};
use open365_preview::{AppKind, Preview};
use open365_ui_core::geom::Point;
use open365_ui_core::{InputEvent, Key, Modifiers, Widget};

pub fn sheets() -> Preview {
    let mut p = Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25);
    p.shell
        .sheet_mut()
        .set_clipboard(MemoryClipboard::default());
    p
}

pub fn open_fixture(name: &str) -> Preview {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx")
        .join(name);
    let mut p = sheets();
    let mut model = SheetModel::open(&path).unwrap();
    model.set_clipboard(MemoryClipboard::default());
    p.shell.set_sheet(&mut p.ui, model);
    p
}

/// Number of opaque vertices of this colour in the frame: a drawn shape of a known colour.
pub fn count(list: &DrawList, rgb: [u8; 3]) -> usize {
    list.vertices()
        .iter()
        .filter(|v| v.color[..3] == rgb && v.color[3] == 255)
        .count()
}

pub fn chord(p: &mut Preview, key: Key, m: Modifiers) {
    for pressed in [true, false] {
        p.input(InputEvent::Key {
            key,
            pressed,
            modifiers: m,
        });
    }
}

pub fn mods(ctrl: bool, shift: bool, alt: bool) -> Modifiers {
    Modifiers { shift, ctrl, alt }
}

pub fn menu_labels(p: &Preview) -> Vec<String> {
    let id = p.ui.context_menu().expect("a context menu");
    match p.ui.tree.get(id).map(|n| &n.widget) {
        Some(Widget::Menu(m)) if m.open => m.items.iter().map(|i| i.label.clone()).collect(),
        _ => panic!("the context menu is not open"),
    }
}

pub fn menu_open(p: &Preview) -> bool {
    p.ui.context_menu()
        .and_then(|id| p.ui.tree.get(id))
        .is_some_and(|n| matches!(&n.widget, Widget::Menu(m) if m.open))
}

/// Picks the item called `label` with the keyboard (Down to it, then Enter).
pub fn pick(p: &mut Preview, label: &str) {
    let id = p.ui.context_menu().expect("a context menu");
    let Some(Widget::Menu(m)) = p.ui.tree.get(id).map(|n| &n.widget) else {
        panic!("not a menu");
    };
    let selectable: Vec<&str> = m
        .items
        .iter()
        .filter(|i| !i.separator && !i.heading && i.enabled)
        .map(|i| i.label.as_str())
        .collect();
    let at = selectable
        .iter()
        .position(|l| *l == label)
        .unwrap_or_else(|| panic!("no item {label} in {selectable:?}"));
    for _ in 0..=at {
        p.key(Key::Down);
    }
    p.key(Key::Enter);
}

/// Device point inside cell `(r, c)`, `from_right` and `from_bottom` logical (96 dpi) pixels in
/// from its bottom right corner.
pub fn in_cell_corner(p: &Preview, r: u32, c: u32, from_right: f32, from_bottom: f32) -> Point {
    let s = p.scale();
    let centre = p.shell.cell_centre(r, c);
    let geo = p.shell.sheet().geometry();
    Point::new(
        centre.x + geo.col_px(c) as f32 * s / 2.0 - from_right * s,
        centre.y + geo.row_px(r) as f32 * s / 2.0 - from_bottom * s,
    )
}
