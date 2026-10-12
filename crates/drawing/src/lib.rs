//! Shared DrawingML layer (pure data, no I/O): the object model, the guide-formula
//! evaluator, paint, a display list in points, hit bounds and read-only fragment parsers.
//! Design: `docs/architecture/drawing.md`.

pub mod error;
pub mod geom;
pub mod hit;
pub mod model;
pub mod paint;
pub mod parse;
pub mod render;
pub mod scene;
pub mod text;
pub mod theme;
pub mod transform;
pub mod xml;

pub use error::ParseError;
pub use model::{DrawingObject, Kind, RawXml, Xfrm};
pub use parse::parse_fragment;
pub use render::{DisplayList, Prim, display_list};
pub use transform::Affine;
