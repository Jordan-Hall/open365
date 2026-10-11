//! Pictures in headers, footers and notes reach the page, resolved through their own part's
//! relationships.

mod common;

use std::io::Read as _;

use common::root;
use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_pdf::{PdfMetadata, export_pdf};
use open365_text::FontLibrary;

fn templates_with_furniture_drawings() -> Vec<std::path::PathBuf> {
    let dir = root().join("fixtures/external/ms-templates");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("templates").flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("docx") {
            continue;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let Ok(mut zip) = zip::ZipArchive::new(file) else {
            continue;
        };
        let mut hit = false;
        for i in 0..zip.len() {
            let Ok(mut f) = zip.by_index(i) else { continue };
            let Ok(name) = f.name().map(|n| n.into_owned()) else {
                continue;
            };
            if name.starts_with("word/header") || name.starts_with("word/footer") {
                let mut data = Vec::new();
                if f.read_to_end(&mut data).is_ok() && data.windows(10).any(|w| w == b"<w:drawing")
                {
                    hit = true;
                }
            }
        }
        if hit {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn pictures_of_headers_and_footers_are_laid_out_and_drawn() {
    let fonts = FontLibrary::bundled();
    let mut found = 0usize;
    for path in templates_with_furniture_drawings() {
        let bytes = std::fs::read(&path).expect("docx");
        let Ok(doc) = Document::open(&bytes, &Limits::default()) else {
            continue;
        };
        let layout = layout_document(&doc, &fonts, LayoutOptions::default());
        let images: usize = layout
            .pages
            .iter()
            .map(|p| p.header_images.len() + p.footer_images.len())
            .sum();
        if images == 0 {
            continue;
        }
        found += 1;
        let out = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default()).expect("export");
        assert!(
            !out.warnings
                .iter()
                .any(|w| w.contains("image part not found")),
            "{}: {:?}",
            path.display(),
            out.warnings
        );
        assert!(
            out.bytes.windows(5).any(|w| w == b"/Im0 "),
            "no image drawn"
        );
    }
    assert!(found > 0, "no template has a picture in a header or footer");
}
