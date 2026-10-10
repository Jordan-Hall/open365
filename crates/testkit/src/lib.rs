//! Test support: fixture manifests, round-trip invariants and external oracles.

mod hidden;
mod inspect;
mod roundtrip;
mod xmlscan;

pub use hidden::{HiddenFinding, inspect_hidden};
pub use inspect::{Inspection, PartInfo, RelInfo, inspect};
pub use roundtrip::{RoundtripReport, roundtrip};
