//! wgpu 2D renderer for the Open365 UI (ADR `docs/architecture/adr-ui-platform.md`).
//!
//! The crate is split so almost all of it runs without a GPU:
//!
//! * [`snap`] - device-pixel snapping maths for rects, strokes and glyph positions.
//! * [`DrawList`] - tessellates rects, rounded rects, lines, images, icons and glyph runs into
//!   vertices and clip-scoped batches (device-pixel coordinates).
//! * [`Assets`] - fonts, glyph/icon/image atlases. Glyphs come from skrifa outlines (hinted),
//!   rasterised by the exact-area rasteriser in [`raster`] at 1/N pixel x bins, in grayscale or
//!   ClearType-style LCD.
//! * [`Renderer`] / [`Offscreen`] - the wgpu side: gamma-correct blending in an sRGB target,
//!   and render-to-texture with readback for headless tests.

pub mod assets;
pub mod atlas;
pub mod color;
pub mod draw;
pub mod error;
pub mod geom;
pub mod path;
pub mod raster;
pub mod render;
pub mod snap;
pub mod text;

pub use assets::{Assets, FontId, GlyphPos, GlyphRun, Hinting, IconId, ImageId, TextOptions};
pub use color::Color;
pub use draw::{Batch, ClipMask, Decorations, DrawList, LinearGradient, Vertex};
pub use error::GfxError;
pub use geom::{PixelRect, Rect};
pub use path::{Cap, Join, Path, PathCmd};
pub use render::{
    GpuContext, OFFSCREEN_FORMAT, Offscreen, RenderParams, Renderer, choose_surface_format,
    pixel_at,
};
pub use text::{FontMetricsPx, TextLayout, TextLayoutOptions, TextRun};
