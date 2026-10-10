//! Range deletion, paragraph splitting, run/paragraph properties, snapshots and styles.

mod common;

use open365_docx::{
    Alignment, DocPos, Document, DocxError, EditLogEntry, Inline, LineSpacing, RunPropsPatch,
};
use open365_opc::Limits;

use common::{W, build, docx_with_body, root_rels};

fn open(bytes: &[u8]) -> Document {
    Document::open(bytes, &Limits::default()).expect("opens")
}

fn doc(body: &str) -> Document {
    open(&docx_with_body(body))
}

fn texts(d: &Document) -> Vec<String> {
    (0..d.paragraph_count())
        .map(|i| d.paragraph_text(i).expect("paragraph"))
        .collect()
}

fn xml(d: &Document) -> String {
    String::from_utf8(d.serialize_main()).expect("utf8")
}

fn pos(para: usize, offset: usize) -> DocPos {
    DocPos::new(para, offset)
}

fn p(text: &str) -> String {
    format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
}

#[test]
fn delete_inside_one_run_and_across_runs() {
    let mut d = doc(
        "<w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\"> big world</w:t></w:r></w:p>",
    );
    d.delete_range(pos(0, 1), pos(0, 3)).expect("delete");
    assert_eq!(texts(&d), ["Hlo big world"]);
    d.delete_range(pos(0, 2), pos(0, 7)).expect("across runs");
    assert_eq!(texts(&d), ["Hl world"]);
    // Reversed bounds are accepted.
    d.delete_range(pos(0, 7), pos(0, 5)).expect("reversed");
    assert_eq!(texts(&d), ["Hl wod"]);
    assert!(d.is_dirty());
}

#[test]
fn deleting_everything_in_a_run_removes_the_run_but_keeps_the_paragraph() {
    let mut d = doc(&format!("{}{}", p("abc"), p("def")));
    d.delete_range(pos(0, 0), pos(0, 3)).expect("delete");
    assert_eq!(texts(&d), ["", "def"]);
    assert!(!xml(&d).contains("<w:t>abc"));
}

#[test]
fn delete_across_paragraphs_merges_and_first_ppr_wins() {
    let mut d = doc(
        "<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:t>Alpha</w:t></w:r></w:p>\
         <w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Beta</w:t></w:r></w:p>\
         <w:p><w:pPr><w:jc w:val=\"right\"/></w:pPr><w:r><w:t>Gamma</w:t></w:r></w:p>",
    );
    d.delete_range(pos(0, 2), pos(2, 2)).expect("delete");
    assert_eq!(texts(&d), ["Almma"]);
    let out = xml(&d);
    assert!(out.contains("w:val=\"center\""));
    assert!(!out.contains("Heading1"));
    assert!(!out.contains("w:val=\"right\""));
}

#[test]
fn backspace_at_paragraph_start_joins_with_previous() {
    let mut d = doc(&format!("{}{}", p("one"), p("two")));
    d.delete_range(pos(0, 3), pos(1, 0)).expect("join");
    assert_eq!(texts(&d), ["onetwo"]);
}

#[test]
fn empty_first_paragraph_yields_to_the_following_paragraph_properties() {
    let mut d = doc(
        "<w:p/><w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p>",
    );
    d.delete_range(pos(0, 0), pos(1, 0)).expect("join");
    assert_eq!(texts(&d), ["Title"]);
    assert!(xml(&d).contains("Heading1"));
}

