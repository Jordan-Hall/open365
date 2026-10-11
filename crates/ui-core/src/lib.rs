//! Retained widget tree for the Open365 UI (ADR `docs/architecture/adr-ui-platform.md`, phase 2).
//!
//! * [`tree`] - node arena, widget data, layout in logical units, hit testing.
//! * [`geom`] - logical geometry and per-scale edge snapping.
//! * [`Ui`] - focus and keyboard routing (Tab order, access keys), pointer capture, hover and
//!   press states, timers, tooltips, IME composition, the clipboard seam and Alt key tips.
//! * [`theme`] / [`settings`] - theme tokens (light, dark, high contrast) and OS settings read
//!   through the safe `windows-registry` crate.
//! * [`paint`] / [`access`] - draw calls for `open365-gfx` and the AccessKit tree.
//! * [`harness`] - headless rendering, simulated input and AccessKit queries for tests.
//! * [`chrome`] - the Excel Home tab row and File button built from the ui-shell spec JSON.

#![allow(clippy::collapsible_match)]

pub mod access;
pub mod chrome;
pub mod clipboard;
pub mod geom;
pub mod harness;
pub mod input;
pub mod keytips;
pub mod paint;
pub mod settings;
pub mod textedit;
pub mod theme;
pub mod tree;
mod ui;

pub use clipboard::{Clipboard, MemoryClipboard};
pub use geom::{DeviceRect, Point, Rect, Scale};
pub use input::{Emit, ImeInput, InputEvent, Key, Modifiers, PointerButton};
pub use keytips::{KeyTips, Tip, TipOutcome};
pub use paint::PaintLayers;
pub use settings::{OsLocale, PlatformSettings, os_locale};
pub use textedit::{Composition, TextEdit};
pub use theme::{Theme, ThemeKind, Token};
pub use tree::{
    Content, Extent, LIST_ROW_FONT_H, LIST_ROW_SIZE_H, Layout, Look, MENU_DETAIL_H, MENU_HEADING_H,
    MENU_ROW_H, MenuItem, MenuState, Node, NodeId, ScrollState, TabItem, Tree, Widget,
};
pub use ui::{TOOLTIP_DELAY, TooltipState, Ui};
