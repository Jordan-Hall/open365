//! Developer tool: `export <in.docx> <out.pdf> [--pdfa 1b|2b] [--pdfua]`, for running an
//! external validator (veraPDF) over our output. Bundled fonts only.
use std::sync::atomic::AtomicBool;

use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_pdf::{Conformance, PdfMetadata, PdfOptions, export_pdf_with};
use open365_text::FontLibrary;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bytes = std::fs::read(&args[0]).expect("read docx");
    let doc = Document::open(&bytes, &Limits::default()).expect("open");
    let fonts = FontLibrary::bundled();
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());
    let mut options = PdfOptions::default();
    if let Some(i) = args.iter().position(|a| a == "--pdfa") {
        options.conformance = match args.get(i + 1).map(String::as_str) {
            Some("1b") => Conformance::PdfA1b,
            _ => Conformance::PdfA2b,
        };
    }
    options.ua = args.iter().any(|a| a == "--pdfua");
    let out = export_pdf_with(
        &doc,
        &layout,
        &fonts,
        &PdfMetadata::default(),
        &options,
        &AtomicBool::new(false),
    )
    .expect("export");
    std::fs::write(&args[1], out.bytes).expect("write");
}