#[test]
fn bookmarks_and_comment_ranges_survive_and_stay_balanced() {
    let mut d = doc(
        "<w:p><w:r><w:t>aaa</w:t></w:r><w:bookmarkStart w:id=\"1\" w:name=\"b\"/><w:commentRangeStart w:id=\"7\"/><w:r><w:t>bbb</w:t></w:r></w:p>\
         <w:p><w:r><w:t>ccc</w:t></w:r><w:bookmarkEnd w:id=\"1\"/><w:commentRangeEnd w:id=\"7\"/><w:r><w:commentReference w:id=\"7\"/></w:r><w:r><w:t>ddd</w:t></w:r></w:p>",
    );
    d.delete_range(pos(0, 1), pos(1, 5)).expect("delete");
    assert_eq!(texts(&d), ["ad"]);
    let out = xml(&d);
    for needle in [
        "<w:bookmarkStart",
        "<w:bookmarkEnd",
        "<w:commentRangeStart",
        "<w:commentRangeEnd",
        "<w:commentReference",
    ] {
        assert_eq!(out.matches(needle).count(), 1, "{needle} in {out}");
    }
    assert!(d.edit_log().is_empty(), "markers are kept silently");
}

#[test]
fn drawings_inside_the_range_are_removed_and_logged() {
    let mut d = doc(
        "<w:p><w:r><w:t>ab</w:t></w:r><w:r><w:drawing><wp:inline xmlns:wp=\"urn:wp\"/></w:drawing></w:r><w:r><w:t>cd</w:t></w:r></w:p>",
    );
    d.delete_range(pos(0, 1), pos(0, 3)).expect("delete");
    assert_eq!(texts(&d), ["ad"]);
    assert!(!xml(&d).contains("drawing"));
    let log: &[EditLogEntry] = d.edit_log();
    assert_eq!(log.len(), 1);
    assert!(log[0].description.contains("w:drawing"), "{log:?}");
}

#[test]
fn drawing_at_the_range_edge_is_kept() {
    let mut d = doc(
        "<w:p><w:r><w:t>ab</w:t></w:r><w:r><w:drawing><wp:inline xmlns:wp=\"urn:wp\"/></w:drawing></w:r><w:r><w:t>cd</w:t></w:r></w:p>",
    );
    d.delete_range(pos(0, 0), pos(0, 2)).expect("delete");
    assert!(xml(&d).contains("drawing"));
    assert!(d.edit_log().is_empty());
}

#[test]
fn a_range_covering_the_whole_paragraph_also_takes_leading_and_trailing_drawings() {
    let body = "<w:p><w:r><w:drawing/></w:r><w:r><w:t>ab</w:t></w:r><w:r><w:drawing/></w:r></w:p>";
    let mut d = doc(body);
    d.delete_range(pos(0, 0), pos(0, 1)).expect("partial");
    assert_eq!(
        xml(&d).matches("<w:drawing").count(),
        2,
        "partial deletes keep the edges"
    );
    d.delete_range(pos(0, 0), pos(0, 1))
        .expect("rest of the text");
    assert_eq!(texts(&d), [""]);
    assert_eq!(
        xml(&d).matches("<w:drawing").count(),
        0,
        "now the paragraph is covered"
    );
    assert_eq!(d.edit_log().len(), 2);
}

const FIELD: &str = "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>7</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>";

#[test]
fn a_range_covering_a_field_result_removes_the_whole_field() {
    let mut d = doc(&format!(
        "<w:p><w:r><w:t>ab</w:t></w:r>{FIELD}<w:r><w:t>cd</w:t></w:r></w:p>"
    ));
    assert_eq!(texts(&d), ["ab7cd"]);
    d.delete_range(pos(0, 2), pos(0, 3)).expect("delete field");
    assert_eq!(texts(&d), ["abcd"]);
    let out = xml(&d);
    assert!(
        !out.contains("fldChar") && !out.contains("instrText"),
        "{out}"
    );
    assert_eq!(d.edit_log().len(), 1);
}

#[test]
fn deleting_text_around_a_field_leaves_it_intact() {
    let mut d = doc(&format!(
        "<w:p><w:r><w:t>ab</w:t></w:r>{FIELD}<w:r><w:t>cd</w:t></w:r></w:p>"
    ));
    d.delete_range(pos(0, 1), pos(0, 2)).expect("before");
    d.delete_range(pos(0, 2), pos(0, 3)).expect("after");
    assert_eq!(texts(&d), ["a7d"]);
    assert_eq!(xml(&d).matches("<w:fldChar ").count(), 3);
}

