//! The description of a dialog: its controls, their rectangles (logical units, relative to the
//! client area under the title bar) and the footer buttons. Pure data; [`super::Dialog`] turns it
//! into widgets.

use std::collections::BTreeMap;

use crate::geom::Rect;

/// Height of the title bar in logical units.
pub const TITLE_H: f32 = 30.0;
/// Height of the footer row that holds OK, Cancel and Apply.
pub const FOOTER_H: f32 = 47.0;
/// Width and height of a push button.
pub const BUTTON_W: f32 = 75.0;
pub const BUTTON_H: f32 = 23.0;
/// Height of a text field, spin box or drop-down.
pub const FIELD_H: f32 = 23.0;
/// Height of a label, check box or radio button row.
pub const ROW_H: f32 = 20.0;
/// Space between the dialog edge and the controls.
pub const MARGIN: f32 = 12.0;
/// Height of a tab strip.
pub const TAB_H: f32 = 26.0;
/// Row height inside a list box.
pub const LIST_ROW_H: f32 = 19.0;

/// What a control is, with its initial state.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Static text; wraps to the rectangle.
    Label,
    /// A group box: a thin frame with its caption.
    Group,
    /// A one-line text field.
    Text(String),
    /// A spin box.
    Spin {
        value: f64,
        min: f64,
        max: f64,
        step: f64,
    },
    Check(bool),
    /// A radio button of `group`; the dialog keeps one checked per group.
    Radio {
        group: String,
        checked: bool,
    },
    Dropdown {
        items: Vec<String>,
        selected: usize,
    },
    /// An editable drop-down.
    Combo {
        items: Vec<String>,
        text: String,
    },
    /// A list box (single selection) with a scroll bar when the items do not fit.
    List {
        items: Vec<String>,
        selected: Option<usize>,
    },
    /// A reference field with a button that collapses the dialog so a range can be picked on the
    /// sheet.
    Ref(String),
    /// A push button that is not part of the footer.
    Button,
}

/// One control. `label` may carry an access key as `&N` (the letter after `&`).
#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    pub id: String,
    pub kind: Kind,
    pub rect: Rect,
    pub label: String,
    /// Shown only on this page (tab or wizard step); `None` shows on all pages.
    pub page: Option<usize>,
    pub enabled: bool,
}

/// What a footer button does when pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonRole {
    /// Enter presses it.
    Default,
    /// Escape and the title bar's close button press it.
    Cancel,
    Normal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FooterButton {
    pub id: String,
    pub label: String,
    pub role: ButtonRole,
    pub enabled: bool,
}

impl FooterButton {
    pub fn new(id: &str, label: &str, role: ButtonRole) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            role,
            enabled: true,
        }
    }
}

/// The description of a whole dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub title: String,
    /// Client width and the height of the area above the footer.
    pub width: f32,
    pub height: f32,
    pub controls: Vec<Control>,
    pub footer: Vec<FooterButton>,
    /// Tab captions; empty for a dialog without a tab strip.
    pub tabs: Vec<String>,
    /// The control focused first (the first in tab order when `None`; a dialog with tabs starts
    /// on its tab strip, as Office's do).
    pub focus: Option<String>,
    /// The tab or step shown first.
    pub start_page: usize,
    page: Option<usize>,
}

impl Spec {
    /// A dialog with OK and Cancel.
    pub fn new(title: &str, width: f32, height: f32) -> Self {
        Self {
            title: title.to_owned(),
            width,
            height,
            controls: Vec::new(),
            footer: vec![
                FooterButton::new("ok", "OK", ButtonRole::Default),
                FooterButton::new("cancel", "Cancel", ButtonRole::Cancel),
            ],
            tabs: Vec::new(),
            focus: None,
            start_page: 0,
            page: None,
        }
    }

    /// OK, Cancel and Apply (Format Cells does not have Apply; Page Setup in Word does).
    #[must_use]
    pub fn with_apply(mut self) -> Self {
        self.footer
            .push(FooterButton::new("apply", "&Apply", ButtonRole::Normal));
        self
    }

    /// A single Close button, which is both the default and the cancel button.
    #[must_use]
    pub fn close_only(mut self) -> Self {
        self.footer = vec![FooterButton::new("cancel", "Close", ButtonRole::Cancel)];
        self
    }

    /// Replaces the footer.
    #[must_use]
    pub fn footer(mut self, buttons: Vec<FooterButton>) -> Self {
        self.footer = buttons;
        self
    }

