//! Design tab controls run through `Shell::run_command` change the document.

#![allow(clippy::unwrap_used)]

use open365_docx::{DocPos, Document};
use open365_opc::Limits;
use open365_preview::doc::WriterPane;
use open365_preview::ribbon::Handled;
use open365_preview::{AppKind, Preview};

mod dialogs_common;

fn rig() -> Preview {
    let mut p = Preview::new(AppKind::Writer, true, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text("hello");
    p.shell.set_writer(&mut p.ui, pane);
    p
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn saved(p: &mut Preview, name: &str) -> Document {
    let path = std::env::temp_dir().join(format!("o365-rw2d-{}-{name}.docx", std::process::id()));
    assert!(p.shell.writer_mut().unwrap().save_to(path.clone()), "save");
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    Document::open(&bytes, &Limits::default()).unwrap()
}

fn part(d: &Document, name: &str) -> String {
    let p = d
        .package()
        .part(name)
        .unwrap_or_else(|| panic!("no {name}"));
    String::from_utf8(p.data().to_vec()).unwrap()
}

fn main_xml(p: &mut Preview, name: &str) -> String {
    String::from_utf8(saved(p, name).serialize_main()).unwrap()
}

fn pane(p: &mut Preview) -> &mut WriterPane {
    p.shell.writer_mut().unwrap()
}

fn style_block(styles: &str, id: &str) -> String {
    let from = &styles[styles.find(&format!("w:styleId=\"{id}\"")).unwrap()..];
    from[..from.find("</w:style>").unwrap()].to_owned()
}

#[test]
fn themes_colours_fonts_and_effects_edit_the_theme_part() {
    let mut p = rig();
    assert!(
        pane(&mut p).theme_colours().is_none(),
        "the blank document has no theme"
    );
    run(&mut p, "design.document-formatting.themes.harbour");
    let d = saved(&mut p, "theme");
    let theme = part(&d, "word/theme/theme1.xml");
    assert!(
        theme.contains("name=\"Harbour\"") && theme.contains("typeface=\"Arial\""),
        "{theme}"
    );
    assert_eq!(d.styles().unwrap().theme_fonts().major, "Arial");
    assert_eq!(pane(&mut p).theme_colours().unwrap()[4], "3B6EA8");

    run(&mut p, "design.document-formatting.colours.forest");
    assert_eq!(pane(&mut p).theme_colours().unwrap()[4], "3E7C4E");
    let theme = part(&saved(&mut p, "colours"), "word/theme/theme1.xml");
    assert!(
        theme.contains("typeface=\"Arial\""),
        "the fonts stay: {theme}"
    );

    run(&mut p, "design.document-formatting.fonts.editorial");
    let d = saved(&mut p, "fonts");
    assert_eq!(d.styles().unwrap().theme_fonts().minor, "Georgia");
    assert_eq!(
        pane(&mut p).theme_colours().unwrap()[4],
        "3E7C4E",
        "the colours stay"
    );

    run(&mut p, "design.document-formatting.effects.intense");
    let theme = part(&saved(&mut p, "effects"), "word/theme/theme1.xml");
    assert_eq!(theme.matches("<a:outerShdw").count(), 3, "{theme}");
    assert!(pane(&mut p).undo());
    let theme = part(&saved(&mut p, "effects-undo"), "word/theme/theme1.xml");
    assert_eq!(
        theme.matches("<a:outerShdw").count(),
        0,
        "back to the Harbour theme's flat effects: {theme}"
    );
}

#[test]
fn fonts_create_a_theme_when_the_document_has_none() {
    let mut p = rig();
    run(&mut p, "design.document-formatting.fonts.modern");
    let d = saved(&mut p, "nofont");
    assert!(d.package().part("word/theme/theme1.xml").is_some());
    assert_eq!(d.styles().unwrap().theme_fonts().minor, "Arial");
    assert!(part(&d, "[Content_Types].xml").contains("/word/theme/theme1.xml"));
    assert!(part(&d, "word/_rels/document.xml.rels").contains("theme/theme1.xml"));
}

#[test]
fn style_sets_redefine_the_heading_styles() {
    let mut p = rig();
    run(&mut p, "design.document-formatting.quickstylessets.centred");
    let styles = part(&saved(&mut p, "set"), "word/styles.xml");
    let h1 = style_block(&styles, "Heading1");
    assert!(
        h1.contains("w:jc w:val=\"center\"") && h1.contains("<w:caps/>"),
        "{h1}"
    );
    run(&mut p, "design.document-formatting.quickstylessets.lines");
    let styles = part(&saved(&mut p, "set2"), "word/styles.xml");
    let h1 = style_block(&styles, "Heading1");
    assert!(h1.contains("w:pBdr") && !h1.contains("w:jc"), "{h1}");
    assert_eq!(styles.matches("w:styleId=\"Heading1\"").count(), 1);
}

#[test]
fn paragraph_spacing_presets_write_the_document_defaults() {
    let mut p = rig();
    run(
        &mut p,
        "design.document-formatting.paragraph-spacing.compact",
    );
    let styles = part(&saved(&mut p, "sp"), "word/styles.xml");
    assert!(
        styles.contains("w:after=\"80\"") && styles.contains("w:line=\"240\""),
        "{styles}"
    );
    run(
        &mut p,
        "design.document-formatting.paragraph-spacing.double",
    );
    let styles = part(&saved(&mut p, "sp2"), "word/styles.xml");
    assert!(
        styles.contains("w:after=\"160\"") && styles.contains("w:line=\"480\""),
        "{styles}"
    );
    run(
        &mut p,
        "design.document-formatting.paragraph-spacing.default",
    );
    let styles = part(&saved(&mut p, "sp3"), "word/styles.xml");
    assert!(styles.contains("w:line=\"259\""), "{styles}");
}

#[test]
fn watermark_goes_into_the_headers_and_comes_out() {
    let mut p = rig();
    assert_eq!(pane(&mut p).watermark_text(), None);
    run(
        &mut p,
        "design.page-background.watermark.confidential-diagonal",
    );
    assert_eq!(
        pane(&mut p).watermark_text().as_deref(),
        Some("CONFIDENTIAL")
    );
    let d = saved(&mut p, "wm");
    let header = part(&d, "word/header1.xml");
    assert!(
        header.contains("PowerPlusWaterMarkObject") && header.contains("rotation:315"),
        "{header}"
    );
    assert!(part(&d, "word/_rels/document.xml.rels").contains("header1.xml"));
    assert!(part(&d, "[Content_Types].xml").contains("/word/header1.xml"));
    assert!(
        String::from_utf8(d.serialize_main())
            .unwrap()
            .contains("headerReference")
    );
    run(&mut p, "design.page-background.watermark.draft-horizontal");
    assert_eq!(pane(&mut p).watermark_text().as_deref(), Some("DRAFT"));
    let header = part(&saved(&mut p, "wm2"), "word/header1.xml");
    assert_eq!(
        header.matches("PowerPlusWaterMarkObject").count(),
        1,
        "a second watermark replaces the first"
    );
    assert!(!header.contains("rotation:315"));
    run(
        &mut p,
        "design.page-background.watermark.custom=Internal|Arial|40|FF0000|d|0",
    );
    let header = part(&saved(&mut p, "wm3"), "word/header1.xml");
    assert!(
        header.contains("string=\"Internal\"")
            && header.contains("fillcolor=\"#FF0000\"")
            && !header.contains("opacity"),
        "{header}"
    );
    run(&mut p, "design.page-background.watermark.remove");
    assert_eq!(pane(&mut p).watermark_text(), None);
    assert!(pane(&mut p).undo());
    assert_eq!(pane(&mut p).watermark_text().as_deref(), Some("Internal"));
}

#[test]
fn watermark_covers_every_section() {
    let mut p = rig();
    pane(&mut p).set_caret(DocPos { para: 0, offset: 2 });
    run(&mut p, "layout.page-set-up.breaks.next-page");
    run(&mut p, "design.page-background.watermark.sample-diagonal");
    let s = saved(&mut p, "wm-sections").sections();
    assert_eq!(s.len(), 2);
    assert!(
        s.iter()
            .all(|s| s.header_footer_refs.iter().any(|r| !r.footer))
    );
}

#[test]
fn page_colour_sets_the_background_and_none_removes_it() {
    let mut p = rig();
    run(&mut p, "design.page-background.page-colour.custom=ffF2cc");
    assert_eq!(pane(&mut p).page_colour().as_deref(), Some("FFF2CC"));
    let d = saved(&mut p, "pc");
    let main = String::from_utf8(d.serialize_main()).unwrap();
    assert!(main.find("<w:background").unwrap() < main.find("<w:body").unwrap());
    assert!(part(&d, "word/settings.xml").contains("displayBackgroundShape"));
    run(&mut p, "design.page-background.page-colour.standard-green");
    assert_eq!(pane(&mut p).page_colour().as_deref(), Some("00B050"));
    run(&mut p, "design.page-background.page-colour.theme-accent2");
    let main = main_xml(&mut p, "pc2");
    assert!(
        main.contains("w:themeColor=\"accent2\"") && main.contains("w:color=\"ED7D31\""),
        "{main}"
    );
    run(&mut p, "design.page-background.page-colour.none");
    assert_eq!(pane(&mut p).page_colour(), None);
    assert!(!main_xml(&mut p, "pc3").contains("w:background"));
}

#[test]
fn page_borders_default_to_a_box_and_take_settings() {
    let mut p = rig();
    // The bare button opens Borders and Shading; Setting: Box with the default style, then OK.
    run(&mut p, "design.page-background.page-borders");
    assert!(p.shell.dialog_open());
    dialogs_common::alt(&mut p, 'b');
    dialogs_common::key(&mut p, open365_ui_core::Key::Enter);
    assert!(!p.shell.dialog_open());
    let main = main_xml(&mut p, "pb");
    for side in ["top", "left", "bottom", "right"] {
        let want =
            format!("<w:{side} w:val=\"single\" w:sz=\"4\" w:space=\"24\" w:color=\"auto\"/>");
        assert!(main.contains(&want), "{main}");
    }
    run(
        &mut p,
        "design.page-background.page-borders=shadow,double,3,FF0000,10,text,first",
    );
    let main = main_xml(&mut p, "pb2");
    for want in [
        "w:offsetFrom=\"text\"",
        "w:display=\"firstPage\"",
        "w:val=\"double\"",
        "w:shadow=\"1\"",
        "w:sz=\"24\"",
    ] {
        assert!(main.contains(want), "{want} in {main}");
    }
    assert_eq!(main.matches("<w:pgBorders").count(), 1);
    run(&mut p, "design.page-background.page-borders=none");
    assert!(!main_xml(&mut p, "pb3").contains("pgBorders"));
}

#[test]
fn every_menu_item_is_handled() {
    use open365_preview::ribbon::writer::design::{IDS, menu_items};
    let mut p = rig();
    for id in IDS {
        let Some(items) = menu_items(id) else {
            continue;
        };
        for it in items.iter().filter(|i| !i.separator && !i.heading) {
            let typed = format!("{}=x", it.id);
            let yes = [it.id.clone(), typed]
                .iter()
                .any(|i| p.shell.run_command(&mut p.ui, i) == Handled::Yes);
            assert!(yes, "menu item {} is not handled", it.id);
        }
    }
}
