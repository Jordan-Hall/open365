use std::path::PathBuf;
use std::sync::Arc;

use crate::{FontError, FontFace};

/// Index of a face inside a [`FontLibrary`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FaceId(usize);

impl FaceId {
    /// The face at `index` of its library. Callers ensure the index is in range.
    pub fn from_index(index: usize) -> Self {
        Self(index)
    }

    pub fn index(self) -> usize {
        self.0
    }
}

/// Requested weight and slant. Only regular/bold and upright/italic are modelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FontStyle {
    pub bold: bool,
    pub italic: bool,
}

/// Ordered faces to try for one run: the first covering a character shapes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontChain {
    pub faces: Vec<FaceId>,
}

/// How a requested family is satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontMatch {
    /// A face of that very family is loaded.
    Exact,
    /// A metric-compatible substitute stands in (Calibri -> Carlito).
    Alias,
    /// Nothing related is loaded; the default family is used.
    Fallback,
}

/// A set of faces plus the family substitution table.
#[derive(Debug)]
pub struct FontLibrary {
    faces: Vec<FontFace>,
    /// (lower-case requested family, installed family) pairs.
    aliases: Vec<(String, String)>,
    default_family: String,
}

macro_rules! bundled_font {
    ($file:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/",
            $file
        )) as &[u8]
    };
}

const BUNDLED: [(&str, bool, bool, &[u8]); 8] = [
    (
        "Carlito",
        false,
        false,
        bundled_font!("Carlito-Regular.ttf"),
    ),
    ("Carlito", true, false, bundled_font!("Carlito-Bold.ttf")),
    ("Carlito", false, true, bundled_font!("Carlito-Italic.ttf")),
    (
        "Carlito",
        true,
        true,
        bundled_font!("Carlito-BoldItalic.ttf"),
    ),
    (
        "Caladea",
        false,
        false,
        bundled_font!("Caladea-Regular.ttf"),
    ),
    ("Caladea", true, false, bundled_font!("Caladea-Bold.ttf")),
    ("Caladea", false, true, bundled_font!("Caladea-Italic.ttf")),
    (
        "Caladea",
        true,
        true,
        bundled_font!("Caladea-BoldItalic.ttf"),
    ),
];

/// The system face of `family` with the given style that is closest to regular or bold weight.
fn best_system_face(
    db: &fontdb::Database,
    family: &str,
    bold: bool,
    italic: bool,
) -> Option<fontdb::ID> {
    let want = family.to_lowercase();
    let want_compact = want.replace(' ', "");
    let target = if bold { 700 } else { 400 };
    db.faces()
        .filter(|f| (f.style != fontdb::Style::Normal) == italic)
        .filter_map(|f| {
            let by_name = f.families.iter().any(|(n, _)| n.to_lowercase() == want);
            // Face base name of the PostScript name: "Calibri-Light" -> "calibri".
            let base = f
                .post_script_name
                .split('-')
                .next()
                .unwrap_or_default()
                .to_lowercase();
            let by_ps = f
                .post_script_name
                .replace('-', "")
                .to_lowercase()
                .starts_with(&want_compact)
                && base != want_compact
                && want_compact.len() > base.len();
            // A bold slot takes only bold faces, a regular slot only regular ones.
            if !(by_name || by_ps) || bold != (f.weight.0 >= 600) {
                return None;
            }
            let mut score = u32::from(f.weight.0.abs_diff(target)) * 10;
            if base != want_compact {
                score += 1;
            }
            if f.stretch != fontdb::Stretch::Normal {
                score += 100_000;
            }
            Some((score, f.id))
        })
        .min_by_key(|(score, _)| *score)
        .map(|(_, id)| id)
}

impl FontLibrary {
    /// A library without faces.
    pub fn empty(default_family: &str) -> Self {
        Self {
            faces: Vec::new(),
            aliases: Vec::new(),
            default_family: default_family.to_owned(),
        }
    }

    /// The deterministic library: only the bundled OFL fonts Carlito and Caladea, which are
    /// metric-compatible with Calibri and Cambria, aliased under those names. Unknown
    /// families fall back to Carlito.
    pub fn bundled() -> Self {
        let mut lib = Self::empty("Carlito");
        for (family, bold, italic, bytes) in BUNDLED {
            // The bundled bytes are valid fonts (unit-tested); a failure would only drop a face.
            let _ = lib.add_bytes(Arc::from(bytes), 0, family, bold, italic);
        }
        lib.add_alias("Calibri", "Carlito");
        lib.add_alias("Cambria", "Caladea");
        lib
    }

    pub fn add_bytes(
        &mut self,
        data: Arc<[u8]>,
        index: u32,
        family: &str,
        bold: bool,
        italic: bool,
    ) -> Result<FaceId, FontError> {
        let face = FontFace::from_bytes(data, index, family, bold, italic)?;
        self.faces.push(face);
        Ok(FaceId(self.faces.len() - 1))
    }

