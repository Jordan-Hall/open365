//! Office-style application chrome for egui: title row with Quick Access Toolbar and search,
//! ribbon with tabs and groups, Backstage, status bar and (for spreadsheets) formula bar and
//! sheet tabs.
//!
//! The chrome knows nothing about documents. A [`ShellHost`] answers command states and runs
//! command ids; the ribbon layout is data (`spec/*.json`) measured from the reference
//! applications (see `docs/architecture/app-shell.md`).

pub mod chrome;
pub mod fonts;
pub mod host;
mod icon_table;
pub mod icons;
pub mod layout;
pub mod menu;
mod shell;
pub mod spec;
pub mod theme;

pub use host::{CommandState, EntryKind, MenuEntry, ShellHost, Swatch, Tile};
pub use shell::{Shell, ShellOptions};
pub use theme::{AppKind, Mode, Theme};
