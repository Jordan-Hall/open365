//! Input events fed to [`crate::Ui`] and events it emits. Hosts translate their native events
//! (winit and the headless harness both do) into these.

use std::time::Instant;

use crate::geom::Point;
use crate::tree::NodeId;

/// Keys the widgets react to. Letters and digits arrive as `Char` (upper case folded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Tab,
    Enter,
    Escape,
    Space,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F2,
    F10,
    Alt,
    Shift,
    Ctrl,
    Char(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Primary,
    Secondary,
}

/// IME composition events (`Preedit` carries the cursor as byte offsets into the text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeInput {
    Enabled,
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    Commit(String),
    Disabled,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    PointerMoved(Point),
    PointerLeft,
    PointerDown(PointerButton, Point),
    PointerUp(PointerButton, Point),
    Wheel {
        delta_y: f32,
    },
    Key {
        key: Key,
        pressed: bool,
        modifiers: Modifiers,
    },
    /// Typed text (after the keyboard layout), not shortcuts.
    Text(String),
    Ime(ImeInput),
    /// Time moved on; fires timers.
    Tick(Instant),
    FocusChanged(bool),
}

/// What a widget reports to the application.
#[derive(Debug, Clone, PartialEq)]
pub enum Emit {
    Clicked(NodeId),
    /// The split button's arrow part, or a dropdown, asked to open its menu.
    MenuRequested(NodeId),
    Toggled(NodeId, bool),
    Selected(NodeId, usize),
    TextChanged(NodeId, String),
    /// Enter pressed in a text field or combo.
    TextCommitted(NodeId, String),
    MenuCommand(NodeId, String),
    ScrollChanged(NodeId, f32),
    ValueChanged(NodeId, f64),
    TabSelected(NodeId, usize),
    /// A key tip ended on this node.
    KeyTip(NodeId),
    /// A key the focused node does not handle itself (`Node::intercept`, or any key on a canvas).
    KeyPressed(NodeId, Key, Modifiers),
    /// Text typed while a canvas had focus.
    CanvasText(NodeId, String),
    /// IME composition (preedit or commit) while a canvas had focus.
    CanvasIme(NodeId, ImeInput),
    /// A primary press on a canvas: `at` is relative to the canvas origin (logical units);
    /// `clicks` counts presses within the double-click time and distance.
    CanvasPointer {
        node: NodeId,
        at: Point,
        clicks: u8,
        shift: bool,
    },
}
