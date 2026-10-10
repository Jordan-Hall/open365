//! Test support: fixture manifests, round-trip invariants and external oracles.

mod inspect;
mod roundtrip;

pub use inspect::{Inspection, PartInfo, inspect};
pub use roundtrip::{RoundtripReport, roundtrip};
