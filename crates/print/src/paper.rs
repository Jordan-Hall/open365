//! Paper sizes and margin presets of the Print page, for both products.
//!
//! The sizes are the ones both products list (points, portrait). The margin presets are the
//! ones the products list in their Print and Page Layout menus.

/// Which product's presets apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    Writer,
    Sheets,
}

/// A paper size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paper {
    pub id: &'static str,
    pub label: &'static str,
    /// Portrait width and height in points.
    pub width_pt: f64,
    pub height_pt: f64,
    /// The IPP `media` name (PWG 5101.1) a CUPS queue understands.
    pub ipp_media: &'static str,
}

const fn p(
    id: &'static str,
    label: &'static str,
    width_pt: f64,
    height_pt: f64,
    ipp_media: &'static str,
) -> Paper {
    Paper {
        id,
        label,
        width_pt,
        height_pt,
        ipp_media,
    }
}

const PAPERS: [Paper; 10] = [
    p("letter", "Letter", 612.0, 792.0, "na_letter_8.5x11in"),
    p("tabloid", "Tabloid", 792.0, 1224.0, "na_ledger_11x17in"),
    p("legal", "Legal", 612.0, 1008.0, "na_legal_8.5x14in"),
    p(
        "statement",
        "Statement",
        396.0,
        612.0,
        "na_invoice_5.5x8.5in",
    ),
    p(
        "executive",
        "Executive",
        522.0,
        756.0,
        "na_executive_7.25x10.5in",
    ),
    p("a3", "A3", 841.9, 1190.55, "iso_a3_297x420mm"),
    p("a4", "A4", 595.3, 841.9, "iso_a4_210x297mm"),
    p("a5", "A5", 419.55, 595.3, "iso_a5_148x210mm"),
    p("b4", "B4 (JIS)", 728.5, 1031.8, "jis_b4_257x364mm"),
    p("b5", "B5 (JIS)", 515.9, 728.5, "jis_b5_182x257mm"),
];

/// Every paper size, in menu order.
#[must_use]
pub fn papers() -> &'static [Paper] {
    &PAPERS
}

/// The paper called `id`.
#[must_use]
pub fn paper_by_id(id: &str) -> Option<&'static Paper> {
    PAPERS.iter().find(|p| p.id == id)
}

/// The paper whose size is `w` by `h` points in either orientation, within 2 points.
#[must_use]
pub fn paper_matching(w: f64, h: f64) -> Option<&'static Paper> {
    let (w, h) = if w <= h { (w, h) } else { (h, w) };
    PAPERS
        .iter()
        .find(|p| (p.width_pt - w).abs() <= 2.0 && (p.height_pt - h).abs() <= 2.0)
}

/// A margin choice of the Print page's margins menu, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarginPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
    /// Mirrored margins (inside and outside), Writer only.
    pub mirrored: bool,
}

const fn m(
    id: &'static str,
    label: &'static str,
    top: f64,
    bottom: f64,
    left: f64,
    right: f64,
    mirrored: bool,
) -> MarginPreset {
    MarginPreset {
        id,
        label,
        top,
        bottom,
        left,
        right,
        mirrored,
    }
}

const WRITER_MARGINS: [MarginPreset; 5] = [
    m("normal", "Normal", 72.0, 72.0, 72.0, 72.0, false),
    m("narrow", "Narrow", 36.0, 36.0, 36.0, 36.0, false),
    m("moderate", "Moderate", 72.0, 72.0, 54.0, 54.0, false),
    m("wide", "Wide", 72.0, 72.0, 144.0, 144.0, false),
    m("mirrored", "Mirrored", 72.0, 72.0, 90.0, 72.0, true),
];

const SHEETS_MARGINS: [MarginPreset; 3] = [
    m("normal", "Normal", 54.0, 54.0, 50.4, 50.4, false),
    m("wide", "Wide", 72.0, 72.0, 72.0, 72.0, false),
    m("narrow", "Narrow", 54.0, 54.0, 18.0, 18.0, false),
];

/// The margin presets of `product`, in menu order.
#[must_use]
pub fn margin_presets(product: Product) -> &'static [MarginPreset] {
    match product {
        Product::Writer => &WRITER_MARGINS,
        Product::Sheets => &SHEETS_MARGINS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_paper_in_either_orientation() {
        assert_eq!(paper_matching(841.9, 595.3).map(|p| p.id), Some("a4"));
        assert_eq!(paper_matching(612.0, 792.0).map(|p| p.id), Some("letter"));
        assert!(paper_matching(500.0, 500.0).is_none());
    }

    #[test]
    fn every_product_starts_with_normal() {
        for product in [Product::Writer, Product::Sheets] {
            assert_eq!(margin_presets(product)[0].id, "normal");
        }
    }
}
