//! Font selection for the document view.
//!
//! A document names fonts such as Calibri or Cambria. To match Word's output the exact font
//! is used whenever the operating system has it installed (system fonts are discovered with
//! `fontdb` and loaded at run time; nothing proprietary is bundled). Only when a font is not
//! installed does the view fall back to the metric-compatible OFL fonts bundled under
//! `assets/fonts` (Carlito for Calibri and other sans fonts, Caladea for Cambria and other
//! serif fonts) and report the substitution, as Word does.
//!
//! egui binds fonts to families by name, so every (font, bold, italic) combination the
//! document uses becomes its own egui family. Font definitions only reach egui at the start
//! of the next pass; until then [`FontBook::resolve`] answers with the base family.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};

use egui::{Context, FontData, FontDefinitions, FontFamily};

macro_rules! bundled {
    ($file:literal) => {
        include_bytes!(concat!("../../../assets/fonts/", $file)) as &'static [u8]
    };
}

/// (data name, bytes) of the bundled faces: Carlito and Caladea, each in four styles.
const BUNDLED: [(&str, &[u8]); 8] = [
    ("Carlito-Regular", bundled!("Carlito-Regular.ttf")),
    ("Carlito-Bold", bundled!("Carlito-Bold.ttf")),
    ("Carlito-Italic", bundled!("Carlito-Italic.ttf")),
    ("Carlito-BoldItalic", bundled!("Carlito-BoldItalic.ttf")),
    ("Caladea-Regular", bundled!("Caladea-Regular.ttf")),
    ("Caladea-Bold", bundled!("Caladea-Bold.ttf")),
    ("Caladea-Italic", bundled!("Caladea-Italic.ttf")),
    ("Caladea-BoldItalic", bundled!("Caladea-BoldItalic.ttf")),
];

/// Largest system font file loaded; bigger ones (some CJK collections) are skipped.
const MAX_FONT_BYTES: usize = 48 * 1024 * 1024;

/// Fonts whose substitute is a serif face.
const SERIF_HINTS: &[&str] = &[
    "cambria",
    "times",
    "georgia",
    "garamond",
    "palatino",
    "book antiqua",
    "bookman",
    "century",
    "constantia",
    "baskerville",
    "serif",
    "minion",
    "didot",
    "caladea",
    "song",
    "mincho",
];

/// Which bundled family stands in for a font that is not installed.
fn bundled_family_for(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.contains("sans") {
        return "Carlito";
    }
    if SERIF_HINTS.iter().any(|h| lower.contains(h)) {
        "Caladea"
    } else {
        "Carlito"
    }
}

fn bundled_data_name(family: &str, bold: bool, italic: bool) -> String {
    let style = match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "BoldItalic",
    };
    format!("{family}-{style}")
}

/// The installed fonts, scanned once per process on first use.
fn system_fonts() -> Option<&'static fontdb::Database> {
    static DB: OnceLock<Option<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        if std::env::var_os("OPEN365_BUNDLED_FONTS_ONLY").is_some() {
            return None;
        }
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Some(db)
    })
    .as_ref()
}

type FaceKey = (String, bool, bool);

/// How a (font, bold, italic) request is drawn.
#[derive(Debug, Clone)]
pub struct Face {
    pub family: FontFamily,
    /// The face itself is bold / italic; when false, bold is faked by overstrike and italic
    /// by egui's slant.
    pub real_bold: bool,
    pub real_italic: bool,
}

/// Font definitions for egui plus the faces resolved for the open document.
#[derive(Debug)]
pub struct FontBook {
    defs: FontDefinitions,
    faces: HashMap<FaceKey, Face>,
    base: Face,
    /// Requested font name to the bundled font used instead.
    substituted: BTreeMap<String, &'static str>,
    defs_gen: u64,
    set_gen: u64,
    set_pass: u64,
    applied_gen: u64,
}

impl Default for FontBook {
    fn default() -> Self {
        Self::new()
    }
}

