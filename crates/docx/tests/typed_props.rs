//! Typed, style-resolved paragraph, run, section and settings accessors, on inline XML and
#![allow(clippy::unwrap_used)]
//! on the Word-made fixtures.

mod common;

use open365_docx::{
    Columns, DEFAULT_COMPATIBILITY_MODE, DocGridType, Document, Inline, Ligatures, Orientation,
    Run, SectionType, TabKind, TabLeader,
};
use open365_opc::Limits;

use common::{W, build, fixtures, root_rels};

const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn styles_xml(inner: &str) -> String {
    format!(r#"<w:styles xmlns:w="{W}">{inner}</w:styles>"#)
}

/// Package with `body`, optional styles and optional settings parts.
fn doc_with(body: &str, styles: Option<&str>, settings: Option<&str>) -> Document {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:w14="{W14}"><w:body>{body}</w:body></w:document>"#
    );
    let mut rels = String::new();
    let mut extra: Vec<(&str, Vec<u8>)> = Vec::new();
    if let Some(s) = styles {
        rels.push_str(&format!(
            r#"<Relationship Id="rId1" Type="{REL}/styles" Target="styles.xml"/>"#
        ));
        extra.push(("word/styles.xml", s.as_bytes().to_vec()));
    }
    if let Some(s) = settings {
        rels.push_str(&format!(
            r#"<Relationship Id="rId2" Type="{REL}/settings" Target="settings.xml"/>"#
        ));
        extra.push(("word/settings.xml", s.as_bytes().to_vec()));
    }
    let doc_rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
    );
    extra.push(("word/_rels/document.xml.rels", doc_rels.into_bytes()));
    let extra_refs: Vec<(&str, &[u8])> = extra.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    let bytes = build(
        &document,
        "word/document.xml",
        &root_rels("word/document.xml", ""),
        &extra_refs,
    );
    Document::open(&bytes, &Limits::default()).expect("opens")
}

fn first_run(doc: &Document, para: usize) -> &Run {
    doc.paragraph(para)
        .expect("paragraph")
        .content
        .iter()
        .find_map(|i| match i {
            Inline::Run(r) => Some(r),
            _ => None,
        })
        .expect("run")
}

