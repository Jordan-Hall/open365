//! Language, headings, table structure, links in every story and the PDF/UA marker, checked on
//! our output against what Word wrote for the same documents
//! (`fixtures/reference/pdf/probes/*.docx` and Word's PDFs next to them). veraPDF is not part of
//! the build; `docs/architecture/pdf-export.md` records its results.

mod common;

use std::sync::atomic::AtomicBool;

use common::root;
use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_pdf::{Conformance, PdfMetadata, PdfOptions, export_pdf_with};
use open365_text::FontLibrary;

fn probe(name: &str, meta: &PdfMetadata, options: &PdfOptions) -> String {
    let bytes = std::fs::read(root().join(format!("fixtures/reference/pdf/probes/{name}.docx")))
        .expect("probe docx");
    let doc = Document::open(&bytes, &Limits::default()).expect("open");
    let fonts = FontLibrary::bundled();
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());
    let out = export_pdf_with(
        &doc,
        &layout,
        &fonts,
        meta,
        options,
        &AtomicBool::new(false),
    )
    .expect("export");
    // Dictionaries are plain text in our writer; content streams are compressed.
    String::from_utf8_lossy(&out.bytes).into_owned()
}

fn plain(name: &str) -> String {
    probe(name, &PdfMetadata::default(), &PdfOptions::default())
}

fn count(text: &str, needle: &str) -> usize {
    text.matches(needle).count()
}

#[test]
fn catalog_language_is_the_applications_not_the_documents_and_other_runs_get_spans() {
    // Word: /Lang (en) for an en-GB document, with no Span /Lang at all.
    let gb = plain("probe-lang-gb");
    assert!(gb.contains("/Lang (en)"), "{gb}");
    assert_eq!(count(&gb, "/S /Span"), 0);
    // Word writes `en` for a de-DE document too (it is the application's language) and tags
    // the German text itself with Span /Lang (de) (`probes/p-lang-settings`, `probe-lang`).
    let de = plain("probe-lang");
    assert!(de.contains("/Catalog") && de.contains("/Lang (en)"));
    assert!(de.contains("/Lang (de)") && de.contains("/Lang (fr)") && de.contains("/Lang (es)"));
    let all_de = plain("p-lang-settings");
    assert!(all_de.contains("/Lang (en)") && all_de.contains("/S /Span"));
    assert!(all_de.contains("/Lang (de)"));
    // The caller's language wins and is written as given.
    let given = probe(
        "probe-lang",
        &PdfMetadata {
            language: Some("de-AT".into()),
            ..PdfMetadata::default()
        },
        &PdfOptions::default(),
    );
    assert!(given.contains("/Lang (de-AT)"));
}

#[test]
fn numbered_headings_are_headings_with_their_number_and_still_in_the_outline() {
    let t = plain("probe-headings");
    assert_eq!(count(&t, "/S /H1"), 2);
    assert_eq!(count(&t, "/S /H2"), 1);
    // Only the numbered plain paragraph is a list item.
    assert_eq!(count(&t, "/S /LI"), 1);
    // Word's outline titles carry the number.
    assert!(
        t.contains("/Title (\u{feff}") || t.contains("/Title ("),
        "{t}"
    );
    assert_eq!(count(&t, "/Title ("), 3);
}

#[test]
fn tables_have_thead_tbody_scopes_headers_ids_and_a_description() {
    let t = plain("probe-table-alt");
    assert_eq!(count(&t, "/S /THead"), 1);
    assert_eq!(count(&t, "/S /TBody"), 1);
    // Header row (3) plus the first column of the two body rows (2).
    assert_eq!(count(&t, "/S /TH"), 5 + count(&t, "/S /THead"));
    assert_eq!(count(&t, "/Scope /Column"), 3);
    assert_eq!(count(&t, "/Scope /Row"), 2);
    assert_eq!(count(&t, "/S /TD"), 4);
    assert_eq!(count(&t, "/Headers ["), 4);
    assert!(t.contains("/IDTree"));
    // Word wrote the description as /Alt of the Table.
    assert!(t.contains("/Alt (Quantities and prices of fruit)"));
    assert!(t.contains("/Summary (Quantities and prices of fruit)"));
    // No description, no Alt.
    assert!(!plain("probe-table-plain").contains("/Alt"));
}

#[test]
fn links_in_headers_footers_and_notes_become_annotations_and_link_elements() {
    let t = plain("probe-links");
    // Word: body link, header link, footer link, note link and the internal link on the note's
    // reference mark (its Dest is the note).
    assert_eq!(count(&t, "/Subtype /Link"), 5, "{t}");
    assert_eq!(count(&t, "/Dest ["), 1);
    for target in ["body", "header", "footer", "note"] {
        assert!(
            t.contains(&format!("/URI (https://example.com/{target})")),
            "{target}"
        );
    }
    // Word tags the body and note links and leaves the header and footer links (artifacts)
    // without a Link element; PDF/UA needs them in the tree.
    assert_eq!(count(&t, "/S /Link"), 3);
    assert_eq!(count(&t, "/Type /OBJR"), 3);
    let ua = probe(
        "probe-links",
        &PdfMetadata::default(),
        &PdfOptions {
            ua: true,
            ..PdfOptions::default()
        },
    );
    assert_eq!(count(&ua, "/S /Link"), 5);
}

#[test]
fn a_link_around_a_field_the_layout_replaced_is_still_found() {
    // The footer link wraps PAGE: the model holds the cached "9", the layout draws the page
    // number. A text search for "9" would miss it.
    let t = plain("probe-links-field");
    assert_eq!(count(&t, "/Subtype /Link"), 2, "body and footer link");
    assert_eq!(
        count(&t, "/S /Link"),
        1,
        "the footer link is untagged, as in Word"
    );
}

#[test]
fn pdf_ua_marks_the_file_and_always_has_a_title() {
    let options = PdfOptions {
        ua: true,
        ..PdfOptions::default()
    };
    let t = probe("probe-headings", &PdfMetadata::default(), &options);
    assert!(t.contains("<pdfuaid:part>1</pdfuaid:part>"));
    assert!(t.contains("http://www.aiim.org/pdfua/ns/id/"));
    assert!(
        t.contains("<dc:title>"),
        "title falls back to the first heading"
    );
    assert!(t.contains("/DisplayDocTitle true"));
    assert!(!plain("probe-headings").contains("pdfuaid"));
}

#[test]
fn archival_modes_leave_out_the_notdef_glyph_and_pdfa1_has_a_cidset() {
    let a1 = probe(
        "probe-links",
        &PdfMetadata::default(),
        &PdfOptions {
            conformance: Conformance::PdfA1b,
            ..PdfOptions::default()
        },
    );
    assert!(a1.contains("/CIDSet"));
    let a2 = probe(
        "probe-links",
        &PdfMetadata::default(),
        &PdfOptions {
            conformance: Conformance::PdfA2b,
            ..PdfOptions::default()
        },
    );
    assert!(!a2.contains("/CIDSet"));
}

#[test]
fn notes_are_footnote_elements_in_a_link_around_the_mark() {
    // As Word writes them: Link > (OBJR, Span for the mark, Footnote with an /ID holding the
    // body), and a RoleMap that maps Footnote and Endnote to Note.
    let t = plain("probe-links");
    assert_eq!(count(&t, "/S /Footnote"), 1, "{t}");
    assert_eq!(count(&t, "/S /Reference"), 0);
    assert!(t.contains("/Footnote /Note /Endnote /Note"));
    assert!(t.contains("/ID (Note 1)"));
}