    /// Resolve requests for `requested` to the installed family `installed`.
    pub fn add_alias(&mut self, requested: &str, installed: &str) {
        self.aliases
            .push((requested.to_lowercase(), installed.to_owned()));
    }

    /// Adds the system faces of `family` (case-insensitive) using fontdb. Returns how many
    /// faces were added. Use only outside deterministic tests.
    pub fn add_system_family(&mut self, family: &str) -> usize {
        self.add_system_families(&[family], &[])
    }

    /// Loads the system fonts plus the font directories `extra_dirs` once and adds, for each
    /// of `families`, the regular, bold, italic and bold italic face that matches best. The
    /// faces are registered under the requested family name, so a typographic subfamily such
    /// as "Calibri Light" or "Aptos Display" can be requested by its own name. Returns how
    /// many faces were added. Use only outside deterministic tests.
    pub fn add_system_families(&mut self, families: &[&str], extra_dirs: &[PathBuf]) -> usize {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        for dir in extra_dirs {
            db.load_fonts_dir(dir);
        }
        let mut added = 0;
        for family in families {
            for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
                let Some(id) = best_system_face(&db, family, bold, italic) else {
                    continue;
                };
                let loaded = db.with_face_data(id, |data, index| (Arc::<[u8]>::from(data), index));
                if let Some((data, index)) = loaded
                    && self.add_bytes(data, index, family, bold, italic).is_ok()
                {
                    added += 1;
                }
            }
        }
        added
    }

    /// How `family` would be satisfied by this library.
    pub fn match_kind(&self, family: &str) -> FontMatch {
        if self.best_in_family(family, FontStyle::default()).is_some() {
            return FontMatch::Exact;
        }
        let lower = family.to_lowercase();
        if self.aliases.iter().any(|(r, installed)| {
            *r == lower
                && self
                    .best_in_family(installed, FontStyle::default())
                    .is_some()
        }) {
            return FontMatch::Alias;
        }
        FontMatch::Fallback
    }

    pub fn face(&self, id: FaceId) -> &FontFace {
        &self.faces[id.0]
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    fn best_in_family(&self, family: &str, style: FontStyle) -> Option<FaceId> {
        let wanted = family.to_lowercase();
        self.faces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.family().to_lowercase() == wanted)
            .min_by_key(|(_, f)| {
                u8::from(f.is_bold() != style.bold) * 2 + u8::from(f.is_italic() != style.italic)
            })
            .map(|(i, _)| FaceId(i))
    }

    /// The face that best matches `family` and `style`: an installed face of that family, else
    /// the aliased family, else the default family, else any face. `None` only when the
    /// library is empty.
    pub fn resolve(&self, family: &str, style: FontStyle) -> Option<FaceId> {
        if let Some(id) = self.best_in_family(family, style) {
            return Some(id);
        }
        let lower = family.to_lowercase();
        if let Some((_, installed)) = self.aliases.iter().find(|(r, _)| *r == lower)
            && let Some(id) = self.best_in_family(installed, style)
        {
            return Some(id);
        }
        self.best_in_family(&self.default_family, style)
            .or_else(|| (!self.faces.is_empty()).then_some(FaceId(0)))
    }

    /// Fallback chain for a run: the resolved face, then the default family in the same
    /// style, then every other face (so any character some face covers is drawn).
    pub fn chain(&self, family: &str, style: FontStyle) -> FontChain {
        let mut faces = Vec::new();
        let mut push = |id: Option<FaceId>| {
            if let Some(id) = id
                && !faces.contains(&id)
            {
                faces.push(id);
            }
        };
        push(self.resolve(family, style));
        push(self.best_in_family(&self.default_family, style));
        for i in 0..self.faces.len() {
            push(Some(FaceId(i)));
        }
        FontChain { faces }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_faces_load_and_alias() {
        let lib = FontLibrary::bundled();
        assert_eq!(lib.len(), 8);
        let calibri = lib.resolve("Calibri", FontStyle::default()).unwrap();
        assert_eq!(lib.face(calibri).family(), "Carlito");
        let cambria = lib
            .resolve(
                "cambria",
                FontStyle {
                    bold: true,
                    italic: false,
                },
            )
            .unwrap();
        assert_eq!(lib.face(cambria).family(), "Caladea");
        assert!(lib.face(cambria).is_bold());
        let unknown = lib.resolve("No Such Font", FontStyle::default()).unwrap();
        assert_eq!(lib.face(unknown).family(), "Carlito");
    }

    #[test]
    fn calibri_metrics_match_the_real_font() {
        // Carlito is metric-compatible with Calibri: 2048 upem and a 2500-unit line (Calibri splits
        // it hhea 1536/512/452, Carlito stores it as 1950/550/0).
        let lib = FontLibrary::bundled();
        let id = lib.resolve("Calibri", FontStyle::default()).unwrap();
        let m = lib.face(id).metrics();
        assert_eq!(m.units_per_em, 2048);
        assert_eq!(m.hhea_ascent + m.hhea_descent + m.hhea_line_gap, 2500);
        assert_eq!((m.win_ascent, m.win_descent), (1950, 550));
    }
}
