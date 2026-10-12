//! Modal dialog framework (ADR UI platform, phase 3): a [`Spec`] describes the controls of an
//! Office-style dialog, [`Dialog`] turns it into widgets on a [`crate::Ui`] and handles the
//! keyboard the way Windows dialogs do.
//!
//! * Tab and Shift+Tab walk the controls in spec order (one stop per radio group and per list
//!   box, the tab strip first, the footer buttons last); Ctrl+Tab switches tab pages.
//! * Enter presses the footer's default button, Esc the cancel button (or closes an open
//!   drop-down, or expands a collapsed reference field first).
//! * `&N` in a caption makes Alt+N press or focus that control; a label's key goes to the next
//!   field.
//! * The panel is a modal AccessKit `Dialog` labelled by its title.
//! * Everything is laid out in logical units, so DPI scaling is the `Ui`'s.
//! * Dragging the title bar moves the dialog; the reference field collapses it to one line so a
//!   range can be picked on the sheet.

mod runtime;
mod spec;
#[cfg(test)]
mod tests;

pub use runtime::{Dialog, Icons, Outcome, Response, format_number, scrim_serial};
pub use spec::{
    BORDER_EDGES, BUTTON_H, BUTTON_W, ButtonRole, Control, FIELD_H, FOOTER_H, FooterButton, Kind,
    LIST_ROW_H, LineLook, MARGIN, ROW_H, Secret, Spec, TAB_H, TITLE_H, Value, Values, split_access,
};
