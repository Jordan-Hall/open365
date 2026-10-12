//! A Title paragraph inside a block content control is centred like any other (`ca4e5536`).
//!
//! The corpus reference gives `left` 72 for these lines because Word's `Information` call reports
//! the control's start for a placeholder; Word's own PDF centres the text (first glyph at 218.0
//! against the engine's 217.92), so this test reads the PDF. Skipped without the local corpus.

mod common;
#[path = "common/wordpdf.rs"]
mod wordpdf;

use common::{Mode, library_with, root};
use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;

#[test]
fn title_in_a_block_content_control_is_centred_where_words_pdf_centres_it() {
    let id = "ca4e5536-5f4f-40f6-8c24-37e4a1f0d8f7";
    let root = root();
    let docx = root.join(format!("fixtures/external/ms-templates/{id}.docx"));
    let pdf = root.join(format!("fixtures/reference/ms-templates/{id}/{id}.pdf"));
    let (Ok(bytes), Ok(pdf)) = (std::fs::read(docx), std::fs::read(pdf)) else {
        println!("ms-template corpus absent; skipped");
        return;
    };
    let doc = Document::open(&bytes, &Limits::default()).expect("open");
    let probe = layout_document(
        &doc,
        &open365_text::FontLibrary::bundled(),
        LayoutOptions::default(),
    );
    let extra: Vec<String> = probe.fonts.iter().map(|f| f.requested.clone()).collect();
    let fonts = library_with(Mode::System, &extra);
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());
    let word = wordpdf::read_pages(&pdf);
    let word_x = word[0]
        .glyphs
        .iter()
        .find(|g| g.ch == 'W')
        .map(|g| g.x)
        .expect("title glyph");
    let (_, title) = layout
        .lines()
        .find(|(_, l)| l.text.starts_with("Wedding"))
        .expect("title line");
    assert!(
        (title.first_glyph_x() - (word_x - 0.024)).abs() <= 0.5,
        "engine {} Word {}",
        title.first_glyph_x(),
        word_x
    );
}
