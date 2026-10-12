//! The Paste Special dialog (Ctrl+Alt+V, the cell context menu). The choices the model can do
//! are enabled (All, Formulas, Values, Formats, Skip blanks, Transpose); the others are drawn
//! disabled, in Excel's order.

use open365_ui_core::Ui;
use open365_ui_core::dialog::{Dialog, Spec, Values};

use super::View;
use crate::dialogs::request::DialogRequest;
use crate::sheet::{Paste, PasteSpecialOpts, PasteWhat};
use crate::shell::Shell;

/// Excel's list, with the model's support: (label, id, supported).
const WHAT: [(&str, &str, bool); 12] = [
    ("&All", "what:all", true),
    ("&Formulas", "what:formulas", true),
    ("&Values", "what:values", true),
    ("Forma&ts", "what:formats", true),
    ("Comments and &Notes", "what:comments", false),
    ("Va&lidation", "what:validation", false),
    ("All using Source t&heme", "what:theme", false),
    ("All e&xcept borders", "what:noborders", false),
    ("Column &widths", "what:widths", false),
    (
        "Formulas and number f&ormats",
        "what:formulas-numfmt",
        false,
    ),
    ("Values and number fo&rmats", "what:values-numfmt", false),
    ("All merging conditional formats", "what:merge-cf", false),
];
const OPERATION: [(&str, &str, bool); 5] = [
    ("Non&e", "op:none", true),
    ("A&dd", "op:add", false),
    ("&Subtract", "op:subtract", false),
    ("&Multiply", "op:multiply", false),
    ("D&ivide", "op:divide", false),
];

#[derive(Debug)]
struct PasteSpecial;

impl Shell {
    /// Ctrl+Alt+V: opens the dialog when something was copied in this window.
    pub fn open_paste_special_dialog(&mut self, ui: &mut Ui) {
        if !self.sheet.has_copied() {
            self.note = "Copy cells first, then paste special".to_owned();
            return;
        }
        self.open_dialog(ui, DialogRequest::PasteSpecial);
    }
}

pub(super) fn make(_s: &mut Shell) -> (Spec, Box<dyn View>) {
    let mut spec = Spec::new("Paste Special", 440.0, 360.0);
    spec.group(12.0, 8.0, 262.0, 300.0, "Paste");
    spec.group(284.0, 8.0, 144.0, 140.0, "Operation");
    for (i, (label, id, ok)) in WHAT.iter().enumerate() {
        let c = spec.radio(
            id,
            "what",
            24.0,
            28.0 + 22.0 * i as f32,
            240.0,
            label,
            i == 0,
        );
        c.enabled = *ok;
    }
    for (i, (label, id, ok)) in OPERATION.iter().enumerate() {
        let c = spec.radio(
            id,
            "op",
            296.0,
            28.0 + 22.0 * i as f32,
            124.0,
            label,
            i == 0,
        );
        c.enabled = *ok;
    }
    spec.check("skip", 12.0, 318.0, 200.0, "Skip &blanks", false);
    spec.check("transpose", 284.0, 318.0, 140.0, "Trans&pose", false);
    spec.focus = Some("what:all".to_owned());
    (spec, Box::new(PasteSpecial))
}

impl View for PasteSpecial {
    fn commit(
        &mut self,
        s: &mut Shell,
        _ui: &mut Ui,
        _d: &mut Dialog,
        v: &Values,
    ) -> Result<(), String> {
        let what = match v.radio("what") {
            "what:formulas" => PasteWhat::Formulas,
            "what:values" => PasteWhat::Values,
            "what:formats" => PasteWhat::Formats,
            _ => PasteWhat::All,
        };
        let o = PasteSpecialOpts {
            what,
            skip_blanks: v.flag("skip"),
            transpose: v.flag("transpose"),
        };
        s.note = match s.sheet.paste_special_with(o) {
            Ok(Paste::Done) => "pasted".to_owned(),
            Ok(_) => "nothing to paste".to_owned(),
            Err(e) => format!("paste failed: {e}"),
        };
        Ok(())
    }
}
