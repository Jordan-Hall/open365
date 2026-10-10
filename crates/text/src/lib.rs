//! Fonts, shaping, metrics and line-break opportunities for the layout engine (layer 3).
//!
//! * [`FontFace`] owns one font's bytes and answers metric and glyph questions.
//! * [`FontLibrary`] holds faces, resolves a requested family to a face (with the bundled
//!   metric-compatible substitutes Calibri -> Carlito, Cambria -> Caladea) and builds a
//!   fallback chain. [`FontLibrary::bundled`] is deterministic and uses no system fonts, so
//!   layout results do not depend on the machine.
//! * [`shape_run`] shapes text with HarfRust (a Rust port of HarfBuzz) through a fallback chain.
//! * [`break_opportunities`] gives UAX #14 line-break opportunities.

mod error;
mod face;
mod library;
mod linebreak;
mod shape;

pub use error::FontError;
pub use face::{FaceMetrics, FontFace};
pub use library::{FaceId, FontChain, FontLibrary, FontMatch, FontStyle};
pub use linebreak::{BreakKind, BreakOpportunity, break_opportunities};
pub use shape::{ShapeFeatures, ShapedGlyph, ShapedRun, shape_run};
