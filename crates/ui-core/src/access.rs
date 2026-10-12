//! The AccessKit tree built from the widget tree: roles, names, states, bounds in device pixels,
//! values and the focus node.

use accesskit::{
    Action, Node, NodeId as AkId, Rect as AkRect, Role, Toggled, Tree as AkTree, TreeId, TreeUpdate,
};

use crate::tree::{NodeId, Widget, tab_rects};
use crate::ui::Ui;

/// The AccessKit id of the window (root) node.
pub const WINDOW_ID: AkId = AkId(0);

/// Maps a widget id to its AccessKit id (never zero, which is the window).
pub fn ak_id(id: NodeId) -> AkId {
    AkId(id.as_u64() + 1)
}

/// The tab of a strip as an AccessKit id: strips own their tabs, ids live in the high range.
fn tab_id(id: NodeId, i: usize) -> AkId {
    AkId((1 << 62) | ((id.as_u64() & 0xFFFF_FFFF) << 16) | i as u64)
}

fn role_of(w: &Widget) -> Role {
    match w {
        Widget::Panel => Role::GenericContainer,
        Widget::Label => Role::Label,
        Widget::Button { .. } => Role::Button,
        Widget::SplitButton { .. } => Role::Button,
        Widget::Dropdown { .. } => Role::ComboBox,
        Widget::Combo { .. } => Role::ComboBox,
        Widget::Menu(_) => Role::Menu,
        Widget::ScrollBar(_) => Role::ScrollBar,
        Widget::TabStrip { .. } => Role::TabList,
        Widget::Tooltip => Role::Tooltip,
        Widget::TextField(e) if e.masked => Role::PasswordInput,
        Widget::TextField(_) => Role::TextInput,
        Widget::Checkbox { .. } => Role::CheckBox,
        Widget::Radio { .. } => Role::RadioButton,
        Widget::Spinner { .. } => Role::SpinButton,
        Widget::Slider { .. } => Role::Slider,
        Widget::Canvas { .. } => Role::Grid,
    }
}

