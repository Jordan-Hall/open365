//! Ribbon command handling, one module per application and tab.
//!
//! `shell.rs` only routes: [`dispatch`] takes the first dotted segment of a control id (`home` in
//! `home.font.bold`, `page-layout` in `page-layout.themes.themes`) and calls that tab's module.
//! Ids can carry a suffix from the widget: `id=text` (a typed combo value) or `id#3` (the 4th
//! list item); the module sees the full string, [`base`] strips the suffix.
//!
//! # How to add a command
//!
//! 1. Open the module of the app and tab: `ribbon/writer/<tab>.rs` or `ribbon/sheets/<tab>.rs`
//!    (`page_layout.rs` is `page-layout.*`). Nobody else needs to edit any other file.
//! 2. In `handle(ctx, id)`, match the id and do the work through [`RibbonCtx`]: `ctx.writer` (the
//!    Writer pane), `ctx.sheet` (the sheet model), `ctx.ui` (the widget tree), `ctx.out` (window
//!    requests), and set `*ctx.refocus_doc = true` when focus should return to the document.
//!    Return [`Handled::Yes`] for every id the module knows, also when there is no document.
//! 3. Add the plain spec id (no `=` or `#` suffix) to the module's `IDS`. The `ribbon_dispatch`
//!    test fails if two modules claim one id, if an id does not start with the module's tab, or
//!    if a handler does not answer [`Handled::Yes`] for an id it lists. It also prints the
//!    coverage per tab (`cargo test -p open365-preview --test ribbon_dispatch -- --nocapture`).
//! 4. Need another field in [`RibbonCtx`]? Add it here and fill it in `Shell::run` (rare; keep
//!    the context small).

use open365_ui_core::Ui;

use crate::sheet::SheetModel;
use crate::shell::WinAct;
use crate::{AppKind, doc};

pub mod sheets;
pub mod writer;

/// Whether a module knew the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    Yes,
    No,
}

/// What a command handler may touch.
pub struct RibbonCtx<'a> {
    pub kind: AppKind,
    pub ui: &'a mut Ui,
    /// The Writer document (Writer only; `None` shows the placeholder page).
    pub writer: Option<&'a mut doc::WriterPane>,
    pub sheet: &'a mut SheetModel,
    /// Set to return keyboard focus to the document after the command.
    pub refocus_doc: &'a mut bool,
    /// Window requests (title, minimise, close) the command produces.
    pub out: &'a mut Vec<WinAct>,
}

impl std::fmt::Debug for RibbonCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RibbonCtx")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// One tab's handler and the ids it claims.
#[derive(Clone, Copy)]
pub struct Module {
    /// The first dotted segment of the ids (`home`, `page-layout`).
    pub prefix: &'static str,
    pub ids: &'static [&'static str],
    pub handle: fn(&mut RibbonCtx<'_>, &str) -> Handled,
}

macro_rules! module {
    ($app:ident, $prefix:literal, $m:ident) => {
        Module {
            prefix: $prefix,
            ids: $app::$m::IDS,
            handle: $app::$m::handle,
        }
    };
}

const WRITER: &[Module] = &[
    module!(writer, "file", file),
    module!(writer, "home", home),
    module!(writer, "insert", insert),
    module!(writer, "draw", draw),
    module!(writer, "design", design),
    module!(writer, "layout", layout),
    module!(writer, "references", references),
    module!(writer, "mailings", mailings),
    module!(writer, "review", review),
    module!(writer, "view", view),
    module!(writer, "help", help),
];

const SHEETS: &[Module] = &[
    module!(sheets, "file", file),
    module!(sheets, "home", home),
    module!(sheets, "insert", insert),
    module!(sheets, "draw", draw),
    module!(sheets, "page-layout", page_layout),
    module!(sheets, "formulas", formulas),
    module!(sheets, "data", data),
    module!(sheets, "review", review),
    module!(sheets, "view", view),
    module!(sheets, "automate", automate),
    module!(sheets, "help", help),
];

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Module")
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

/// The modules of one application.
#[must_use]
pub fn modules(kind: AppKind) -> &'static [Module] {
    match kind {
        AppKind::Writer => WRITER,
        AppKind::Sheets => SHEETS,
    }
}

/// `id` without the widget suffix: `home.font.font=Arial` and `home.font.font#2` give
/// `home.font.font`.
#[must_use]
pub fn base(id: &str) -> &str {
    id.split(['=', '#']).next().unwrap_or(id)
}

/// Routes `id` to the module of its tab.
pub fn dispatch(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let prefix = base(id).split('.').next().unwrap_or_default();
    match modules(ctx.kind).iter().find(|m| m.prefix == prefix) {
        Some(m) => (m.handle)(ctx, id),
        None => Handled::No,
    }
}
