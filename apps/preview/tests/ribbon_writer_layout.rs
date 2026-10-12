//! Layout tab controls run through `Shell::run_command` change the document.

#![allow(clippy::unwrap_used)]

use open365_docx::{DocPos, Document, Orientation, SectionType};
use open365_opc::Limits;
use open365_preview::doc::WriterPane;
use open365_preview::ribbon::Handled;
use open365_preview::{AppKind, Preview};

fn rig(text: &str) -> Preview {
    let mut p = Preview::new(AppKind::Writer, true, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text(text);
    p.shell.set_writer(&mut p.ui, pane);
    p
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

/// The saved file, reopened.
fn saved(p: &mut Preview, name: &str) -> Document {
    let path = std::env::temp_dir().join(format!("o365-rw2-{}-{name}.docx", std::process::id()));
    assert!(p.shell.writer_mut().unwrap().save_to(path.clone()), "save");
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    Document::open(&bytes, &Limits::default()).unwrap()
}

fn main_xml(p: &mut Preview, name: &str) -> String {
    String::from_utf8(saved(p, name).serialize_main()).unwrap()
}

fn pane(p: &mut Preview) -> &mut WriterPane {
    p.shell.writer_mut().unwrap()
}

#[test]
fn margins_presets_change_the_section_and_undo() {
    let mut p = rig("hello");
    run(&mut p, "layout.page-set-up.margins.narrow");
    let m = pane(&mut p).caret_section().unwrap().margins;
    assert_eq!(
        (m.top_twips, m.left_twips, m.right_twips, m.bottom_twips),
        (720, 720, 720, 720)
    );
    run(&mut p, "layout.page-set-up.margins.moderate");
    let m = pane(&mut p).caret_section().unwrap().margins;
    assert_eq!((m.top_twips, m.left_twips), (1440, 1080));
    run(&mut p, "layout.page-set-up.margins.wide");
    assert_eq!(
        pane(&mut p).caret_section().unwrap().margins.left_twips,
        2880
    );
    assert!(pane(&mut p).undo());
    assert_eq!(
        pane(&mut p).caret_section().unwrap().margins.left_twips,
        1080
    );
    assert_eq!(
        saved(&mut p, "margins").sections()[0].margins.left_twips,
        1080
    );
}

#[test]
fn mirrored_margins_set_the_setting_and_custom_margins_take_values() {
    let mut p = rig("hello");
    run(&mut p, "layout.page-set-up.margins.mirrored");
    let d = saved(&mut p, "mirror");
    let settings = d
        .package()
        .part("word/settings.xml")
        .unwrap()
        .data()
        .to_vec();
    assert!(
        String::from_utf8(settings)
            .unwrap()
            .contains("mirrorMargins")
    );
    assert_eq!(d.sections()[0].margins.left_twips, 1800);
    run(
        &mut p,
        "layout.page-set-up.margins.custom=1.5\",1\",2 cm,2 cm",
    );
    let m = pane(&mut p).caret_section().unwrap().margins;
    assert_eq!((m.top_twips, m.bottom_twips), (2160, 1440));
    assert_eq!(m.left_twips, 1134);
}

#[test]
fn orientation_swaps_the_page_and_turns_the_margins() {
    let mut p = rig("hello");
    run(&mut p, "layout.page-set-up.margins.moderate");
    let (_, _, w0, h0) = pane(&mut p).page_rect(0).unwrap();
    assert!(w0 < h0);
    run(&mut p, "layout.page-set-up.orientation.landscape");
    let s = pane(&mut p).caret_section().unwrap();
    assert_eq!(s.page.orient, Orientation::Landscape);
    assert_eq!((s.page.width_twips, s.page.height_twips), (16838, 11906));
    // Moderate (1" top and bottom, 0.75" sides) turned a quarter turn.
    assert_eq!((s.margins.top_twips, s.margins.left_twips), (1080, 1440));
    let (_, _, w1, h1) = pane(&mut p).page_rect(0).unwrap();
    assert!(w1 > h1, "the page is drawn landscape");
    run(&mut p, "layout.page-set-up.orientation.portrait");
    let s = pane(&mut p).caret_section().unwrap();
    assert_eq!(s.page.orient, Orientation::Portrait);
    assert_eq!((s.margins.top_twips, s.margins.left_twips), (1440, 1080));
}

#[test]
fn size_presets_keep_the_orientation() {
    let mut p = rig("hello");
    run(&mut p, "layout.page-set-up.size.letter");
    let s = pane(&mut p).caret_section().unwrap();
    assert_eq!(
        (s.page.width_twips, s.page.height_twips, s.page.code),
        (12240, 15840, Some(1))
    );
    run(&mut p, "layout.page-set-up.orientation.landscape");
    run(&mut p, "layout.page-set-up.size.legal");
    let s = pane(&mut p).caret_section().unwrap();
    assert_eq!((s.page.width_twips, s.page.height_twips), (20160, 12240));
    run(&mut p, "layout.page-set-up.size.custom=5\",7\"");
    let s = pane(&mut p).caret_section().unwrap();
    assert_eq!(
        (s.page.width_twips, s.page.height_twips),
        (10080, 7200),
        "landscape keeps the long side across"
    );
    assert_eq!(s.page.code, None);
}

#[test]
fn columns_presets_and_more_columns() {
    let mut p = rig("hello");
    run(&mut p, "layout.page-set-up.columns.two");
    let s = pane(&mut p).caret_section().unwrap();
    assert_eq!(
        (
            s.columns.count,
            s.columns.space_twips,
            s.columns.equal_width
        ),
        (2, 720, true)
    );
    run(&mut p, "layout.page-set-up.columns.left");
    let s = pane(&mut p).caret_section().unwrap();
    assert_eq!(s.columns.count, 2);
    assert!(!s.columns.equal_width);
    assert!(s.columns.columns[0].width_twips < s.columns.columns[1].width_twips);
    run(&mut p, "layout.page-set-up.columns.more=3,0.25\",line");
    let s = pane(&mut p).caret_section().unwrap();
    assert_eq!(
        (s.columns.count, s.columns.space_twips, s.columns.separator),
        (3, 360, true)
    );
    run(&mut p, "layout.page-set-up.columns.one");
    assert_eq!(pane(&mut p).caret_section().unwrap().columns.count, 1);
}

#[test]
fn page_and_column_breaks_split_the_paragraph() {
    let mut p = rig("hello world");
    pane(&mut p).set_caret(DocPos { para: 0, offset: 5 });
    assert_eq!(pane(&mut p).page_count(), 1);
    run(&mut p, "layout.page-set-up.breaks.page");
    assert_eq!(pane(&mut p).paragraph_count(), 2);
    assert_eq!(pane(&mut p).paragraph_text(1), Some(" world"));
    assert_eq!(pane(&mut p).caret().para, 1);
    assert_eq!(
        pane(&mut p).page_count(),
        2,
        "the text after the break starts a page"
    );
    assert!(pane(&mut p).undo());
    assert_eq!(pane(&mut p).paragraph_count(), 1);
    assert_eq!(pane(&mut p).page_count(), 1);
    pane(&mut p).set_caret(DocPos { para: 0, offset: 5 });
    run(&mut p, "layout.page-set-up.breaks.column");
    assert_eq!(pane(&mut p).paragraph_count(), 2);
    run(&mut p, "layout.page-set-up.breaks.text-wrapping");
    assert!(pane(&mut p).paragraph_text(1).unwrap().contains('\n'));
}

#[test]
fn section_breaks_start_new_sections() {
    let mut p = rig("hello world");
    pane(&mut p).set_caret(DocPos { para: 0, offset: 5 });
    run(&mut p, "layout.page-set-up.breaks.continuous");
    assert_eq!(pane(&mut p).section_count(), 2);
    assert_eq!(
        pane(&mut p).caret_section().unwrap().kind,
        SectionType::Continuous
    );
    run(&mut p, "layout.page-set-up.orientation.landscape");
    let d = saved(&mut p, "sections");
    let s = d.sections();
    assert_eq!(s.len(), 2);
    assert_eq!(
        s[0].page.orient,
        Orientation::Portrait,
        "only the caret's section turned"
    );
    assert_eq!(s[1].page.orient, Orientation::Landscape);
    assert_eq!(s[1].kind, SectionType::Continuous);
    run(&mut p, "layout.page-set-up.breaks.odd-page");
    assert_eq!(pane(&mut p).section_count(), 3);
    assert_eq!(
        pane(&mut p).caret_section().unwrap().kind,
        SectionType::OddPage
    );
    assert!(pane(&mut p).undo());
    assert_eq!(pane(&mut p).section_count(), 2);
}

#[test]
fn line_numbers_and_suppress() {
    let mut p = rig("hello");
    run(&mut p, "layout.page-set-up.line-numbers.restart-page");
    let xml = main_xml(&mut p, "ln");
    assert!(
        xml.contains("lnNumType") && xml.contains("newPage"),
        "{xml}"
    );
    run(
        &mut p,
        "layout.page-set-up.line-numbers.options=5,3,0.25\",s",
    );
    let xml = main_xml(&mut p, "ln2");
    assert!(
        xml.contains("countBy=\"5\"") && xml.contains("start=\"2\"") && xml.contains("newSection"),
        "{xml}"
    );
    assert!(xml.contains("distance=\"360\""), "{xml}");
    run(&mut p, "layout.page-set-up.line-numbers.suppress");
    assert!(pane(&mut p).all_suppressed(&[0]));
    run(&mut p, "layout.page-set-up.line-numbers.suppress");
    assert!(!pane(&mut p).all_suppressed(&[0]), "the item toggles");
    run(&mut p, "layout.page-set-up.line-numbers.none");
    assert!(!main_xml(&mut p, "ln3").contains("lnNumType"));
}

#[test]
fn hyphenation_switches_automatic_on_and_off() {
    let mut p = rig("hello");
    assert!(!pane(&mut p).hyphenation_on());
    run(&mut p, "layout.page-set-up.hyphenation.automatic");
    assert!(pane(&mut p).hyphenation_on());
    run(&mut p, "layout.page-set-up.hyphenation.options=0.25\",2,0");
    assert!(pane(&mut p).hyphenation_on(), "options keep automatic on");
    let d = saved(&mut p, "hy");
    assert!(d.settings().auto_hyphenation);
    let xml = String::from_utf8(
        d.package()
            .part("word/settings.xml")
            .unwrap()
            .data()
            .to_vec(),
    )
    .unwrap();
    assert!(
        xml.contains("hyphenationZone")
            && xml.contains("w:val=\"360\"")
            && xml.contains("consecutiveHyphenLimit")
            && xml.contains("doNotHyphenateCaps"),
        "{xml}"
    );
    run(&mut p, "layout.page-set-up.hyphenation.none");
    assert!(!pane(&mut p).hyphenation_on());
    run(&mut p, "layout.page-set-up.hyphenation.manual");
    assert!(
        pane(&mut p)
            .status
            .as_deref()
            .unwrap_or_default()
            .contains("Manual")
    );
}

#[test]
fn indent_and_spacing_spinners_edit_the_selected_paragraphs() {
    let mut p = rig("one");
    pane(&mut p).enter(false);
    pane(&mut p).insert_text("two");
    pane(&mut p).select_all();
    run(&mut p, "layout.paragraph.indent-left=0.5\"");
    run(&mut p, "layout.paragraph.indent-right=1 cm");
    run(&mut p, "layout.paragraph.spacing-before=6 pt");
    run(&mut p, "layout.paragraph.spacing-after=12");
    let xml = main_xml(&mut p, "para");
    assert_eq!(xml.matches("w:left=\"720\"").count(), 2, "{xml}");
    assert_eq!(xml.matches("w:right=\"567\"").count(), 2, "{xml}");
    assert_eq!(xml.matches("w:before=\"120\"").count(), 2, "{xml}");
    assert_eq!(xml.matches("w:after=\"240\"").count(), 2, "{xml}");
    pane(&mut p).set_caret(DocPos { para: 0, offset: 0 });
    run(&mut p, "layout.paragraph.indent-left=0");
    let xml = main_xml(&mut p, "para2");
    assert_eq!(
        xml.matches("w:left=\"720\"").count(),
        1,
        "a caret edits one paragraph: {xml}"
    );
    assert!(pane(&mut p).undo());
    let xml = main_xml(&mut p, "para3");
    assert_eq!(xml.matches("w:left=\"720\"").count(), 2, "{xml}");
    run(&mut p, "layout.paragraph.indent-left=wide");
    assert!(
        pane(&mut p)
            .status
            .as_deref()
            .unwrap_or_default()
            .contains("Indent")
    );
}

#[test]
fn every_menu_item_is_handled() {
    use open365_preview::ribbon::writer::layout::{IDS, menu_items};
    let mut p = rig("hello");
    for id in IDS {
        let Some(items) = menu_items(id) else {
            continue;
        };
        for it in items.iter().filter(|i| !i.separator && !i.heading) {
            let typed = format!("{}=1", it.id);
            let yes = [it.id.clone(), typed]
                .iter()
                .any(|i| p.shell.run_command(&mut p.ui, i) == Handled::Yes);
            assert!(yes, "menu item {} is not handled", it.id);
        }
    }
}