impl FontBook {
    /// Base definitions only: the UI family is Carlito Regular.
    pub fn new() -> Self {
        let mut defs = FontDefinitions::empty();
        let (name, bytes) = BUNDLED[0];
        defs.font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            defs.families.insert(family, vec![name.to_owned()]);
        }
        Self {
            defs,
            faces: HashMap::new(),
            base: Face {
                family: FontFamily::Proportional,
                real_bold: false,
                real_italic: false,
            },
            substituted: BTreeMap::new(),
            defs_gen: 1,
            set_gen: 0,
            set_pass: 0,
            applied_gen: 0,
        }
    }

    /// Pushes new definitions to egui and notes when they have taken effect. Call once per
    /// frame before laying out any text.
    pub fn sync(&mut self, ctx: &Context) {
        if self.set_gen != self.defs_gen {
            ctx.set_fonts(self.defs.clone());
            self.set_gen = self.defs_gen;
            self.set_pass = ctx.cumulative_pass_nr();
            ctx.request_repaint();
        } else if self.applied_gen != self.set_gen && ctx.cumulative_pass_nr() > self.set_pass {
            self.applied_gen = self.set_gen;
            ctx.request_repaint();
        }
    }

    /// True when egui uses the latest definitions.
    pub fn is_ready(&self) -> bool {
        self.applied_gen == self.defs_gen
    }

    /// Changes whenever the set of usable families changes; caches keyed on text layout must
    /// be dropped when it does.
    pub fn generation(&self) -> u64 {
        self.applied_gen
    }

    /// Fonts the document asked for that are not installed, with what is drawn instead.
    pub fn substitutions(&self) -> Vec<(String, String)> {
        self.substituted
            .iter()
            .map(|(from, to)| (from.clone(), (*to).to_owned()))
            .collect()
    }

    /// The face to draw `name` with. Faces are registered on first use and become usable one
    /// pass later; until then the base family answers (with faked bold and italic).
    pub fn resolve(&mut self, name: &str, bold: bool, italic: bool) -> Face {
        let key = (name.to_ascii_lowercase(), bold, italic);
        if let Some(face) = self.faces.get(&key) {
            return if self.is_ready() {
                face.clone()
            } else {
                self.base.clone()
            };
        }
        let face = self.register(name, bold, italic, &key);
        self.faces.insert(key, face);
        self.defs_gen += 1;
        self.base.clone()
    }

    fn add_bundled(&mut self, family: &str, bold: bool, italic: bool) -> String {
        let name = bundled_data_name(family, bold, italic);
        if !self.defs.font_data.contains_key(&name)
            && let Some((_, bytes)) = BUNDLED.iter().find(|(n, _)| *n == name)
        {
            self.defs
                .font_data
                .insert(name.clone(), Arc::new(FontData::from_static(bytes)));
        }
        name
    }

    fn register(&mut self, name: &str, bold: bool, italic: bool, key: &FaceKey) -> Face {
        let family = FontFamily::Name(
            format!("doc:{}:{}{}", key.0, u8::from(bold), u8::from(italic)).into(),
        );
        // The bundled face of the same style always backs the primary for missing glyphs.
        let fallback_family = bundled_family_for(name);
        let fallback = self.add_bundled(fallback_family, bold, italic);

        if let Some((data_name, data, real_bold, real_italic)) =
            system_face(name, bold, italic, &self.defs)
        {
            self.defs.font_data.entry(data_name.clone()).or_insert(data);
            self.defs
                .families
                .insert(family.clone(), vec![data_name, fallback]);
            return Face {
                family,
                real_bold,
                real_italic,
            };
        }

        // Not installed. Asking for the bundled family by its own name is no substitution.
        if !name.eq_ignore_ascii_case(fallback_family) {
            self.substituted.insert(name.to_owned(), fallback_family);
        }
        self.defs.families.insert(family.clone(), vec![fallback]);
        Face {
            family,
            real_bold: bold,
            real_italic: italic,
        }
    }
}

/// Looks `name` up among the installed fonts: case-insensitive family match, then the best
/// weight and slant. Returns the data name, the data and whether bold/italic are real.
fn system_face(
    name: &str,
    bold: bool,
    italic: bool,
    defs: &FontDefinitions,
) -> Option<(String, Arc<FontData>, bool, bool)> {
    let db = system_fonts()?;
    let wanted_weight: i32 = if bold { 700 } else { 400 };
    let best = db
        .faces()
        .filter(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)))
        .filter(|f| f.stretch == fontdb::Stretch::Normal)
        .min_by_key(|f| {
            let slant = i32::from((f.style != fontdb::Style::Normal) != italic);
            let weight = (i32::from(f.weight.0) - wanted_weight).abs();
            (slant, weight)
        })?;
    let data_name = format!(
        "sys:{}:{}:{}",
        name.to_ascii_lowercase(),
        best.weight.0,
        best.style != fontdb::Style::Normal
    );
    let real_bold = best.weight.0 >= 600;
    let real_italic = best.style != fontdb::Style::Normal;
    if defs.font_data.contains_key(&data_name) {
        let data = Arc::clone(&defs.font_data[&data_name]);
        return Some((data_name, data, real_bold, real_italic));
    }
    let (bytes, index) = db.with_face_data(best.id, |data, index| (data.to_vec(), index))?;
    if bytes.len() > MAX_FONT_BYTES {
        return None;
    }
    let data = FontData {
        font: Cow::Owned(bytes),
        index,
        tweak: egui::FontTweak::default(),
    };
    Some((data_name, Arc::new(data), real_bold, real_italic))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serif_and_sans_substitutes() {
        assert_eq!(bundled_family_for("Cambria"), "Caladea");
        assert_eq!(bundled_family_for("Times New Roman"), "Caladea");
        assert_eq!(bundled_family_for("Calibri"), "Carlito");
        assert_eq!(bundled_family_for("Segoe UI"), "Carlito");
        assert_eq!(bundled_family_for("Noto Sans"), "Carlito");
        assert_eq!(
            bundled_data_name("Carlito", true, true),
            "Carlito-BoldItalic"
        );
    }

    #[test]
    fn missing_fonts_fall_back_and_are_reported() {
        let mut book = FontBook::new();
        let face = book.resolve("Open365 Test Font That Does Not Exist", true, false);
        // Not applied yet: the base family answers.
        assert_eq!(face.family, FontFamily::Proportional);
        assert!(!book.is_ready());
        assert_eq!(
            book.substitutions(),
            [(
                "Open365 Test Font That Does Not Exist".to_owned(),
                "Carlito".to_owned()
            )]
        );
        assert!(book.defs.font_data.contains_key("Carlito-Bold"));
    }

    #[test]
    fn asking_for_the_bundled_family_is_not_a_substitution() {
        let mut book = FontBook::new();
        book.resolve("Carlito", false, false);
        assert!(book.substitutions().is_empty());
    }
}
