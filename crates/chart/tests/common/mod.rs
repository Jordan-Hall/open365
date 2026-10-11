//! Shared helpers: repository paths, the fixture workbooks and a Carlito-based text measure
//! (Carlito is metric-compatible with Calibri, Excel's default chart font).

#![allow(dead_code)]

use std::path::PathBuf;

use open365_chart::{FontSpec, TextMeasure};
use open365_opc::Limits;
use open365_text::{FontLibrary, FontStyle, ShapeFeatures, shape_run};
use open365_xlsx::Workbook;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn fixture(name: &str) -> Vec<u8> {
    let path = repo_root()
        .join("fixtures/xlsx")
        .join(format!("{name}.xlsx"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub fn open(name: &str) -> Workbook {
    Workbook::open(&fixture(name), &Limits::default()).expect("fixture opens")
}

pub fn reference(name: &str, file: &str) -> serde_json::Value {
    let path = repo_root()
        .join("fixtures/reference/xlsx")
        .join(name)
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("valid JSON")
}

pub struct CarlitoMeasure {
    lib: FontLibrary,
}

impl CarlitoMeasure {
    pub fn new() -> Self {
        Self {
            lib: FontLibrary::bundled(),
        }
    }
}

impl TextMeasure for CarlitoMeasure {
    fn width(&self, text: &str, font: &FontSpec) -> f64 {
        let style = FontStyle {
            bold: font.bold,
            italic: font.italic,
        };
        let chain = self.lib.chain(&font.family, style);
        let run = shape_run(&self.lib, &chain, text, ShapeFeatures::default());
        f64::from(run.width_pt(&self.lib, font.size_pt as f32))
    }

    fn line_height(&self, font: &FontSpec) -> f64 {
        let style = FontStyle {
            bold: font.bold,
            italic: font.italic,
        };
        let Some(id) = self.lib.resolve(&font.family, style) else {
            return font.size_pt * 1.2207;
        };
        let m = self.lib.face(id).metrics();
        let em = f64::from(m.units_per_em);
        font.size_pt * f64::from(m.hhea_ascent + m.hhea_descent + m.hhea_line_gap) / em
    }
}

/// A generated corpus recorded from Excel by `tools/office/make-chart-corpus.ps1`; `None` when
/// the file has not been recorded yet.
pub fn scale_corpus(file: &str) -> Option<Vec<serde_json::Value>> {
    let path = repo_root()
        .join("fixtures/reference/xlsx/chart-scale")
        .join(file);
    let text = std::fs::read_to_string(&path).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    Some(doc["cases"].as_array().expect("cases").clone())
}

pub fn f64s(v: &serde_json::Value) -> Vec<f64> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|x| x.as_f64().expect("number"))
        .collect()
}