    /// Access keys that two controls share on one page, as `(page, key)`: Alt+key could only
    /// reach one of them. Footer buttons and controls shown on every page count on each page.
    #[must_use]
    pub fn access_conflicts(&self) -> Vec<(usize, char)> {
        let pages = self
            .controls
            .iter()
            .filter_map(|c| c.page)
            .filter(|p| *p < 64)
            .max()
            .map_or(1, |m| m + 1);
        let mut out = Vec::new();
        for page in 0..pages {
            let mut seen: Vec<char> = Vec::new();
            let keys = self
                .controls
                .iter()
                .filter(|c| c.page.is_none() || c.page == Some(page))
                .filter(|c| !matches!(c.kind, Kind::Group))
                .filter_map(|c| c.caption().1)
                .chain(self.footer.iter().filter_map(|b| split_access(&b.label).1));
            for k in keys {
                if seen.contains(&k) {
                    if !out.contains(&(page, k)) {
                        out.push((page, k));
                    }
                } else {
                    seen.push(k);
                }
            }
        }
        out
    }

    /// The following controls belong to `page` (`None` for every page).
    pub fn on_page(&mut self, page: Option<usize>) -> &mut Self {
        self.page = page;
        self
    }

    fn push(&mut self, id: &str, kind: Kind, rect: Rect, label: &str) -> &mut Control {
        self.controls.push(Control {
            id: id.to_owned(),
            kind,
            rect,
            label: label.to_owned(),
            page: self.page,
            enabled: true,
        });
        let last = self.controls.len() - 1;
        &mut self.controls[last]
    }

    /// Static text. A `&X` in the text gives the next field its access key.
    pub fn label(&mut self, x: f32, y: f32, w: f32, text: &str) -> &mut Control {
        self.push("", Kind::Label, Rect::new(x, y, w, ROW_H), text)
    }

    /// Wrapped static text of height `h`.
    pub fn note(&mut self, x: f32, y: f32, w: f32, h: f32, text: &str) -> &mut Control {
        self.push("", Kind::Label, Rect::new(x, y, w, h), text)
    }

    /// A group box.
    pub fn group(&mut self, x: f32, y: f32, w: f32, h: f32, caption: &str) -> &mut Control {
        self.push("", Kind::Group, Rect::new(x, y, w, h), caption)
    }

    pub fn text(&mut self, id: &str, x: f32, y: f32, w: f32, value: &str) -> &mut Control {
        self.push(
            id,
            Kind::Text(value.to_owned()),
            Rect::new(x, y, w, FIELD_H),
            "",
        )
    }

    pub fn spin(
        &mut self,
        id: &str,
        x: f32,
        y: f32,
        w: f32,
        (value, min, max, step): (f64, f64, f64, f64),
    ) -> &mut Control {
        self.push(
            id,
            Kind::Spin {
                value,
                min,
                max,
                step,
            },
            Rect::new(x, y, w, FIELD_H),
            "",
        )
    }

    pub fn check(
        &mut self,
        id: &str,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        on: bool,
    ) -> &mut Control {
        self.push(id, Kind::Check(on), Rect::new(x, y, w, ROW_H), label)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn radio(
        &mut self,
        id: &str,
        group: &str,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        checked: bool,
    ) -> &mut Control {
        self.push(
            id,
            Kind::Radio {
                group: group.to_owned(),
                checked,
            },
            Rect::new(x, y, w, ROW_H),
            label,
        )
    }

    pub fn dropdown(
        &mut self,
        id: &str,
        x: f32,
        y: f32,
        w: f32,
        items: &[&str],
        selected: usize,
    ) -> &mut Control {
        self.push(
            id,
            Kind::Dropdown {
                items: items.iter().map(|s| (*s).to_owned()).collect(),
                selected,
            },
            Rect::new(x, y, w, FIELD_H),
            "",
        )
    }

    pub fn combo(
        &mut self,
        id: &str,
        x: f32,
        y: f32,
        w: f32,
        items: &[&str],
        text: &str,
    ) -> &mut Control {
        self.push(
            id,
            Kind::Combo {
                items: items.iter().map(|s| (*s).to_owned()).collect(),
                text: text.to_owned(),
            },
            Rect::new(x, y, w, FIELD_H),
            "",
        )
    }

    pub fn list(
        &mut self,
        id: &str,
        rect: Rect,
        items: Vec<String>,
        selected: Option<usize>,
    ) -> &mut Control {
        self.push(id, Kind::List { items, selected }, rect, "")
    }

    pub fn reference(&mut self, id: &str, x: f32, y: f32, w: f32, value: &str) -> &mut Control {
        self.push(
            id,
            Kind::Ref(value.to_owned()),
            Rect::new(x, y, w, FIELD_H),
            "",
        )
    }

    pub fn button(&mut self, id: &str, x: f32, y: f32, w: f32, label: &str) -> &mut Control {
        self.push(id, Kind::Button, Rect::new(x, y, w, BUTTON_H), label)
    }
}

impl Control {
    /// Greys the control out.
    pub fn disabled(&mut self) -> &mut Self {
        self.enabled = false;
        self
    }