#[test]
fn partially_cutting_a_field_is_refused_and_changes_nothing() {
    let field = FIELD.replace("<w:t>7</w:t>", "<w:t>77</w:t>");
    let body = format!("<w:p><w:r><w:t>ab</w:t></w:r>{field}<w:r><w:t>cd</w:t></w:r></w:p>");
    let mut d = doc(&body);
    assert_eq!(texts(&d), ["ab77cd"]);
    let before = xml(&d);
    // The begin marker (offset 2) is interior but the end marker (offset 4) is outside.
    let err = d
        .delete_range(pos(0, 1), pos(0, 3))
        .expect_err("cut at the front");
    assert!(matches!(err, DocxError::FieldCut), "{err:?}");
    // The end marker is interior but the begin marker is outside.
    let err = d
        .delete_range(pos(0, 3), pos(0, 5))
        .expect_err("cut at the back");
    assert!(matches!(err, DocxError::FieldCut), "{err:?}");
    assert_eq!(xml(&d), before, "a refused delete changes nothing");
    assert!(!d.is_dirty());
    // Deleting only part of the result is fine, and so is covering the whole field.
    d.delete_range(pos(0, 3), pos(0, 4))
        .expect("inside the result");
    assert_eq!(texts(&d), ["ab7cd"]);
    d.delete_range(pos(0, 1), pos(0, 4)).expect("whole field");
    assert_eq!(texts(&d), ["ad"]);
}

#[test]
fn a_cut_through_a_multi_paragraph_field_is_refused() {
    let body = "<w:p><w:r><w:t>x</w:t></w:r><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText>TOC</w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>r1</w:t></w:r></w:p>\
                <w:p><w:r><w:t>r2</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r><w:r><w:t>y</w:t></w:r></w:p>\
                <w:p><w:r><w:t>z</w:t></w:r></w:p>";
    let mut d = doc(body);
    let before = xml(&d);
    // Starts before the field (begin is interior) and ends after its end: covered, fine.
    // Starts inside the result and ends after the field end: the end marker is interior
    // while the begin is outside.
    let err = d.delete_range(pos(0, 2), pos(2, 1)).expect_err("cut");
    assert!(matches!(err, DocxError::FieldCut), "{err:?}");
    assert_eq!(xml(&d), before, "a refused delete changes nothing");
    assert!(
        d.delete_range(pos(0, 0), pos(2, 1)).is_ok(),
        "covering it whole is fine"
    );
    assert!(!xml(&d).contains("fldChar"));
}

#[test]
fn ranges_across_a_table_are_refused() {
    let body = format!(
        "{}<w:tbl><w:tr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>{}",
        p("before"),
        p("after")
    );
    let mut d = doc(&body);
    let err = d.delete_range(pos(0, 1), pos(2, 1)).expect_err("table");
    assert!(matches!(err, DocxError::UnsupportedRange(_)), "{err:?}");
    // Inside the cell it works.
    d.delete_range(pos(1, 0), pos(1, 2)).expect("in cell");
    assert_eq!(texts(&d)[1], "ll");
}

#[test]
fn out_of_bounds_positions_are_errors() {
    let mut d = doc(&p("abc"));
    assert!(d.delete_range(pos(0, 0), pos(0, 9)).is_err());
    assert!(d.delete_range(pos(3, 0), pos(3, 0)).is_err());
    assert!(d.split_paragraph(pos(0, 9)).is_err());
}

