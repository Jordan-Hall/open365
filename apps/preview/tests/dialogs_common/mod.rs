//! Helpers the dialog tests share: a Sheets or Writer preview, synthetic key events routed to
//! the dialog host first (as the Sheets grid does through `dialog_event` and the Writer will
//! through the hook at the top of `Shell::handle`), and queries about the open dialog.

#![allow(dead_code, clippy::unwrap_used)]

use open365_preview::doc::WriterPane;
use open365_preview::{AppKind, Preview};
use open365_ui_core::dialog::scrim_serial;
use open365_ui_core::{InputEvent, Key, Modifiers, NodeId};

pub fn sheets() -> Preview {
    Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25)
}

pub fn writer(text: &str) -> Preview {
    let mut p = Preview::new(AppKind::Writer, true, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text(text);
    p.shell.set_writer(&mut p.ui, pane);
    p
}

/// Routes the pending widget events to the dialog host, then to the shell.
pub fn route(p: &mut Preview) {
    for _ in 0..4 {
        let events = p.ui.take_events();
        if events.is_empty() {
            break;
        }
        let mut rest = Vec::new();
        for ev in events {
            if !p.shell.dialog_handle(&mut p.ui, &ev) {
                rest.push(ev);
            }
        }
        if !rest.is_empty() {
            let acts = p.shell.handle(&mut p.ui, rest);
            p.window_acts.extend(acts);
        }
    }
}

pub fn key_with(p: &mut Preview, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        p.ui.handle(InputEvent::Key {
            key,
            pressed,
            modifiers,
        });
        route(p);
    }
}

pub fn key(p: &mut Preview, key: Key) {
    key_with(p, key, Modifiers::default());
}

pub fn keys(p: &mut Preview, list: &[Key]) {
    for k in list {
        key(p, *k);
    }
}

pub fn shift(p: &mut Preview, k: Key) {
    key_with(
        p,
        k,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
}

pub fn ctrl(p: &mut Preview, k: Key) {
    key_with(
        p,
        k,
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    );
}

pub fn type_text(p: &mut Preview, s: &str) {
    p.ui.handle(InputEvent::Text(s.to_owned()));
    route(p);
}

/// Alt+`c`: the access key of a dialog control.
pub fn alt(p: &mut Preview, c: char) {
    let m = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    p.ui.handle(InputEvent::Key {
        key: Key::Alt,
        pressed: true,
        modifiers: m,
    });
    route(p);
    key_with(p, Key::Char(c.to_ascii_uppercase()), m);
    p.ui.handle(InputEvent::Key {
        key: Key::Alt,
        pressed: false,
        modifiers: Modifiers::default(),
    });
    route(p);
}

/// Opens the dialog a bare ribbon command stands for: the shell's own mapping first, then the
/// request the model left (`take_request()`, which `run_command` now opens itself).
pub fn bare(p: &mut Preview, id: &str) -> bool {
    if p.shell.try_open_dialog(&mut p.ui, id) {
        return true;
    }
    p.shell.run_command(&mut p.ui, id);
    p.shell.poll_dialog_requests(&mut p.ui) || is_open(p)
}

pub fn is_open(p: &Preview) -> bool {
    scrim_serial(&p.ui).is_some()
}

/// The title of the open dialog.
pub fn title(p: &Preview) -> Option<String> {
    let serial = scrim_serial(&p.ui)?;
    let root = p.ui.tree.get(p.ui.tree.root())?;
    let scrim = root.children.iter().rev().find(|c| {
        p.ui.tree
            .get(**c)
            .is_some_and(|n| n.label == format!("o365-dialog:{serial}"))
    })?;
    let panel = p.ui.tree.get(*scrim)?.children.first().copied()?;
    p.ui.tree.get(panel).map(|n| n.label.clone())
}

pub fn find(p: &Preview, label: &str) -> Option<NodeId> {
    p.ui.tree
        .paint_order()
        .into_iter()
        .find(|id| p.ui.tree.get(*id).is_some_and(|n| n.label == label))
}

pub fn focus_label(p: &Preview) -> String {
    p.ui.focus()
        .and_then(|f| p.ui.tree.get(f))
        .map(|n| n.label.clone())
        .unwrap_or_default()
}

pub fn click(p: &mut Preview, id: NodeId) {
    let r = p.ui.tree.abs_rect(id).center();
    let s = p.scale();
    p.move_to(r.x * s, r.y * s);
    p.press(r.x * s, r.y * s);
    p.release(r.x * s, r.y * s);
    route(p);
}

/// Fills a column of the Sheets model from `rows`, starting at row 1.
pub fn fill_col(p: &mut Preview, col: u32, rows: &[&str]) {
    for (i, v) in rows.iter().enumerate() {
        p.shell.sheet_mut().enter(i as u32 + 1, col, v).unwrap();
    }
}

/// Every non-empty label under the open dialog, for failure messages.
pub fn dump(p: &Preview) -> Vec<String> {
    let Some(serial) = scrim_serial(&p.ui) else {
        return Vec::new();
    };
    let root = p.ui.tree.get(p.ui.tree.root()).unwrap();
    let Some(scrim) = root.children.iter().rev().find(|c| {
        p.ui.tree
            .get(**c)
            .is_some_and(|n| n.label == format!("o365-dialog:{serial}"))
    }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![*scrim];
    while let Some(id) = stack.pop() {
        if let Some(n) = p.ui.tree.get(id) {
            if !n.label.is_empty() && p.ui.tree.shown(id) {
                out.push(n.label.clone());
            }
            stack.extend(n.children.iter().copied());
        }
    }
    out
}
