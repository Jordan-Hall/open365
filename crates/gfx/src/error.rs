use thiserror::Error;

/// Everything the renderer can fail at. CPU-side failures (bad font, bad SVG, atlas full) are
/// separate from GPU setup failures so tests can run without an adapter.
#[derive(Debug, Error)]
pub enum GfxError {
    #[error("no suitable GPU adapter: {0}")]
    NoAdapter(String),
    #[error("GPU device request failed: {0}")]
    Device(String),
    #[error("surface error: {0}")]
    Surface(String),
    #[error("font data rejected: {0}")]
    Font(String),
    #[error("SVG rejected: {0}")]
    Svg(String),
    #[error("image rejected: {0}")]
    Image(String),
    #[error("atlas full: cannot place a {width}x{height} item")]
    AtlasFull { width: u32, height: u32 },
    #[error("readback failed: {0}")]
    Readback(String),
}