#[test]
fn split_in_the_middle_of_a_run_clones_ppr_and_run_props() {
    let mut d = doc(
        "<w:p w14:paraId=\"AAAA0001\" xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\"><w:pPr><w:pStyle w:val=\"Heading1\"/><w:jc w:val=\"center\"/></w:pPr><w:r><w:rPr><w:i/></w:rPr><w:t>Hello world</w:t></w:r></w:p>",
    );
    d.split_paragraph(pos(0, 5)).expect("split");
    assert_eq!(texts(&d), ["Hello", " world"]);
    let out = xml(&d);
    assert_eq!(out.matches("Heading1").count(), 2);
    assert_eq!(out.matches("w:val=\"center\"").count(), 2);
    assert_eq!(out.matches("<w:i/>").count(), 2);
    assert_eq!(
        out.matches("paraId").count(),
        1,
        "identity is not duplicated"
    );
}

#[test]
fn enter_at_the_end_gives_an_empty_paragraph_that_carries_the_caret_format() {
    let mut d = doc(
        "<w:p><w:pPr><w:jc w:val=\"right\"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val=\"40\"/></w:rPr><w:t>Big</w:t></w:r></w:p>",
    );
    d.split_paragraph(pos(0, 3)).expect("split");
    assert_eq!(texts(&d), ["Big", ""]);
    // Typing into the new paragraph uses the paragraph-mark properties.
    d.insert_text(1, 0, "next").expect("type");
    let out = xml(&d);
    let second = out.split("</w:p>").nth(1).expect("second paragraph");
    assert!(
        second.contains("<w:b/>") && second.contains("w:val=\"40\""),
        "{second}"
    );
    assert!(second.contains("w:val=\"right\""));
}

#[test]
fn splitting_a_hyperlink_clones_the_container() {
    let mut d = doc(
        "<w:p><w:hyperlink r:id=\"rId9\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><w:r><w:t>linktext</w:t></w:r></w:hyperlink></w:p>",
    );
    d.split_paragraph(pos(0, 4)).expect("split");
    assert_eq!(texts(&d), ["link", "text"]);
    assert_eq!(xml(&d).matches("<w:hyperlink").count(), 2);
}

#[test]
fn splitting_keeps_a_section_break_with_the_second_half() {
    let mut d = doc(
        "<w:p><w:pPr><w:sectPr><w:pgSz w:w=\"1\" w:h=\"2\"/></w:sectPr></w:pPr><w:r><w:t>abcd</w:t></w:r></w:p>",
    );
    d.split_paragraph(pos(0, 2)).expect("split");
    let out = xml(&d);
    assert_eq!(out.matches("<w:sectPr").count(), 1);
    assert!(out.find("<w:sectPr").expect("sect") > out.find("</w:p>").expect("end of first"));
}

#[test]
fn split_then_delete_restores_the_text() {
    let mut d = doc(&p("Hello world"));
    d.split_paragraph(pos(0, 5)).expect("split");
    d.delete_range(pos(0, 5), pos(1, 0)).expect("join");
    assert_eq!(texts(&d), ["Hello world"]);
}

#[test]
fn replace_range_types_over_a_selection_and_pastes_lines() {
    let mut d = doc(&format!("{}{}", p("one two"), p("three")));
    let end = d
        .replace_range(pos(0, 4), pos(1, 2), "X\nY")
        .expect("replace");
    assert_eq!(texts(&d), ["one X", "Yree"]);
    assert_eq!(end, pos(1, 1));
}

