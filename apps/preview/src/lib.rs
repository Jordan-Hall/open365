//! Side-by-side preview of the Open365 chrome on the own UI platform.
//!
//! The two binaries, `open365-writer-next` and `open365-sheets-next`, open a window that draws
//! the title bar, Quick Access Toolbar, ribbon tabs and groups, Backstage, status bar and (for
//! Sheets) the Name Box, formula bar and sheet tabs with `open365-ui-core` widgets rendered by
//! `open365-gfx` on `open365-ui-host`'s winit window. Geometry and responsive rules come from the
//! `ui-shell` spec JSON and its pure layout code, which are compiled in from there unchanged
//! (read only); nothing here depends on egui.
//!
//! The document area is a placeholder page or grid.

// The pure parts of ui-shell, shared by path so the two shells cannot drift apart.
#[path = "../../../crates/ui-shell/src/chrome.rs"]
pub mod chrome;
pub mod clip;
#[path = "../../../crates/ui-shell/src/icon_table.rs"]
mod icon_table;
#[path = "../../../crates/ui-shell/src/layout.rs"]
pub mod layout;
#[path = "../../../crates/ui-shell/src/spec.rs"]
pub mod spec;

pub mod compare;
pub mod dialogs;
pub mod doc;
pub mod fonts;
pub mod headless;
pub mod icons;
pub mod menus;
pub mod palette;
pub mod print;
pub mod ribbon;
pub mod sheet;
pub use sheet::sheet_ext;
pub mod shell;

mod app;

pub use app::run;
pub use headless::{Preview, Snapshot};
pub use palette::Palette;
pub use shell::{Shell, WinAct};

/// Which application the preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    Writer,
    Sheets,
}
