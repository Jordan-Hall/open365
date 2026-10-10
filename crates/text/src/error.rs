/// Why a font could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("not a usable font file: {0}")]
    Parse(String),
    #[error("font has no {0} table")]
    MissingTable(&'static str),
    #[error("font reports zero units per em")]
    ZeroUnitsPerEm,
}
