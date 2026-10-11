//! UI fonts: Segoe UI from the machine when it is installed (it is what Office draws its chrome
//! in), the bundled Carlito otherwise. Fonts are read from the system, never redistributed.

use std::sync::Arc;

use open365_text::{FontLibrary, FontStyle};

/// The UI font bytes, face index and symbol fallbacks.
#[derive(Debug, Clone)]
pub struct UiFonts {
    pub primary: Arc<[u8]>,
    pub index: u32,
    pub fallbacks: Vec<Arc<[u8]>>,
    /// True when the primary font is Segoe UI.
    pub segoe: bool,
}

pub fn load() -> UiFonts {
    let mut lib = FontLibrary::bundled();
    let segoe = lib.add_system_family("Segoe UI") > 0;
    lib.add_system_family("Segoe UI Symbol");
    let style = FontStyle::default();
    let family = if segoe { "Segoe UI" } else { "Carlito" };
    let (primary, index) = lib
        .resolve(family, style)
        .and_then(|id| lib.try_face(id))
        .map(|f| (Arc::from(f.bytes().to_vec()), f.index()))
        .unwrap_or_else(|| (Arc::from(Vec::new()), 0));
    let fallbacks = lib
        .resolve("Segoe UI Symbol", style)
        .and_then(|id| lib.try_face(id))
        .map(|f| vec![Arc::from(f.bytes().to_vec())])
        .unwrap_or_default();
    UiFonts {
        primary,
        index,
        fallbacks,
        segoe,
    }
}
