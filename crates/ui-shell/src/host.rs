//! What the shell asks of the application that hosts it.

use egui::Ui;

/// Whether a command can run and whether it is on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandState {
    pub enabled: bool,
    pub checked: bool,
}

impl CommandState {
    pub const DISABLED: Self = Self {
        enabled: false,
        checked: false,
    };
    pub const ENABLED: Self = Self {
        enabled: true,
        checked: false,
    };

    #[must_use]
    pub fn checked(on: bool) -> Self {
        Self {
            enabled: true,
            checked: on,
        }
    }
}

/// One colour cell of a swatch grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swatch {
    /// Control id the host runs when the cell is chosen.
    pub id: String,
    /// Tool tip (the colour's name).
    pub name: String,
    pub rgb: [u8; 3],
    pub checked: bool,
}

impl Swatch {
    #[must_use]
    pub fn new(id: &str, name: &str, rgb: [u8; 3]) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            rgb,
            checked: false,
        }
    }
}

/// A text tile of a tile grid (a style gallery).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub id: String,
    pub label: String,
    /// Background of the tile; `None` draws the field colour.
    pub fill: Option<[u8; 3]>,
    /// Text colour on a filled tile.
    pub ink: Option<[u8; 3]>,
    pub checked: bool,
    /// Edges of the tile's sample cell, `[left, right, top, bottom]`: colour and width in `u`.
    pub borders: [Option<([u8; 3], u8)>; 4],
    /// The style's font size relative to the 11 pt Normal font, in percent (100 = same size).
    pub text_pct: u32,
    pub bold: bool,
    pub italic: bool,
    /// The style's real font face; `None` draws the UI font.
    pub font: Option<TileFont>,
}

/// The face a tile's sample text is drawn with: an egui family holding the real font (or its metric-compatible
/// substitute) in the style's weight and slant, plus what the family could not supply and must be faked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileFont {
    pub family: egui::FontFamily,
    /// Bold is wanted but the family has no bold face: draw the text twice, one device pixel apart.
    pub fake_bold: bool,
    /// Italic is wanted but the family has no italic face: slant the glyphs.
    pub fake_italic: bool,
}

impl Tile {
    /// A plain tile: field colour, no borders, label size.
    #[must_use]
    pub fn plain(id: &str, label: &str, checked: bool) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            fill: None,
            ink: None,
            checked,
            borders: [None; 4],
            text_pct: 100,
            bold: false,
            italic: false,
            font: None,
        }
    }
}

/// What a menu line is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EntryKind {
    /// A command line (with a check mark when `checked`, a sub-menu when it has `children`).
    #[default]
    Item,
    /// A thin rule between groups.
    Separator,
    /// A group name.
    Heading,
    /// Rows of colour cells: the theme tint grid or the standard colours. `tight` rows touch (tints).
    Swatches { rows: Vec<Vec<Swatch>>, tight: bool },
    /// Excel's "Gallery Filters" strip (52 u) above a colour picker, with its "High-contrast only"
    /// switch (`label`). While the switch is on, colours with a contrast below 3:1 against the
    /// menu background are left out.
    Filter,
    /// Text tiles in `columns` columns.
    Tiles {
        columns: usize,
        /// Tile width and height in `u`.
        size: (u32, u32),
        tiles: Vec<Tile>,
    },
}

/// One line of a drop-down menu.
#[derive(Debug, Clone, Default)]
pub struct MenuEntry {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub kind: EntryKind,
    pub checked: bool,
    /// A sub-menu opens beside the line when this is not empty.
    pub children: Vec<MenuEntry>,
    /// Second line under the label (a sample in the Number Format list).
    pub detail: Option<String>,
    /// Icon name (Tabler) drawn in the left gutter.
    pub icon: Option<String>,
    /// Right-aligned shortcut text.
    pub shortcut: Option<String>,
    /// Row height in `u` when it is not the menu default (list rows of a combo).
    pub height: Option<u32>,
    /// Row text is centred (the font size list).
    pub centred: bool,
}

impl MenuEntry {
    #[must_use]
    pub fn new(id: &str, label: &str, enabled: bool) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            enabled,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn separator() -> Self {
        Self {
            kind: EntryKind::Separator,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn heading(label: &str) -> Self {
        Self {
            label: label.to_owned(),
            kind: EntryKind::Heading,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn filter(label: &str) -> Self {
        Self {
            label: label.to_owned(),
            enabled: true,
            kind: EntryKind::Filter,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn swatches(rows: Vec<Vec<Swatch>>, tight: bool) -> Self {
        Self {
            enabled: true,
            kind: EntryKind::Swatches { rows, tight },
            ..Self::default()
        }
    }

    #[must_use]
    pub fn tiles(columns: usize, size: (u32, u32), tiles: Vec<Tile>) -> Self {
        Self {
            enabled: true,
            kind: EntryKind::Tiles {
                columns,
                size,
                tiles,
            },
            ..Self::default()
        }
    }

    #[must_use]
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }

    #[must_use]
    pub fn submenu(mut self, children: Vec<MenuEntry>) -> Self {
        self.children = children;
        self
    }

    #[must_use]
    pub fn detail(mut self, text: &str) -> Self {
        self.detail = Some(text.to_owned());
        self
    }

    #[must_use]
    pub fn icon(mut self, name: &str) -> Self {
        self.icon = Some(name.to_owned());
        self
    }
}

/// The application behind the chrome. Commands are data: the shell passes control ids; the host
/// maps each id to its command layer. An id the host does not know stays disabled.
pub trait ShellHost {
    /// State of command `id`. Unwired commands answer [`CommandState::DISABLED`].
    fn state(&self, id: &str) -> CommandState;
    /// Runs command `id`. `value` carries a number for sliders (zoom).
    fn run(&mut self, id: &str, value: Option<f64>);
    /// The document name for the title bar.
    fn title(&self) -> String;
    /// Text of a status-bar item (`status.page`, `status.words`...), when the host has one.
    fn status_text(&self, id: &str) -> Option<String>;
    /// Zoom as a fraction (1.0 is 100 %).
    fn zoom(&self) -> f32 {
        1.0
    }
    /// The drop-down menu of control `id`, when it has one.
    fn menu(&self, _id: &str) -> Option<Vec<MenuEntry>> {
        None
    }
    /// The user finished typing `text` into the editable combo `id` (Enter, or a list pick). Return the control id to
    /// run (for instance `fmt.font-name#Arial`), or `None` to refuse the text.
    fn combo_commit(&mut self, _id: &str, _text: &str) -> Option<String> {
        None
    }
    /// What a combo box or gallery shows for control `id`.
    fn value(&self, _id: &str) -> Option<String> {
        None
    }
    /// Gallery tiles: (value id, label).
    fn gallery(&self, _id: &str) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Recent files for the Backstage view.
    fn recent(&self) -> Vec<String> {
        Vec::new()
    }
    /// Property lines for Backstage Info.
    fn info(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Draws the Name Box editor into `ui` (a child of the Name Box rectangle). Return false to let
    /// the shell show the static `value("formula.name")`.
    fn name_box_ui(&mut self, _ui: &mut Ui) -> bool {
        false
    }
    /// Draws the formula text editor into `ui`. Return false to show `value("formula.text")`.
    fn formula_ui(&mut self, _ui: &mut Ui) -> bool {
        false
    }
    /// A menu, list, group pop-up, combo editor or the Backstage view just closed: hand the keyboard back to the document.
    fn chrome_closed(&mut self) {}
    /// Draws the document area.
    fn central(&mut self, ui: &mut Ui);
}
