//! Sheets ribbon: the Automate tab (`automate.*`).
//!
//! Not wired, each needing a scripting subsystem the product does not have yet: New Script,
//! View Scripts and the Scripts Library gallery (`automate.scripts.new-script`,
//! `automate.scripts.view-scripts`, `automate.scripts-library.*`). Open365's own automation
//! surface is the command layer's CLI and JSON/MCP interface, not a script editor in the app.

use crate::ribbon::{Handled, RibbonCtx};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[];

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(_ctx: &mut RibbonCtx<'_>, _id: &str) -> Handled {
    Handled::No
}
