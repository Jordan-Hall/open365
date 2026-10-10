//! Editing behaviour on small synthetic documents.

mod common;

use open365_docx::{Document, DocxError, Inline, Paragraph};
use open365_opc::{Limits, Package};

use common::{assert_infoset_eq, build, docx_with_body, root_rels};

fn open_body(body: &str) -> Document {
    Document::open(&docx_with_body(body), &Limits::default()).expect("opens")
}

/// Text of each direct and grouped run of paragraph `index`, in order.
fn run_texts(doc: &Document, index: usize) -> Vec<String> {
    fn walk(content: &[Inline], out: &mut Vec<String>) {
        for inline in content {
            match inline {
                Inline::Run(r) => out.push(r.text()),
                Inline::Group(g) => walk(&g.content, out),
                Inline::Raw(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.paragraph(index).expect("paragraph").content, &mut out);
    out
}

fn run_bold(paragraph: &Paragraph) -> Vec<Option<bool>> {
    fn walk(content: &[Inline], out: &mut Vec<Option<bool>>) {
        for inline in content {
            match inline {
                Inline::Run(r) => out.push(r.props.bold()),
                Inline::Group(g) => walk(&g.content, out),
                Inline::Raw(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&paragraph.content, &mut out);
    out
}

fn main_xml(doc: &Document) -> String {
    String::from_utf8(doc.serialize_main()).expect("utf-8")
}

#[test]
fn bold_splits_a_run_and_clones_its_properties() {
    let mut doc = open_body(
        r#"<w:p><w:r><w:rPr><w:i/><w:color w:val="FF0000"/></w:rPr><w:t>Hello world</w:t></w:r></w:p>"#,
    );
    doc.set_bold(0, 6..11, true).expect("bold");
    assert_eq!(run_texts(&doc, 0), ["Hello ", "world"]);
    let paragraph = doc.paragraph(0).expect("p");
    assert_eq!(run_bold(paragraph), [None, Some(true)]);
    // Both halves keep italic and colour; bold is placed before italic (schema order).
    let xml = main_xml(&doc);
    assert!(
        xml.contains(r#"<w:r><w:rPr><w:i/><w:color w:val="FF0000"/></w:rPr><w:t xml:space="preserve">Hello </w:t></w:r>"#),
        "{xml}"
    );
    assert!(
        xml.contains(
            r#"<w:r><w:rPr><w:b/><w:i/><w:color w:val="FF0000"/></w:rPr><w:t>world</w:t></w:r>"#
        ),
        "{xml}"
    );
}

#[test]
fn bold_in_the_middle_makes_three_runs() {
    let mut doc = open_body("<w:p><w:r><w:t>abcdef</w:t></w:r></w:p>");
    doc.set_bold(0, 2..4, true).expect("bold");
    assert_eq!(run_texts(&doc, 0), ["ab", "cd", "ef"]);
    assert_eq!(
        run_bold(doc.paragraph(0).expect("p")),
        [None, Some(true), None]
    );
    // Turning it off again writes an explicit w:val="0".
    doc.set_bold(0, 2..4, false).expect("unbold");
    assert!(main_xml(&doc).contains(r#"<w:b w:val="0"/>"#));
    assert_eq!(doc.paragraph_text(0).as_deref(), Some("abcdef"));
}

#[test]
fn bold_spanning_runs_and_hyperlinks() {
    let mut doc = open_body(
        r#"<w:p><w:r><w:t>one </w:t></w:r><w:hyperlink w:anchor="x"><w:r><w:t>two</w:t></w:r></w:hyperlink><w:r><w:t> three</w:t></w:r></w:p>"#,
    );
    assert_eq!(doc.paragraph_text(0).as_deref(), Some("one two three"));
    doc.set_bold(0, 2..9, true).expect("bold");
    assert_eq!(run_texts(&doc, 0), ["on", "e ", "two", " t", "hree"]);
    assert_eq!(
        run_bold(doc.paragraph(0).expect("p")),
        [None, Some(true), Some(true), Some(true), None]
    );
}

#[test]
fn bold_keeps_tabs_breaks_and_unknown_run_children() {
    let mut doc = open_body(
        r#"<w:p><w:r><w:t>ab</w:t><w:tab/><w:fldChar w:fldCharType="begin"/><w:t>cd</w:t><w:br/></w:r></w:p>"#,
    );
    assert_eq!(doc.paragraph_text(0).as_deref(), Some("ab\tcd\n"));
    doc.set_bold(0, 3..4, true).expect("bold");
    let xml = main_xml(&doc);
    assert_eq!(
        doc.paragraph_text(0).as_deref(),
        Some(
            "ab	cd
"
        )
    );
    // A zero-width child between two characters moves to the right-hand run.
    assert!(
        xml.contains(r#"<w:r><w:t>ab</w:t><w:tab/></w:r><w:r><w:rPr><w:b/></w:rPr><w:fldChar w:fldCharType="begin"/><w:t>c</w:t></w:r><w:r><w:t>d</w:t><w:br/></w:r>"#),
        "{xml}"
    );
    assert_eq!(xml.matches("<w:fldChar").count(), 1);
}

#[test]
fn insert_inherits_the_covering_run() {
    let mut doc = open_body(
        r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r><w:r><w:t>plain</w:t></w:r></w:p>"#,
    );
    // Offset 4 is the boundary: the preceding (bold) run covers it.
    doc.insert_text(0, 4, "!").expect("insert");
    assert_eq!(run_texts(&doc, 0), ["bold!", "plain"]);
    // Offset 0 goes to the start of the first run.
    doc.insert_text(0, 0, ">").expect("insert");
    assert_eq!(run_texts(&doc, 0), [">bold!", "plain"]);
    // The end of the paragraph goes to the last run.
    doc.insert_text(0, 11, "<").expect("insert");
    assert_eq!(run_texts(&doc, 0), [">bold!", "plain<"]);
    assert_eq!(doc.paragraph_text(0).as_deref(), Some(">bold!plain<"));
}

#[test]
fn insert_into_an_empty_paragraph_creates_a_run() {
    let mut doc = open_body("<w:p/>");
    doc.insert_text(0, 0, "hi").expect("insert");
    assert_eq!(doc.paragraph_text(0).as_deref(), Some("hi"));
    assert!(main_xml(&doc).contains("<w:p><w:r><w:t>hi</w:t></w:r></w:p>"));

    // A paragraph with properties keeps them first.
    let mut doc = open_body(r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr></w:p>"#);
    doc.insert_text(0, 0, "x").expect("insert");
    assert!(
        main_xml(&doc)
            .contains(r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#)
    );
}

#[test]
fn preserve_space_is_set_only_when_needed() {
    let mut doc = open_body("<w:p><w:r><w:t>ab</w:t></w:r></w:p>");
    doc.insert_text(0, 1, "X").expect("insert");
    assert!(main_xml(&doc).contains("<w:t>aXb</w:t>"));
    doc.insert_text(0, 0, " ").expect("insert");
    assert!(main_xml(&doc).contains(r#"<w:t xml:space="preserve"> aXb</w:t>"#));

    let mut doc = open_body("<w:p><w:r><w:t>ab</w:t></w:r></w:p>");
    doc.insert_text(0, 2, " ").expect("insert");
    assert!(main_xml(&doc).contains(r#"<w:t xml:space="preserve">ab </w:t>"#));

    // Whitespace the document already preserved survives a round trip untouched.
    let doc =
        open_body(r#"<w:p><w:r><w:t xml:space="preserve">  lead and trail  </w:t></w:r></w:p>"#);
    assert_eq!(doc.paragraph_text(0).as_deref(), Some("  lead and trail  "));
    assert!(main_xml(&doc).contains(r#"<w:t xml:space="preserve">  lead and trail  </w:t>"#));

    // Splitting a run whose halves end or start with spaces preserves them.
    let mut doc = open_body("<w:p><w:r><w:t>one two</w:t></w:r></w:p>");
    doc.set_bold(0, 0..3, true).expect("bold");
    let xml = main_xml(&doc);
    assert!(
        xml.contains(r#"<w:t xml:space="preserve"> two</w:t>"#),
        "{xml}"
    );
    assert!(xml.contains("<w:t>one</w:t>"), "{xml}");
}

#[test]
fn tabs_and_newlines_in_inserted_text_become_elements() {
    let mut doc = open_body("<w:p><w:r><w:t>ab</w:t></w:r></w:p>");
    doc.insert_text(0, 1, "x\ty\r\nz").expect("insert");
    assert_eq!(doc.paragraph_text(0).as_deref(), Some("ax\ty\nzb"));
    let xml = main_xml(&doc);
    assert!(
        xml.contains("<w:t>a</w:t><w:t>x</w:t><w:tab/><w:t>y</w:t><w:br/><w:t>z</w:t><w:t>b</w:t>"),
        "{xml}"
    );
}

#[test]
fn unicode_offsets_are_scalar_values() {
    // a, U+1F600 (surrogate pair in UTF-16), e + U+0301 (combining acute), b.
    let text = "a\u{1F600}e\u{301}b";
    let mut doc = open_body(&format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"));
    assert_eq!(doc.paragraph_text(0).as_deref(), Some(text));
    assert_eq!(doc.paragraph(0).expect("p").char_len(), 5);

    doc.insert_text(0, 2, "X").expect("after the emoji");
    assert_eq!(
        doc.paragraph_text(0).as_deref(),
        Some("a\u{1F600}Xe\u{301}b")
    );
    // Offset 4 sits between e and its combining mark: the model allows it (scalar values).
    doc.insert_text(0, 5, "Y")
        .expect("between e and the accent");
    assert_eq!(
        doc.paragraph_text(0).as_deref(),
        Some("a\u{1F600}Xe\u{301}Yb")
    );
    assert!(doc.insert_text(0, 8, "!").is_err(), "past the end");

    doc.set_bold(0, 1..2, true).expect("bold the emoji only");
    assert_eq!(run_texts(&doc, 0), ["a", "\u{1F600}", "Xe\u{301}Yb"]);
    assert!(main_xml(&doc).contains("<w:t>\u{1F600}</w:t>"));
}

#[test]
fn out_of_range_arguments_are_errors_and_change_nothing() {
    let mut doc = open_body("<w:p><w:r><w:t>abc</w:t></w:r></w:p>");
    assert!(matches!(
        doc.insert_text(1, 0, "x"),
        Err(DocxError::ParagraphOutOfRange { index: 1, count: 1 })
    ));
    assert!(matches!(
        doc.insert_text(0, 4, "x"),
        Err(DocxError::RangeOutOfBounds { .. })
    ));
    assert!(matches!(
        doc.set_bold(0, 2..4, true),
        Err(DocxError::RangeOutOfBounds { .. })
    ));
    assert!(matches!(
        doc.insert_text(0, 0, "bad\u{0}"),
        Err(DocxError::InvalidText('\u{0}'))
    ));
    // An empty range is a no-op, not an edit.
    doc.set_bold(0, 1..1, true).expect("empty range");
    assert!(!doc.is_dirty());
    assert!(doc.hygiene_log().is_empty());
}

#[test]
fn paragraphs_in_tables_are_counted_in_document_order() {
    let doc = open_body(
        r#"<w:p><w:r><w:t>before</w:t></w:r></w:p>
<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="100"/></w:tblGrid>
<w:tr><w:tc><w:tcPr/><w:p><w:r><w:t>a1</w:t></w:r></w:p><w:p><w:r><w:t>a2</w:t></w:r></w:p></w:tc>
<w:tc><w:tbl><w:tr><w:tc><w:p><w:r><w:t>nested</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>b1</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>c1</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:r><w:t>after</w:t></w:r></w:p>"#,
    );
    let texts: Vec<String> = (0..doc.paragraph_count())
        .map(|i| doc.paragraph_text(i).expect("p"))
        .collect();
    assert_eq!(texts, ["before", "a1", "a2", "nested", "b1", "c1", "after"]);
    assert!(doc.paragraph_text(7).is_none());
}

#[test]
fn editing_a_paragraph_in_a_table_cell() {
    let mut doc = open_body(
        r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
    );
    doc.insert_text(0, 4, "!").expect("insert");
    doc.set_bold(0, 0..2, true).expect("bold");
    assert_eq!(doc.paragraph_text(0).as_deref(), Some("cell!"));
    assert_eq!(run_texts(&doc, 0), ["ce", "ll!"]);
}

#[test]
fn unknown_markup_survives_an_edit_in_place() {
    let body = r#"<!-- note --><w:p w14:paraId="1A2B3C4D" w:rsidR="00AA11BB" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:bookmarkStart w:id="0" w:name="b"/><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="wps"><w:r><w:t>choice</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>fallback</w:t></w:r></mc:Fallback></mc:AlternateContent>
  <w:r w:rsidRPr="00112233"><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="28"/><w14:textOutline xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"/></w:rPr><w:t>text</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p><w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr>"#;
    let original = docx_with_body(body);
    let mut doc = Document::open(&original, &Limits::default()).expect("opens");
    assert!(doc.dropped().is_empty());

    // Unedited: infoset-equal, whitespace between elements included.
    let before = Package::read(&original, &Limits::default()).expect("package");
    assert_infoset_eq(
        before.part("word/document.xml").expect("main").data(),
        &doc.serialize_main(),
    )
    .expect("unedited infoset");

    // The paragraph text only counts directly modelled runs (AlternateContent is raw).
    assert_eq!(doc.paragraph_text(0).as_deref(), Some("text"));
    doc.insert_text(0, 4, "s").expect("insert");
    let xml = main_xml(&doc);
    assert!(xml.contains("<w:t>texts</w:t>"));
    assert!(xml.contains(r#"w14:paraId="1A2B3C4D""#));
    assert!(xml.contains("<mc:Fallback>"));
    assert!(xml.contains("<!-- note -->"));
    assert!(xml.contains(
        r#"<w14:textOutline xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"/>"#
    ));
    assert!(xml.contains(r#"<w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="28"/>"#));
}

#[test]
fn run_props_expose_typed_values_and_extras() {
    let doc = open_body(
        r#"<w:p><w:r><w:rPr><w:rStyle w:val="Emph"/><w:rFonts w:ascii="Arial"/><w:b w:val="0"/><w:i/><w:strike w:val="true"/><w:color w:val="00FF00"/><w:sz w:val="22"/><w:highlight w:val="yellow"/><w:u w:val="double"/><w:vertAlign w:val="superscript"/></w:rPr><w:t>x</w:t></w:r></w:p>"#,
    );
    let paragraph = doc.paragraph(0).expect("p");
    let Inline::Run(run) = &paragraph.content[0] else {
        panic!("run expected")
    };
    let p = &run.props;
    assert_eq!(p.bold(), Some(false));
    assert_eq!(p.italic(), Some(true));
    assert_eq!(p.strike(), Some(true));
    assert_eq!(p.color(), Some("00FF00"));
    assert_eq!(p.size_half_points(), Some(22));
    assert_eq!(p.highlight(), Some("yellow"));
    assert_eq!(p.underline(), Some("double"));
    assert_eq!(p.vert_align(), Some("superscript"));
    assert!(p.fonts().is_some());
    assert_eq!(p.extras().count(), 1, "only rStyle is untyped");
}

#[test]
fn main_part_is_found_through_relationships() {
    let document = format!(
        r#"<w:document xmlns:w="{}"><w:body><w:p><w:r><w:t>moved</w:t></w:r></w:p></w:body></w:document>"#,
        common::W
    );
    let bytes = build(
        &document,
        "content/story.xml",
        &root_rels("/content/story.xml", ""),
        &[],
    );
    let doc = Document::open(&bytes, &Limits::default()).expect("opens");
    assert_eq!(doc.main_part().as_str(), "content/story.xml");
    assert_eq!(doc.paragraph_text(0).as_deref(), Some("moved"));

    let no_rel = build(
        &document,
        "word/document.xml",
        &root_rels("x", "").replace("officeDocument\"", "other\""),
        &[],
    );
    assert!(matches!(
        Document::open(&no_rel, &Limits::default()),
        Err(DocxError::NoMainDocument)
    ));
}

#[test]
fn first_edit_removes_thumbnail_and_signatures_and_nothing_else() {
    let document = format!(
        r#"<w:document xmlns:w="{}"><w:body><w:p><w:r><w:t>hello</w:t></w:r></w:p></w:body></w:document>"#,
        common::W
    );
    let rels = root_rels(
        "word/document.xml",
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail" Target="docProps/thumbnail.jpeg"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/origin" Target="_xmlsignatures/origin.sigs"/><Relationship Id="rId4" Type="http://example.com/other" Target="docProps/other.xml"/>"#,
    );
    let bytes = build(
        &document,
        "word/document.xml",
        &rels,
        &[
            ("docProps/thumbnail.jpeg", b"JPEGDATA"),
            ("docProps/other.xml", b"<other/>"),
            ("_xmlsignatures/origin.sigs", b""),
            ("_xmlsignatures/sig1.xml", b"<Signature/>"),
            ("_xmlsignatures/_rels/origin.sigs.rels", b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>"),
            ("customXml/item1.xml", b"<keep/>"),
        ],
    );

    // Opening and saving without an edit changes nothing.
    let mut doc = Document::open(&bytes, &Limits::default()).expect("opens");
    let untouched = Package::read(&doc.save().expect("save"), &Limits::default()).expect("pkg");
    assert!(untouched.part("docProps/thumbnail.jpeg").is_some());
    assert!(untouched.part("_xmlsignatures/sig1.xml").is_some());
    assert!(doc.hygiene_log().is_empty());

    let mut doc = Document::open(&bytes, &Limits::default()).expect("opens");
    doc.insert_text(0, 5, " world").expect("insert");
    assert!(!doc.hygiene_log().is_empty());
    let saved = Package::read(&doc.save().expect("save"), &Limits::default()).expect("pkg");
    assert!(saved.part("docProps/thumbnail.jpeg").is_none());
    assert!(saved.part("_xmlsignatures/origin.sigs").is_none());
    assert!(saved.part("_xmlsignatures/sig1.xml").is_none());
    assert!(
        saved
            .part("_xmlsignatures/_rels/origin.sigs.rels")
            .is_none()
    );
    assert_eq!(
        saved.part("customXml/item1.xml").expect("kept").data(),
        b"<keep/>"
    );
    assert_eq!(
        saved.part("docProps/other.xml").expect("kept").data(),
        b"<other/>"
    );

    let rels_after =
        String::from_utf8(saved.part("_rels/.rels").expect("rels").data().to_vec()).expect("utf-8");
    assert!(rels_after.contains("rId1") && rels_after.contains("rId4"));
    assert!(!rels_after.contains("rId2") && !rels_after.contains("rId3"));
    assert!(
        saved.check_integrity().is_empty(),
        "{:?}",
        saved.check_integrity()
    );

    // The log names what was removed.
    let log: Vec<String> = doc.hygiene_log().iter().map(ToString::to_string).collect();
    assert!(
        log.iter().any(|l| l.contains("docProps/thumbnail.jpeg")),
        "{log:?}"
    );
    assert!(
        log.iter().any(|l| l.contains("_xmlsignatures/sig1.xml")),
        "{log:?}"
    );

    // A second edit does not repeat the cleanup.
    let count = doc.hygiene_log().len();
    doc.insert_text(0, 0, "x").expect("insert");
    assert_eq!(doc.hygiene_log().len(), count);
}