    /// Splits `label` into its text and its access key (`&Name` is `Name` and `N`).
    pub fn caption(&self) -> (String, Option<char>) {
        split_access(&self.label)
    }
}

/// Removes the `&` from a caption and returns the access key it marked (`&&` is a literal `&`).
pub fn split_access(label: &str) -> (String, Option<char>) {
    let mut out = String::with_capacity(label.len());
    let mut key = None;
    let mut it = label.chars().peekable();
    while let Some(c) = it.next() {
        if c == '&' {
            match it.peek().copied() {
                Some('&') => {
                    out.push('&');
                    it.next();
                }
                Some(k) if key.is_none() && !k.is_whitespace() => {
                    key = Some(k.to_ascii_lowercase());
                }
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    (out, key)
}

/// A value read from a control.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Number(f64),
    Bool(bool),
    /// The chosen item of a drop-down or list (`None` when a list has no selection).
    Index(Option<usize>),
}

/// Every control's value at the moment of the call, by control id, plus the checked radio of each
/// group.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Values {
    map: BTreeMap<String, Value>,
    groups: BTreeMap<String, String>,
}

impl Values {
    pub(crate) fn insert(&mut self, id: &str, v: Value) {
        self.map.insert(id.to_owned(), v);
    }

    pub(crate) fn set_group(&mut self, group: &str, id: &str) {
        self.groups.insert(group.to_owned(), id.to_owned());
    }

    pub fn get(&self, id: &str) -> Option<&Value> {
        self.map.get(id)
    }

    /// The text of a field, combo or reference field; an empty string for anything else.
    pub fn text(&self, id: &str) -> &str {
        match self.map.get(id) {
            Some(Value::Text(s)) => s,
            _ => "",
        }
    }

    /// The number in a spin box (or the number a text field holds); 0 when it is not one.
    pub fn num(&self, id: &str) -> f64 {
        match self.map.get(id) {
            Some(Value::Number(n)) => *n,
            Some(Value::Text(s)) => s.trim().parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    pub fn flag(&self, id: &str) -> bool {
        matches!(self.map.get(id), Some(Value::Bool(true)))
    }

    /// The selected item of a drop-down or list.
    pub fn index(&self, id: &str) -> Option<usize> {
        match self.map.get(id) {
            Some(Value::Index(i)) => *i,
            _ => None,
        }
    }

    /// The id of the checked radio button of `group`.
    pub fn radio(&self, group: &str) -> &str {
        self.groups.get(group).map_or("", String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_keys_come_out_of_captions() {
        assert_eq!(split_access("&Name:"), ("Name:".to_owned(), Some('n')));
        assert_eq!(
            split_access("Fish && chips"),
            ("Fish & chips".to_owned(), None)
        );
        assert_eq!(split_access("Cancel"), ("Cancel".to_owned(), None));
        assert_eq!(split_access("Sort &By"), ("Sort By".to_owned(), Some('b')));
    }

    #[test]
    fn shared_access_keys_are_found_per_page() {
        let mut s = Spec::new("T", 100.0, 100.0);
        s.on_page(Some(0));
        s.check("a", 0.0, 0.0, 10.0, "&Wrap", false);
        s.on_page(Some(1));
        s.check("b", 0.0, 0.0, 10.0, "&Wrap", false);
        assert!(
            s.access_conflicts().is_empty(),
            "same key on different pages is fine"
        );
        s.on_page(None);
        s.check("c", 0.0, 0.0, 10.0, "S&hrink", false);
        s.check("d", 0.0, 0.0, 10.0, "&Hide", false);
        assert_eq!(s.access_conflicts(), vec![(0, 'h'), (1, 'h')]);
    }

    #[test]
    fn values_read_back_by_kind() {
        let mut v = Values::default();
        v.insert("a", Value::Text("12".into()));
        v.insert("b", Value::Bool(true));
        v.insert("c", Value::Index(Some(2)));
        v.set_group("g", "r2");
        assert_eq!(v.text("a"), "12");
        assert!((v.num("a") - 12.0).abs() < 1e-9);
        assert!(v.flag("b") && !v.flag("a"));
        assert_eq!(v.index("c"), Some(2));
        assert_eq!(v.radio("g"), "r2");
        assert_eq!(v.radio("none"), "");
    }
}