impl Ui {
    /// Builds the full accessibility tree. Hidden nodes are omitted.
    pub fn accessibility_tree(&mut self) -> TreeUpdate {
        self.tree.layout();
        let s = self.scale;
        let mut nodes: Vec<(AkId, Node)> = Vec::new();
        let mut root_children = Vec::new();
        let rect = |r: crate::geom::Rect| {
            let d = s.snap(r);
            AkRect {
                x0: f64::from(d.x0),
                y0: f64::from(d.y0),
                x1: f64::from(d.x1),
                y1: f64::from(d.y1),
            }
        };
        let root = self.tree.root();
        let mut order = self.tree.paint_order();
        order.retain(|i| *i != root);
        for id in order {
            let Some(n) = self.tree.get(id).cloned() else {
                continue;
            };
            let mut node = Node::new(role_of(&n.widget));
            if !n.label.is_empty() {
                node.set_label(n.label.clone());
            }
            if let Some(t) = &n.tooltip {
                node.set_description(t.clone());
            }
            node.set_bounds(rect(self.tree.abs_rect(id)));
            if !n.enabled {
                node.set_disabled();
            }
            if let Some(k) = n
                .access_key
                .or_else(|| n.key_tip.as_ref().and_then(|t| t.chars().next()))
            {
                node.set_keyboard_shortcut(format!("Alt+{}", k.to_ascii_uppercase()));
            }
            if n.widget.focusable() {
                node.add_action(Action::Focus);
            }
            match &n.widget {
                Widget::Button { toggled } => {
                    node.add_action(Action::Click);
                    if let Some(t) = toggled {
                        node.set_toggled(if *t { Toggled::True } else { Toggled::False });
                    }
                }
                Widget::SplitButton { .. } => {
                    node.add_action(Action::Click);
                }
                Widget::Checkbox { checked } | Widget::Radio { checked, .. } => {
                    node.add_action(Action::Click);
                    node.set_toggled(if *checked {
                        Toggled::True
                    } else {
                        Toggled::False
                    });
                }
                Widget::Dropdown {
                    items,
                    selected,
                    open,
                } => {
                    node.set_value(items.get(*selected).cloned().unwrap_or_default());
                    node.set_expanded(*open);
                    node.add_action(Action::Click);
                }
                Widget::Combo { edit, open, .. } => {
                    node.set_value(edit.text.clone());
                    node.set_expanded(*open);
                    node.add_action(Action::SetValue);
                }
                Widget::Canvas { .. } => {
                    node.add_action(Action::Click);
                }
                Widget::TextField(e) => {
                    // A password is not told to screen readers.
                    if !e.masked {
                        node.set_value(e.text.clone());
                    }
                    node.add_action(Action::SetValue);
                }
                Widget::Spinner {
                    value,
                    min,
                    max,
                    step,
                    edit,
                } => {
                    node.set_value(edit.text.clone());
                    node.set_numeric_value(*value);
                    node.set_min_numeric_value(*min);
                    node.set_max_numeric_value(*max);
                    node.set_numeric_value_step(*step);
                    node.add_action(Action::Increment);
                    node.add_action(Action::Decrement);
                }
                Widget::Slider {
                    value,
                    min,
                    max,
                    step,
                } => {
                    node.set_numeric_value(f64::from(*value));
                    node.set_min_numeric_value(f64::from(*min));
                    node.set_max_numeric_value(f64::from(*max));
                    node.set_numeric_value_step(f64::from(*step));
                    node.add_action(Action::Increment);
                    node.add_action(Action::Decrement);
                }
                Widget::ScrollBar(sc) => {
                    node.set_numeric_value(f64::from(sc.value));
                    node.set_min_numeric_value(0.0);
                    node.set_max_numeric_value(f64::from(sc.max));
                    node.set_orientation(if sc.vertical {
                        accesskit::Orientation::Vertical
                    } else {
                        accesskit::Orientation::Horizontal
                    });
                }
                Widget::TabStrip {
                    tabs,
                    selected,
                    gap,
                } => {
                    let ar = self.tree.abs_rect(id);
                    let rects = tab_rects(crate::geom::Point::new(ar.x, ar.y), ar.h, tabs, *gap);
                    let mut kids = Vec::new();
                    for (i, (t, r)) in tabs.iter().zip(rects).enumerate() {
                        let mut tn = Node::new(Role::Tab);
                        tn.set_label(t.label.clone());
                        tn.set_bounds(rect(r));
                        tn.set_selected(i == *selected);
                        tn.add_action(Action::Click);
                        if !t.enabled {
                            tn.set_disabled();
                        }
                        if let Some(k) = &t.key_tip {
                            tn.set_keyboard_shortcut(format!("Alt+{k}"));
                        }
                        kids.push(tab_id(id, i));
                        nodes.push((tab_id(id, i), tn));
                    }
                    node.set_children(kids);
                }
                Widget::Menu(m) => {
                    if m.open {
                        let items: Vec<AkId> = (0..m.items.len())
                            .map(|i| AkId((1 << 61) | (id.as_u64() << 8) | i as u64))
                            .collect();
                        for (i, it) in m.items.iter().enumerate() {
                            let mut mi = Node::new(if it.separator {
                                Role::Splitter
                            } else if it.heading {
                                Role::Label
                            } else if it.checked {
                                Role::MenuItemCheckBox
                            } else {
                                Role::MenuItem
                            });
                            mi.set_label(it.label.clone());
                            if !it.enabled {
                                mi.set_disabled();
                            }
                            if !it.submenu.is_empty() {
                                mi.set_has_popup(accesskit::HasPopup::Menu);
                            }
                            nodes.push((items[i], mi));
                        }
                        node.set_children(items);
                    } else {
                        continue;
                    }
                }
                _ => {}
            }
            if let Some((over, extra)) = self.canvas_access.get(&id) {
                let mut o = over.clone();
                if let Some(l) = node.label() {
                    o.set_label(l.to_owned());
                }
                if let Some(b) = node.bounds() {
                    o.set_bounds(b);
                }
                node = o;
                nodes.extend(extra.iter().cloned());
            }
            let id_ak = ak_id(id);
            // Parent chain: top-level widgets hang off the window; nested ones off their parent.
            nodes.push((id_ak, node));
            if n.parent == Some(root) {
                root_children.push(id_ak);
            }
        }
        // Wire nested children (a widget that is not a direct child of the root).
        let ids: Vec<(AkId, Option<NodeId>)> = nodes.iter().map(|(a, _)| (*a, None)).collect();
        let _ = ids;
        let mut by_parent: std::collections::HashMap<AkId, Vec<AkId>> =
            std::collections::HashMap::new();
        for id in self.tree.paint_order() {
            if id == root {
                continue;
            }
            let Some(n) = self.tree.get(id) else { continue };
            if let Some(p) = n.parent
                && p != root
            {
                by_parent.entry(ak_id(p)).or_default().push(ak_id(id));
            }
        }
        for (a, node) in &mut nodes {
            if let Some(kids) = by_parent.remove(a) {
                let mut all: Vec<AkId> = node.children().to_vec();
                all.extend(kids);
                node.set_children(all);
            }
        }
        let mut window = Node::new(Role::Window);
        window.set_children(root_children);
        let wr = self.tree.get(root).map(|n| n.bounds).unwrap_or_default();
        window.set_bounds(rect(crate::geom::Rect::new(0.0, 0.0, wr.w, wr.h)));
        nodes.push((WINDOW_ID, window));
        TreeUpdate {
            nodes,
            tree: Some(AkTree::new(WINDOW_ID)),
            tree_id: TreeId::ROOT,
            focus: self.focus.map_or(WINDOW_ID, ak_id),
        }
    }

    /// Where the IME candidate window belongs: the caret of the focused text field, in device
    /// pixels, or `None` when no text field has focus.
    pub fn ime_cursor_area(&self) -> Option<crate::geom::DeviceRect> {
        let f = self.focus?;
        let n = self.tree.get(f)?;
        if let Widget::Canvas { caret } = &n.widget {
            let ar = self.tree.abs_rect(f);
            return Some(self.scale.snap(crate::geom::Rect::new(
                ar.x + caret.x,
                ar.y + caret.y,
                caret.w,
                caret.h,
            )));
        }
        let e = match &n.widget {
            Widget::TextField(e)
            | Widget::Combo { edit: e, .. }
            | Widget::Spinner { edit: e, .. } => e,
            _ => return None,
        };
        let e = e.display();
        let ar = self.tree.abs_rect(f);
        let lay = self.assets.layout_text(
            self.font,
            &e.text,
            self.text_px(),
            0.0,
            0.0,
            Default::default(),
        );
        let x = self.scale.edge(ar.x + 4.0) + lay.x_at(e.caret).round() as i32;
        let d = self.scale.snap(ar);
        Some(crate::geom::DeviceRect {
            x0: x,
            y0: d.y0,
            x1: x + 1,
            y1: d.y1,
        })
    }

    /// True while a text field has focus and the IME should be allowed.
    pub fn wants_ime(&self) -> bool {
        self.ime_enabled
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_id_is_reserved() {
        assert_eq!(super::WINDOW_ID.0, 0);
    }
}
