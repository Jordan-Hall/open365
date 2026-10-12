//! Sheets ribbon: the Draw tab (`draw.*`). Stub: inking needs pen input and an ink layer on the grid; every id answers `Handled::No`.

use crate::ribbon::{Handled, RibbonCtx};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[];

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(_ctx: &mut RibbonCtx<'_>, _id: &str) -> Handled {
    Handled::No
}