#[test]
fn run_props_patch_sets_every_property_across_paragraphs() {
    let mut d = doc(&format!("{}{}", p("abcdef"), p("ghijkl")));
    let patch = RunPropsPatch {
        bold: Some(true),
        italic: Some(true),
        underline: Some(true),
        strike: Some(true),
        size_half_points: Some(48),
        color: Some("FF0000".into()),
        font: Some("Georgia".into()),
    };
    d.set_run_props(pos(0, 2), pos(1, 3), &patch)
        .expect("patch");
    assert_eq!(texts(&d), ["abcdef", "ghijkl"]);
    let p0 = d.paragraph(0).expect("p0");
    let styled: Vec<_> = p0
        .content
        .iter()
        .filter_map(|i| match i {
            Inline::Run(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(styled.len(), 2);
    assert_eq!(styled[0].props.bold(), None);
    let r = styled[1];
    assert_eq!(r.text(), "cdef");
    assert_eq!(r.props.bold(), Some(true));
    assert_eq!(r.props.italic(), Some(true));
    assert_eq!(r.props.underline(), Some("single"));
    assert_eq!(r.props.strike(), Some(true));
    assert_eq!(r.props.size_half_points(), Some(48));
    assert_eq!(r.props.color(), Some("FF0000"));
    assert_eq!(
        r.props.fonts().and_then(|f| f.head.attr_w("ascii")),
        Some("Georgia")
    );
    // Schema order inside w:rPr: rFonts, b, i, strike, color, sz, u.
    let out = xml(&d);
    let rpr = &out[out.find("<w:rPr>").expect("rPr")..out.find("</w:rPr>").expect("end")];
    let order = [
        "rFonts", "<w:b/>", "<w:i/>", "strike", "color", "w:sz", "<w:u ",
    ];
    let at: Vec<usize> = order.iter().map(|n| rpr.find(n).expect(n)).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{rpr}");
    // Turning bold off writes an explicit override.
    d.set_run_props(pos(0, 2), pos(0, 6), &RunPropsPatch::bold(false))
        .expect("off");
    assert!(xml(&d).contains("<w:b w:val=\"0\"/>"));
}

fn styled_doc(body: &str) -> Document {
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"{W}\"><w:body>{body}</w:body></w:document>"
    );
    let styles = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"{W}\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme=\"minorHAnsi\"/><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"259\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
         <w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Base\"><w:name w:val=\"Base\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:keepNext/><w:ind w:left=\"720\" w:hanging=\"360\"/><w:jc w:val=\"both\"/></w:pPr><w:rPr><w:b/><w:color w:val=\"112233\"/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Derived\"><w:name w:val=\"Derived\"/><w:basedOn w:val=\"Base\"/><w:pPr><w:spacing w:before=\"200\" w:line=\"480\" w:lineRule=\"exact\"/></w:pPr><w:rPr><w:b w:val=\"0\"/><w:i/><w:sz w:val=\"30\"/></w:rPr></w:style>\
         <w:style w:type=\"character\" w:styleId=\"Loud\"><w:name w:val=\"Loud\"/><w:rPr><w:b/><w:u w:val=\"single\"/><w:rFonts w:ascii=\"Arial\"/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"CycleA\"><w:name w:val=\"A\"/><w:basedOn w:val=\"CycleB\"/></w:style><w:style w:type=\"paragraph\" w:styleId=\"CycleB\"><w:name w:val=\"B\"/><w:basedOn w:val=\"CycleA\"/></w:style></w:styles>"
    );
    let theme = "<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><a:themeElements><a:fontScheme name=\"t\"><a:majorFont><a:latin typeface=\"Aptos Display\"/></a:majorFont><a:minorFont><a:latin typeface=\"Aptos\"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>";
    let doc_rels = "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/></Relationships>";
    let bytes = build(
        &document,
        "word/document.xml",
        &root_rels("word/document.xml", ""),
        &[
            ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
            ("word/styles.xml", styles.as_bytes()),
            ("word/theme/theme1.xml", theme.as_bytes()),
        ],
    );
    open(&bytes)
}

#[test]
fn style_resolution_follows_defaults_based_on_chain_char_style_and_direct_formatting() {
    let d = styled_doc(
        "<w:p><w:pPr><w:pStyle w:val=\"Derived\"/></w:pPr><w:r><w:t>plain</w:t></w:r><w:r><w:rPr><w:rStyle w:val=\"Loud\"/><w:sz w:val=\"40\"/></w:rPr><w:t>loud</w:t></w:r></w:p>\
         <w:p><w:r><w:t>normal</w:t></w:r></w:p>\
         <w:p><w:pPr><w:pStyle w:val=\"CycleA\"/><w:jc w:val=\"right\"/></w:pPr><w:r><w:t>c</w:t></w:r></w:p>",
    );
    let styles = d.styles().expect("styles");
    assert_eq!(styles.theme_fonts().minor, "Aptos");
    let para = d.paragraph(0).expect("p0");
    let runs: Vec<_> = para
        .content
        .iter()
        .filter_map(|i| match i {
            Inline::Run(r) => Some(r),
            _ => None,
        })
        .collect();
    let plain = styles.effective_run_props(para, runs[0]);
    assert_eq!(plain.font, "Aptos", "theme minor font from docDefaults");
    assert_eq!(
        plain.size_half_points, 30,
        "Derived overrides the default size"
    );
    assert!(!plain.bold, "Derived turns Base's bold off");
    assert!(plain.italic);
    assert_eq!(
        plain.color,
        Some([0x11, 0x22, 0x33]),
        "colour inherited from Base"
    );
    let loud = styles.effective_run_props(para, runs[1]);
    assert!(loud.bold && loud.underline);
    assert_eq!(loud.font, "Arial");
    assert_eq!(loud.size_half_points, 40, "direct formatting wins");

    let pp = styles.effective_para_props(para);
    assert_eq!(pp.alignment, Alignment::Justify);
    assert!(pp.keep_next);
    assert_eq!(pp.space_before_pt, 10.0);
    assert_eq!(pp.space_after_pt, 8.0, "from docDefaults");
    assert_eq!(pp.line, LineSpacing::Exactly(24.0));
    assert_eq!(pp.indent_left_pt, 36.0);
    assert_eq!(pp.first_line_pt, -18.0);

    let normal = d.paragraph(1).expect("p1");
    let np = styles.effective_para_props(normal);
    assert_eq!(np.line, LineSpacing::Multiple(259.0 / 240.0));
    assert_eq!(np.alignment, Alignment::Left);
    let nr = styles.effective_run_props(normal, runs_of(normal)[0]);
    assert_eq!((nr.size_half_points, nr.bold), (22, false));

    // A basedOn cycle terminates; direct alignment still applies.
    let cyc = d.paragraph(2).expect("p2");
    assert_eq!(styles.effective_para_props(cyc).alignment, Alignment::Right);
}

fn runs_of(p: &open365_docx::Paragraph) -> Vec<&open365_docx::Run> {
    p.content
        .iter()
        .filter_map(|i| match i {
            Inline::Run(r) => Some(r),
            _ => None,
        })
        .collect()
}

#[test]
fn fixtures_resolve_headings_larger_than_body_text() {
    let (_, bytes) = common::fixtures()
        .into_iter()
        .find(|(n, _)| n == "styles-headings")
        .expect("fixture");
    let d = open(&bytes);
    let styles = d.styles().expect("styles");
    let mut heading = None;
    let mut body = None;
    for i in 0..d.paragraph_count() {
        let para = d.paragraph(i).expect("p");
        let Some(run) = runs_of(para).first().copied() else {
            continue;
        };
        let e = styles.effective_run_props(para, run);
        match para.style_id() {
            Some("Heading1") => heading = Some(e),
            None => body = body.or(Some(e)),
            _ => {}
        }
    }
    let (heading, body) = (heading.expect("heading"), body.expect("body"));
    assert!(
        heading.size_half_points > body.size_half_points,
        "{heading:?} {body:?}"
    );
    assert!(!body.font.is_empty());
}

#[test]
fn set_paragraph_style_adds_builtins_and_alignment_is_set() {
    let mut d = styled_doc("<w:p><w:r><w:t>x</w:t></w:r></w:p>");
    d.set_paragraph_style(0, "Heading2").expect("style");
    assert_eq!(d.paragraph(0).expect("p").style_id(), Some("Heading2"));
    let saved = d.save().expect("save");
    let reopened = open(&saved);
    let styles = reopened.styles().expect("styles");
    assert!(styles.has_style("Heading2"));
    let para = reopened.paragraph(0).expect("p");
    let run = runs_of(para)[0];
    let e = styles.effective_run_props(para, run);
    assert_eq!(e.size_half_points, 26);
    assert_eq!(e.font, "Aptos Display", "major theme font");
    assert_eq!(e.color, Some([0x2F, 0x54, 0x96]));

    d.set_paragraph_style(0, "Normal").expect("back to normal");
    assert_eq!(
        d.paragraph(0).expect("p").style_id(),
        None,
        "default style writes no pStyle"
    );
    assert!(matches!(
        d.set_paragraph_style(0, "NoSuchStyle"),
        Err(DocxError::UnknownStyle(_))
    ));
    d.set_alignment(0, Alignment::Center).expect("jc");
    assert!(xml(&d).contains("<w:jc w:val=\"center\"/>"));
    d.set_alignment(0, Alignment::Justify).expect("jc");
    assert!(xml(&d).contains("<w:jc w:val=\"both\"/>"));
    assert_eq!(xml(&d).matches("<w:jc ").count(), 1);
}

#[test]
fn ppr_children_are_inserted_in_schema_order() {
    let mut d = doc(
        "<w:p><w:pPr><w:spacing w:after=\"0\"/><w:rPr><w:b/></w:rPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>",
    );
    d.set_alignment(0, Alignment::Right).expect("jc");
    // Without a styles part any id is written as given.
    d.set_paragraph_style(0, "Heading1").expect("style");
    let out = xml(&d);
    let ppr = &out[out.find("<w:pPr>").expect("pPr")..out.find("</w:pPr>").expect("end")];
    let order = ["pStyle", "spacing", "jc", "rPr"];
    let at: Vec<usize> = order.iter().map(|n| ppr.find(n).expect(n)).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{ppr}");
}

#[test]
fn snapshots_undo_every_range_edit_exactly() {
    let body = format!(
        "{}{}{}<w:tbl><w:tr><w:tc>{}{}</w:tc></w:tr></w:tbl>",
        p("alpha"),
        p("beta"),
        p("gamma"),
        p("cell one"),
        p("cell two")
    );
    let mut d = doc(&body);
    let original = xml(&d);

    let snap = d.capture(0, 2).expect("capture");
    d.delete_range(pos(0, 2), pos(2, 2)).expect("delete");
    assert_ne!(xml(&d), original);
    d.restore(&snap).expect("restore");
    assert_eq!(xml(&d), original);

    let snap = d.capture(1, 1).expect("capture");
    d.split_paragraph(pos(1, 2)).expect("split");
    assert_eq!(d.paragraph_count(), 6);
    d.restore(&snap).expect("restore");
    assert_eq!(xml(&d), original);

    let snap = d.capture(3, 3).expect("capture cell paragraph");
    d.set_run_props(pos(3, 0), pos(3, 4), &RunPropsPatch::bold(true))
        .expect("bold");
    d.replace_range(pos(3, 1), pos(3, 2), "Z").expect("replace");
    d.restore(&snap).expect("restore");
    assert_eq!(xml(&d), original);

    // A snapshot spanning a table boundary cannot be captured.
    assert!(d.capture(2, 3).is_err());
}

#[test]
fn edit_log_is_truncated_by_restore() {
    let mut d =
        doc("<w:p><w:r><w:t>a</w:t></w:r><w:r><w:drawing/></w:r><w:r><w:t>b</w:t></w:r></w:p>");
    let snap = d.capture(0, 0).expect("capture");
    d.delete_range(pos(0, 0), pos(0, 2)).expect("delete");
    assert_eq!(d.edit_log().len(), 1);
    d.restore(&snap).expect("restore");
    assert!(d.edit_log().is_empty());
}
