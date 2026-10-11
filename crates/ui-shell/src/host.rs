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

/// One line of a drop-down menu.
#[derive(Debug, Clone)]
pub struct MenuEntry {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

impl MenuEntry {
    #[must_use]
    pub fn new(id: &str, label: &str, enabled: bool) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            enabled,
        }
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
    /// Draws the document area.
    fn central(&mut self, ui: &mut Ui);
}
