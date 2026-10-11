//! Table shading and borders reach the page as vector drawing.

mod common;

use std::io::{Read as _, Write as _};

use common::{Reader, layout_fixture, root};
use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_pdf::{PdfMetadata, export_pdf};
use open365_text::FontLibrary;

fn content(bytes: &[u8]) -> String {
    let r = Reader::new(bytes);
    let mut all = String::new();
    for (_, page) in r.pages() {
        let c = r.stream(&r.key(&page, "Contents").expect("contents"));
        all.push_str(&String::from_utf8_lossy(&c));
    }
    all
}

#[test]
fn table_borders_are_drawn_as_artifacts() {
    let fonts = FontLibrary::bundled();
    let (doc, layout) = layout_fixture("layout-table-basic", &fonts).expect("layout");
    let out = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default()).expect("export");
    let text = content(&out.bytes);
    assert!(text.contains("/Artifact BMC"), "no artifact block");
    assert!(text.contains(" re f"), "no filled border strips");
}

#[test]
fn cell_shading_is_filled_with_its_colour() {
    let src = std::fs::read(root().join("fixtures/docx/layout-table-basic.docx")).expect("docx");
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(src)).expect("zip");
    let mut out = Vec::new();
    {
        let mut zout = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        for i in 0..zin.len() {
            let mut f = zin.by_index(i).expect("entry");
            let name = f.name().expect("name").into_owned();
            let mut data = Vec::new();
            f.read_to_end(&mut data).expect("read");
            if name == "word/document.xml" {
                let xml = String::from_utf8(data).expect("utf8");
                let marker = "<w:tcPr>";
                let at = xml.find(marker).expect("a cell with properties") + marker.len();
                data = format!(
                    "{}<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"FFCC00\"/>{}",
                    &xml[..at],
                    &xml[at..]
                )
                .into_bytes();
            }
            zout.start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start");
            zout.write_all(&data).expect("write");
        }
        zout.finish().expect("finish");
    }
    let doc = Document::open(&out, &Limits::default()).expect("open");
    let fonts = FontLibrary::bundled();
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());
    let pdf = export_pdf(&doc, &layout, &fonts, &PdfMetadata::default()).expect("export");
    let text = content(&pdf.bytes);
    assert!(text.contains("1 0.8 0 rg"), "shading colour missing");
}