#[test]
fn paragraph_defaults_without_styles() {
    let doc = doc_with(r#"<w:p><w:r><w:t>x</w:t></w:r></w:p>"#, None, None);
    assert!(doc.styles().is_none());
    // No styles part: the resolver is absent, so use a minimal one to check defaults.
    let doc = doc_with(
        r#"<w:p><w:r><w:t>x</w:t></w:r></w:p>"#,
        Some(&styles_xml("")),
        None,
    );
    let p = doc
        .styles()
        .unwrap()
        .paragraph_flow_props(doc.paragraph(0).unwrap());
    assert!(p.widow_control, "widowControl is on when never specified");
    assert!(!p.keep_next && !p.keep_lines && !p.page_break_before);
    assert!(!p.contextual_spacing && !p.suppress_auto_hyphens);
    assert!(p.tabs.is_empty());
}

#[test]
fn paragraph_flags_resolve_defaults_style_chain_then_direct() {
    let styles = styles_xml(
        r#"<w:docDefaults><w:pPrDefault><w:pPr><w:widowControl w:val="0"/><w:suppressAutoHyphens/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Base"><w:name w:val="Base"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:keepLines/><w:pageBreakBefore/><w:contextualSpacing/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Child"><w:name w:val="Child"/><w:basedOn w:val="Base"/><w:pPr><w:keepNext w:val="0"/><w:widowControl/></w:pPr></w:style>"#,
    );
    let body = r#"<w:p><w:pPr><w:pStyle w:val="Base"/></w:pPr></w:p>
<w:p><w:pPr><w:pStyle w:val="Child"/></w:pPr></w:p>
<w:p><w:pPr><w:pStyle w:val="Child"/><w:keepLines w:val="false"/><w:suppressAutoHyphens w:val="0"/></w:pPr></w:p>
<w:p/>"#;
    let doc = doc_with(body, Some(&styles), None);
    let s = doc.styles().unwrap();
    let props = |i: usize| s.paragraph_flow_props(doc.paragraph(i).unwrap());

    let base = props(0);
    assert!(!base.widow_control, "docDefaults turns widow control off");
    assert!(base.keep_next && base.keep_lines && base.page_break_before);
    assert!(base.contextual_spacing && base.suppress_auto_hyphens);

    let child = props(1);
    assert!(child.widow_control, "child style turns it back on");
    assert!(!child.keep_next, "w:val=0 overrides the base style");
    assert!(child.keep_lines && child.page_break_before);

    let direct = props(2);
    assert!(!direct.keep_lines, "direct w:val=false wins");
    assert!(!direct.suppress_auto_hyphens);

    let plain = props(3);
    assert!(!plain.widow_control && plain.suppress_auto_hyphens);
    assert!(!plain.keep_next);
}

#[test]
fn tab_stops_merge_across_levels_and_clear() {
    let styles = styles_xml(
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/>
<w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/><w:tab w:val="right" w:leader="dot" w:pos="9000"/></w:tabs></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Wide"><w:name w:val="Wide"/><w:basedOn w:val="Normal"/>
<w:pPr><w:tabs><w:tab w:val="clear" w:pos="720"/><w:tab w:val="decimal" w:leader="underscore" w:pos="5000"/></w:tabs></w:pPr></w:style>"#,
    );
    let body = r#"<w:p/>
<w:p><w:pPr><w:pStyle w:val="Wide"/></w:pPr></w:p>
<w:p><w:pPr><w:pStyle w:val="Wide"/><w:tabs><w:tab w:val="clear" w:pos="9000"/><w:tab w:val="center" w:leader="hyphen" w:pos="5000"/><w:tab w:val="bar" w:pos="-100"/><w:tab w:val="end" w:leader="heavy" w:pos="10000"/><w:tab w:val="num" w:leader="middleDot" w:pos="200"/></w:tabs></w:pPr></w:p>"#;
    let doc = doc_with(body, Some(&styles), None);
    let s = doc.styles().unwrap();
    let tabs = |i: usize| s.paragraph_flow_props(doc.paragraph(i).unwrap()).tabs;

    let a = tabs(0);
    assert_eq!(a.len(), 2);
    assert_eq!((a[0].pos_twips, a[0].kind), (720, TabKind::Left));
    assert_eq!(a[0].leader, TabLeader::None);
    assert_eq!(
        (a[1].pos_pt(), a[1].kind, a[1].leader),
        (450.0, TabKind::Right, TabLeader::Dot)
    );

    let b = tabs(1);
    assert_eq!(
        b.iter().map(|t| t.pos_twips).collect::<Vec<_>>(),
        [5000, 9000]
    );
    assert_eq!(
        (b[0].kind, b[0].leader),
        (TabKind::Decimal, TabLeader::Underscore)
    );

    let c = tabs(2);
    assert_eq!(
        c.iter()
            .map(|t| (t.pos_twips, t.kind, t.leader))
            .collect::<Vec<_>>(),
        [
            (-100, TabKind::Bar, TabLeader::None),
            (200, TabKind::Num, TabLeader::MiddleDot),
            (5000, TabKind::Center, TabLeader::Hyphen),
            (10000, TabKind::Right, TabLeader::Heavy),
        ]
    );
}

#[test]
fn paragraph_mark_run_properties_resolve_through_the_style() {
    let styles = styles_xml(
        r#"<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="22"/><w:spacing w:val="10"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Big"><w:name w:val="Big"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="40"/></w:rPr></w:style>"#,
    );
    let body = r#"<w:p/>
<w:p><w:pPr><w:pStyle w:val="Big"/></w:pPr></w:p>
<w:p><w:pPr><w:pStyle w:val="Big"/><w:rPr><w:b w:val="0"/><w:i/><w:sz w:val="18"/><w:kern w:val="24"/><w:position w:val="-4"/></w:rPr></w:pPr></w:p>"#;
    let doc = doc_with(body, Some(&styles), None);
    let s = doc.styles().unwrap();
    let m0 = s.paragraph_mark_props(doc.paragraph(0).unwrap());
    assert_eq!((m0.run.size_half_points, m0.run.bold), (22, false));
    assert_eq!(m0.layout.spacing_twips, 10);
    let m1 = s.paragraph_mark_props(doc.paragraph(1).unwrap());
    assert_eq!((m1.run.size_half_points, m1.run.bold), (40, true));
    let m2 = s.paragraph_mark_props(doc.paragraph(2).unwrap());
    assert_eq!(
        (m2.run.size_half_points, m2.run.bold, m2.run.italic),
        (18, false, true)
    );
    assert_eq!(
        (m2.layout.kern_half_points, m2.layout.position_half_points),
        (24, -4)
    );
}

#[test]
fn run_layout_defaults() {
    let doc = doc_with(
        r#"<w:p><w:r><w:t>x</w:t></w:r></w:p>"#,
        Some(&styles_xml("")),
        None,
    );
    let r = doc
        .styles()
        .unwrap()
        .effective_run_layout(doc.paragraph(0).unwrap(), first_run(&doc, 0));
    assert_eq!(r.kern_half_points, 0);
    assert!(r.ligatures.is_none());
    assert_eq!((r.position_half_points, r.spacing_twips), (0, 0));
    assert_eq!(r.scale_percent, 100);
}

#[test]
fn run_layout_resolves_defaults_styles_and_direct_formatting() {
    let styles = styles_xml(
        r#"<w:docDefaults><w:rPrDefault><w:rPr><w:kern w:val="28"/><w:w w:val="90"/><w14:ligatures xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" w14:val="standard"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Spaced"><w:name w:val="Spaced"/><w:basedOn w:val="Normal"/><w:rPr><w:spacing w:val="-20"/><w:position w:val="6"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Wide"><w:name w:val="Wide"/><w:rPr><w:w w:val="150"/><w:spacing w:val="40"/></w:rPr></w:style>"#,
    );
    let body = r#"<w:p><w:r><w:t>plain</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Spaced"/></w:pPr><w:r><w:t>styled</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Spaced"/></w:pPr><w:r><w:rPr><w:rStyle w:val="Wide"/></w:rPr><w:t>char style</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:kern w:val="0"/><w:w w:val="50"/><w:position w:val="-3"/><w:spacing w:val="5"/><w14:ligatures w14:val="standardContextual"/></w:rPr><w:t>direct</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w14:ligatures w14:val="none"/></w:rPr><w:t>off</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w14:ligatures w14:val="all"/></w:rPr><w:t>all</w:t></w:r></w:p>"#;
    let doc = doc_with(body, Some(&styles), None);
    let s = doc.styles().unwrap();
    let layout = |i: usize| s.effective_run_layout(doc.paragraph(i).unwrap(), first_run(&doc, i));

    let plain = layout(0);
    assert_eq!((plain.kern_half_points, plain.scale_percent), (28, 90));
    assert_eq!(
        plain.ligatures,
        Ligatures {
            standard: true,
            ..Ligatures::default()
        }
    );

    let styled = layout(1);
    assert_eq!(
        (styled.spacing_twips, styled.position_half_points),
        (-20, 6)
    );
    assert_eq!(styled.spacing_pt(), -1.0);
    assert_eq!(styled.position_pt(), 3.0);
    assert_eq!(styled.scale_percent, 90, "document default still applies");

    let chr = layout(2);
    assert_eq!((chr.scale_percent, chr.spacing_twips), (150, 40));
    assert_eq!(chr.position_half_points, 6, "paragraph style value kept");

    let direct = layout(3);
    assert_eq!(direct.kern_half_points, 0);
    assert_eq!(
        (
            direct.scale_percent,
            direct.position_half_points,
            direct.spacing_twips
        ),
        (50, -3, 5)
    );
    assert_eq!(
        direct.ligatures,
        Ligatures {
            standard: true,
            contextual: true,
            historical: false,
            discretionary: false
        }
    );

    assert!(layout(4).ligatures.is_none());
    let all = layout(5).ligatures;
    assert!(all.standard && all.contextual && all.historical && all.discretionary);
}

#[test]
fn sections_default_when_no_sect_pr() {
    let doc = doc_with(r#"<w:p/>"#, None, None);
    let sections = doc.sections();
    assert_eq!(sections.len(), 1);
    let s = &sections[0];
    assert_eq!(s.end_paragraph, None);
    assert_eq!((s.page.width_twips, s.page.height_twips), (12240, 15840));
    assert_eq!(s.page.orient, Orientation::Portrait);
    assert_eq!(s.columns, Columns::default());
    assert_eq!(s.columns.count, 1);
    assert_eq!(s.columns.space_twips, 720);
    assert!(s.columns.equal_width && !s.columns.separator);
    assert!(!s.title_pg);
    assert_eq!(s.kind, SectionType::NextPage);
    assert_eq!(s.doc_grid.kind, DocGridType::Default);
    assert_eq!(s.margins.gutter_twips, 0);
}

#[test]
fn sections_report_each_end_paragraph_and_properties() {
    let body = r#"<w:p/>
<w:p><w:pPr><w:sectPr><w:type w:val="continuous"/><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1000" w:right="1100" w:bottom="1200" w:left="1300" w:header="400" w:footer="500" w:gutter="60"/><w:cols w:num="2" w:space="360" w:sep="1"/><w:titlePg/><w:docGrid w:type="linesAndChars" w:linePitch="312" w:charSpace="-100"/></w:sectPr></w:pPr></w:p>
<w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>
<w:p><w:pPr><w:sectPr><w:type w:val="oddPage"/><w:pgSz w:w="15840" w:h="12240" w:orient="landscape" w:code="1"/><w:cols w:num="3" w:equalWidth="0"><w:col w:w="2000" w:space="200"/><w:col w:w="3000"/><w:col w:w="4000" w:space="0"/></w:cols><w:titlePg w:val="0"/></w:sectPr></w:pPr></w:p>
<w:p/>
<w:sectPr><w:type w:val="nextColumn"/></w:sectPr>"#;
    let doc = doc_with(body, None, None);
    let sections = doc.sections();
    assert_eq!(sections.len(), 3);
    assert_eq!(
        sections
            .iter()
            .map(|s| (s.index, s.end_paragraph))
            .collect::<Vec<_>>(),
        [(0, Some(1)), (1, Some(3)), (2, None)]
    );

    let a = &sections[0];
    assert_eq!(a.kind, SectionType::Continuous);
    assert_eq!((a.page.width_pt(), a.page.height_pt()), (595.3, 841.9));
    assert_eq!(
        (
            a.margins.top_twips,
            a.margins.right_twips,
            a.margins.bottom_twips,
            a.margins.left_twips,
            a.margins.header_twips,
            a.margins.footer_twips,
            a.margins.gutter_twips
        ),
        (1000, 1100, 1200, 1300, 400, 500, 60)
    );
    assert_eq!(
        (a.columns.count, a.columns.space_twips, a.columns.separator),
        (2, 360, true)
    );
    assert!(a.title_pg);
    assert_eq!(a.doc_grid.kind, DocGridType::LinesAndChars);
    assert_eq!(
        (a.doc_grid.line_pitch_twips, a.doc_grid.char_space),
        (Some(312), -100)
    );

    let b = &sections[1];
    assert_eq!(b.kind, SectionType::OddPage);
    assert_eq!(b.page.orient, Orientation::Landscape);
    assert_eq!(
        (b.page.width_twips, b.page.height_twips, b.page.code),
        (15840, 12240, Some(1))
    );
    assert!(!b.columns.equal_width);
    assert_eq!(b.columns.columns.len(), 3);
    assert_eq!(b.columns.columns[0].space_twips, Some(200));
    assert_eq!(b.columns.columns[1].space_twips, None);
    assert!(!b.title_pg, "titlePg w:val=0 is off");
    assert_eq!(
        b.margins.top_twips, 1440,
        "absent pgMar uses the documented default"
    );

    let c = &sections[2];
    assert_eq!(c.kind, SectionType::NextColumn);
    assert_eq!(c.page.width_twips, 12240);
}

#[test]
fn settings_defaults_when_absent() {
    let doc = doc_with(r#"<w:p/>"#, None, None);
    let s = doc.settings();
    assert_eq!(s.default_tab_stop_twips, 720);
    assert_eq!(s.default_tab_stop_pt(), 36.0);
    assert_eq!(s.compatibility_mode, DEFAULT_COMPATIBILITY_MODE);
    assert!(!s.even_and_odd_headers && !s.auto_hyphenation);

    let doc = doc_with(
        r#"<w:p/>"#,
        None,
        Some(&format!(r#"<w:settings xmlns:w="{W}"/>"#)),
    );
    assert_eq!(doc.settings().default_tab_stop_twips, 720);
}

#[test]
fn settings_read_each_value() {
    let xml = format!(
        r#"<w:settings xmlns:w="{W}"><w:evenAndOddHeaders/><w:defaultTabStop w:val="425"/><w:autoHyphenation w:val="true"/><w:compat><w:compatSetting w:name="other" w:uri="u" w:val="99"/><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="14"/></w:compat></w:settings>"#
    );
    let doc = doc_with(r#"<w:p/>"#, None, Some(&xml));
    let s = doc.settings();
    assert_eq!(s.default_tab_stop_twips, 425);
    assert_eq!(s.compatibility_mode, 14);
    assert!(s.even_and_odd_headers && s.auto_hyphenation);

    let off = format!(
        r#"<w:settings xmlns:w="{W}"><w:evenAndOddHeaders w:val="0"/><w:autoHyphenation w:val="off"/><w:defaultTabStop w:val="0"/></w:settings>"#
    );
    let doc = doc_with(r#"<w:p/>"#, None, Some(&off));
    let s = doc.settings();
    assert!(!s.even_and_odd_headers && !s.auto_hyphenation);
    assert_eq!(
        s.default_tab_stop_twips, 720,
        "zero is invalid, default kept"
    );
}

#[test]
fn fixtures_expose_typed_properties() {
    let all = fixtures();
    assert!(!all.is_empty());
    for (name, bytes) in &all {
        let doc = Document::open(bytes, &Limits::default()).expect("opens");
        let settings = doc.settings();
        assert!(settings.default_tab_stop_twips > 0, "{name}");
        assert!(
            (11..=15).contains(&settings.compatibility_mode),
            "{name}: {}",
            settings.compatibility_mode
        );
        let sections = doc.sections();
        assert!(!sections.is_empty(), "{name}");
        assert_eq!(sections.last().unwrap().end_paragraph, None, "{name}");
        if let Some(styles) = doc.styles() {
            for i in 0..doc.paragraph_count() {
                let p = doc.paragraph(i).unwrap();
                let _ = styles.paragraph_flow_props(p);
                let _ = styles.paragraph_mark_props(p);
            }
        }
    }
    let find = |n: &str| {
        let (_, bytes) = all.iter().find(|(name, _)| name == n).expect(n);
        Document::open(bytes, &Limits::default()).unwrap()
    };

    let tabs = find("layout-tabs");
    let styles = tabs.styles().unwrap();
    let stops: Vec<_> = (0..tabs.paragraph_count())
        .flat_map(|i| styles.paragraph_flow_props(tabs.paragraph(i).unwrap()).tabs)
        .collect();
    assert!(
        stops
            .iter()
            .any(|t| t.pos_twips == 2880 && t.kind == TabKind::Left)
    );
    assert!(
        stops
            .iter()
            .any(|t| t.pos_twips == 4680 && t.kind == TabKind::Center)
    );
    assert!(stops.iter().any(|t| t.leader == TabLeader::Dot));

    let landscape = find("sections-landscape").sections();
    assert_eq!(landscape.len(), 2);
    assert_eq!(landscape[0].page.orient, Orientation::Portrait);
    assert_eq!(
        (
            landscape[0].page.width_twips,
            landscape[0].page.height_twips
        ),
        (11906, 16838)
    );
    assert_eq!(landscape[1].page.orient, Orientation::Landscape);
    assert_eq!(
        (
            landscape[1].page.width_twips,
            landscape[1].page.height_twips
        ),
        (16838, 11906)
    );
    assert!(landscape[0].end_paragraph.is_some());
    assert_eq!(landscape[0].margins.header_twips, 708);
    assert_eq!(landscape[0].doc_grid.line_pitch_twips, Some(360));

    let pag = find("layout-pagination");
    let styles = pag.styles().unwrap();
    let flags: Vec<_> = (0..pag.paragraph_count())
        .map(|i| styles.paragraph_flow_props(pag.paragraph(i).unwrap()))
        .collect();
    assert!(
        flags
            .iter()
            .any(|p| p.keep_next || p.keep_lines || p.page_break_before)
    );
}
