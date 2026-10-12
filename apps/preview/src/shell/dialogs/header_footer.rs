//! Custom Header and Custom Footer: three section boxes and the buttons that insert field codes.
//! They open over Page Setup, which takes the answer with [`take_result`] when they close.

use std::cell::RefCell;

use open365_ui_core::Ui;
use open365_ui_core::dialog::{Dialog, Outcome, Spec, Values};
use open365_ui_core::tree::Widget;

use super::{View, notice};
use crate::dialogs::header_footer::{self as hf, Context, Sections};
use crate::dialogs::request::DialogRequest;
use crate::shell::Shell;

thread_local! {
    static RESULT: RefCell<Option<Sections>> = const { RefCell::new(None) };
}

/// The sections the last Custom Header/Footer dialog was closed with (OK), once.
pub(super) fn take_result() -> Option<Sections> {
    RESULT.with(|r| r.borrow_mut().take())
}

/// What the field codes print for the active sheet, for the previews.
pub(super) fn context(s: &Shell) -> Context {
    s.sheet.header_context()
}

pub(super) fn make(_s: &mut Shell, req: &DialogRequest) -> Option<(Spec, Box<dyn View>)> {
    let DialogRequest::HeaderFooterCustom { footer, sections } = req else {
        return None;
    };
    Some(custom(*footer, sections))
}

/// (id, caption, code) of the buttons that put a field code in a box, in two rows.
const INSERTS: [(&str, &str, &str); 7] = [
    ("ins_page", "Page Number", "&P"),
    ("ins_pages", "Page Count", "&N"),
    ("ins_date", "Date", "&D"),
    ("ins_time", "Time", "&T"),
    ("ins_path", "File Path", "&Z"),
    ("ins_file", "File Name", "&F"),
    ("ins_sheet", "Sheet Name", "&A"),
];
const BOXES: [(&str, &str); 3] = [
    ("left", "&Left section:"),
    ("center", "&Center section:"),
    ("right", "&Right section:"),
];

#[derive(Debug)]
struct Custom {
    /// The box a code goes in: the last one that was typed in or focused.
    last: usize,
}

fn custom(footer: bool, sections: &Sections) -> (Spec, Box<dyn View>) {
    let title = if footer { "Footer" } else { "Header" };
    let mut spec = Spec::new(title, 480.0, 214.0);
    spec.note(
        12.0,
        8.0,
        456.0,
        50.0,
        "To insert a page number, date, time, file path, file name or sheet name: put the\ninsertion point in a box, then choose the button.\nCodes: &P page, &N pages, &D date, &T time, &F file, &Z path, &A sheet, && an ampersand.",
    );
    let mut x = 12.0;
    let mut y = 66.0;
    spec.button("fmt_text", x, y, 88.0, "Format Text").enabled = false;
    x += 92.0;
    for (i, (id, caption, _)) in INSERTS.iter().enumerate() {
        if i == 4 {
            x = 12.0;
            y += 28.0;
        }
        spec.button(id, x, y, 88.0, caption);
        x += 92.0;
    }
    spec.button("pic", x, y, 88.0, "Picture").enabled = false;
    spec.button("fmt_pic", x + 92.0, y, 88.0, "Format Pic.")
        .enabled = false;
    for (i, (id, caption)) in BOXES.iter().enumerate() {
        let bx = 12.0 + 156.0 * i as f32;
        spec.label(bx, 134.0, 144.0, caption);
        spec.text(id, bx, 156.0, 144.0, &sections[i]);
    }
    spec.focus = Some("center".to_owned());
    (spec, Box::new(Custom { last: 1 }))
}

impl View for Custom {
    fn event(&mut self, _s: &mut Shell, ui: &mut Ui, d: &mut Dialog, o: &Outcome) {
        match o {
            Outcome::Changed(id) => {
                if let Some(i) = BOXES.iter().position(|(b, _)| b == id) {
                    self.last = i;
                }
            }
            Outcome::Pressed(id) => {
                let Some((_, _, code)) = INSERTS.iter().find(|(b, _, _)| b == id) else {
                    return;
                };
                let box_id = BOXES[self.last].0;
                let Some(node) = d.node(box_id) else { return };
                let (text, sel) = match ui.tree.get(node).map(|n| &n.widget) {
                    Some(Widget::TextField(e)) => (e.text.clone(), e.selection()),
                    _ => return,
                };
                let (new, caret) = hf::insert_code(&text, sel, code);
                d.set_text(ui, box_id, &new);
                d.focus(ui, box_id);
                if let Some(Widget::TextField(e)) = ui.tree.get_mut(node).map(|n| &mut n.widget) {
                    e.caret = caret;
                    e.anchor = caret;
                }
            }
            _ => {}
        }
    }

    fn commit(
        &mut self,
        _s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let sections: Sections = [
            v.text("left").to_owned(),
            v.text("center").to_owned(),
            v.text("right").to_owned(),
        ];
        if let Err(msg) = hf::check_length(&sections) {
            notice("Open365", &msg);
            return Err(String::new());
        }
        RESULT.with(|r| *r.borrow_mut() = Some(sections));
        Ok(())
    }

    fn cancel(&mut self, _s: &mut Shell, _ui: &mut Ui) {
        RESULT.with(|r| *r.borrow_mut() = None);
    }
}
