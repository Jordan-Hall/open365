//! A hyperlink inside a text box becomes a `Link` annotation over the box's text.

mod common;

use std::io::{Cursor, Read as _, Write as _};
use std::sync::atomic::AtomicBool;

use common::root;
use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_pdf::{PdfMetadata, PdfOptions, export_pdf_with};
use open365_text::FontLibrary;

fn patched(name: &str) -> Vec<u8> {
    let bytes = std::fs::read(root().join(format!("fixtures/docx/{name}.docx"))).expect("docx");
    let mut zin = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).expect("entry");
        let entry = f.name().expect("name").into_owned();
        let mut data = Vec::new();
        f.read_to_end(&mut data).expect("read");
        let text = String::from_utf8_lossy(&data).into_owned();
        let text = match entry.as_str() {
            "word/document.xml" => text.replacen(
                "<w:r><w:t>Zeta never wraps",
                "<w:hyperlink r:id=\"rIdBox1\"><w:r><w:t>Zeta never wraps</w:t></w:r></w:hyperlink><w:r><w:t> and",
                1,
            )
            .replacen(" and runs past", " runs past", 1),
            "word/_rels/document.xml.rels" => text.replacen(
                "</Relationships>",
                "<Relationship Id=\"rIdBox1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"https://example.com/box\" TargetMode=\"External\"/></Relationships>",
                1,
            ),
            _ => text,
        };
        out.start_file(&entry, zip::write::SimpleFileOptions::default())
            .expect("start");
        out.write_all(text.as_bytes()).expect("write");
    }
    out.finish().expect("finish").into_inner()
}

#[test]
fn a_link_in_a_text_box_is_an_annotation_with_a_link_element() {
    let doc = Document::open(&patched("layout-txbx-basic"), &Limits::default()).expect("open");
    let fonts = FontLibrary::bundled();
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());
    assert!(layout.pages.iter().any(|p| !p.frames.is_empty()));
    let out = export_pdf_with(
        &doc,
        &layout,
        &fonts,
        &PdfMetadata::default(),
        &PdfOptions::default(),
        &AtomicBool::new(false),
    )
    .expect("export");
    let t = String::from_utf8_lossy(&out.bytes).into_owned();
    assert_eq!(t.matches("/Subtype /Link").count(), 1, "{t}");
    assert!(t.contains("/URI (https://example.com/box)"));
    assert_eq!(t.matches("/S /Link").count(), 1);
}
