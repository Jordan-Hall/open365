//! Slot for the generated preset table.
//!
//! This file is intentionally empty. A generator will replace [`table`] once the licence
//! of the source data is settled (see `docs/architecture/drawing.md`). Until then every
//! preset except the hand-written `rect` falls back to a rectangle with a diagnostic.

use super::ShapeDef;

/// Generated presets `(name, definition)`. Empty until the generator lands.
pub fn table() -> Vec<(&'static str, ShapeDef)> {
    Vec::new()
}
