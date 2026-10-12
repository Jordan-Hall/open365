//! Writer Insert tab controls run through `Shell::run_command` and change the document.

#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicUsize, Ordering};

use open365_docx::{CivilDateTime, DocPos, Document};
use open365_opc::Limits;
use open365_preview::doc::WriterPane;
use open365_preview::ribbon::Handled;
use open365_preview::{AppKind, Preview};

fn rig(text: &str) -> Preview {
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.paste_text(text);
    p.shell.set_writer(&mut p.ui, pane);
    p
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn pane(p: &mut Preview) -> &mut WriterPane {
    p.shell.writer_mut().unwrap()
}

static N: AtomicUsize = AtomicUsize::new(0);

fn saved_bytes(p: &mut Preview) -> Vec<u8> {
    let path = std::env::temp_dir().join(format!(
        "open365-insert-{}-{}.docx",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    assert!(pane(p).save_to(path.clone()), "save");
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    bytes
}

fn saved(p: &mut Preview) -> Document {
    Document::open(&saved_bytes(p), &Limits::default()).unwrap()
}

fn xml(p: &mut Preview) -> String {
    String::from_utf8(saved(p).serialize_main()).unwrap()
}

fn part(p: &mut Preview, name: &str) -> Option<String> {
    let doc = saved(p);
    doc.package()
        .part(name)
        .map(|x| String::from_utf8_lossy(x.data()).into_owned())
}

fn undo(p: &mut Preview) {
    assert!(pane(p).undo());
}

/// A 2x1 PNG on disk.
fn png() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "open365-pic-{}-{}.png",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let mut img = image::RgbaImage::new(96, 48);
    for px in img.pixels_mut() {
        *px = image::Rgba([200, 30, 30, 255]);
    }
    img.save(&path).unwrap();
    path
}

#[test]
fn page_break_splits_and_breaks() {
    let mut p = rig("before after");
    pane(&mut p).set_caret(DocPos::new(0, 7));
    run(&mut p, "insert.pages.page-break");
    assert_eq!(pane(&mut p).paragraph_count(), 2);
    assert_eq!(pane(&mut p).paragraph_text(1), Some("after"));
    assert!(xml(&mut p).contains("<w:br w:type=\"page\"/>"));
    assert_eq!(pane(&mut p).caret(), DocPos::new(1, 0));
    assert_eq!(
        pane(&mut p).page_count(),
        2,
        "the text after the break is on page 2"
    );
    undo(&mut p);
    assert_eq!(pane(&mut p).paragraph_count(), 1);
    assert!(!xml(&mut p).contains("w:type=\"page\""));
}

#[test]
fn blank_page_adds_an_empty_page() {
    let mut p = rig("before after");
    pane(&mut p).set_caret(DocPos::new(0, 7));
    run(&mut p, "insert.pages.blank-page");
    assert_eq!(xml(&mut p).matches("w:type=\"page\"").count(), 2);
    assert_eq!(pane(&mut p).page_count(), 3);
    undo(&mut p);
    assert_eq!(pane(&mut p).page_count(), 1);
}

#[test]
fn table_dimensions_and_default() {
    let mut p = rig("text");
    pane(&mut p).set_caret(DocPos::new(0, 0));
    run(&mut p, "insert.tables.table=3x4");
    let doc = saved(&mut p);
    assert_eq!(doc.table_count(), 1);
    let t = doc.table(0).unwrap();
    assert_eq!(t.rows().count(), 3);
    assert_eq!(t.rows().next().unwrap().cells().count(), 4);
    assert_eq!(
        pane(&mut p).caret(),
        DocPos::new(0, 0),
        "the caret is in the first cell"
    );
    undo(&mut p);
    assert_eq!(saved(&mut p).table_count(), 0);
    run(&mut p, "insert.tables.table");
    let doc = saved(&mut p);
    let t = doc.table(0).unwrap();
    assert_eq!(
        (t.rows().count(), t.rows().next().unwrap().cells().count()),
        (2, 5)
    );
}

#[test]
fn picture_is_embedded_and_scaled() {
    let mut p = rig("text");
    let path = png();
    pane(&mut p).set_caret(DocPos::new(0, 4));
    run(
        &mut p,
        &format!("insert.illustrations.pictures={}", path.display()),
    );
    let bytes = saved_bytes(&mut p);
    let doc = Document::open(&bytes, &Limits::default()).unwrap();
    let x = String::from_utf8(doc.serialize_main()).unwrap();
    assert!(x.contains("<wp:inline"), "{x}");
    // 96 px at 96 dpi is 72 pt = 914400 EMU.
    assert!(
        x.contains("cx=\"914400\"") && x.contains("cy=\"457200\""),
        "{x}"
    );
    assert!(doc.package().part("word/media/image1.png").is_some());
    assert!(
        doc.package()
            .part("[Content_Types].xml")
            .map(|c| String::from_utf8_lossy(c.data()).contains("Extension=\"png\""))
            .unwrap_or(false)
    );
    assert_eq!(
        pane(&mut p).paragraph_text(0),
        Some("text"),
        "text is unchanged"
    );
    undo(&mut p);
    assert!(!xml(&mut p).contains("<wp:inline"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn link_wraps_the_selection_and_makes_a_relationship() {
    let mut p = rig("see the site now");
    pane(&mut p).select(DocPos::new(0, 8), DocPos::new(0, 12));
    run(&mut p, "insert.links.link=https://example.org/a?b=1&c=2");
    let x = xml(&mut p);
    assert!(x.contains("<w:hyperlink r:id=\""), "{x}");
    assert!(x.contains("Hyperlink"), "{x}");
    assert_eq!(pane(&mut p).paragraph_text(0), Some("see the site now"));
    let rels = part(&mut p, "word/_rels/document.xml.rels").unwrap();
    assert!(
        rels.contains("https://example.org/a?b=1&amp;c=2")
            && rels.contains("TargetMode=\"External\""),
        "{rels}"
    );
    assert!(
        part(&mut p, "word/styles.xml")
            .unwrap()
            .contains("w:styleId=\"Hyperlink\"")
    );
    undo(&mut p);
    assert!(!xml(&mut p).contains("w:hyperlink"));
}

#[test]
fn link_without_selection_inserts_the_address() {
    let mut p = rig("go ");
    pane(&mut p).set_caret(DocPos::new(0, 3));
    run(&mut p, "insert.links.link=https://example.org");
    assert_eq!(
        pane(&mut p).paragraph_text(0),
        Some("go https://example.org")
    );
    assert!(xml(&mut p).contains("<w:hyperlink"));
}

#[test]
fn bookmark_marks_the_selection_and_moves() {
    let mut p = rig("alpha beta");
    pane(&mut p).select(DocPos::new(0, 0), DocPos::new(0, 5));
    run(&mut p, "insert.links.bookmark=Intro");
    let x = xml(&mut p);
    assert!(x.contains("w:name=\"Intro\""), "{x}");
    pane(&mut p).select(DocPos::new(0, 6), DocPos::new(0, 10));
    run(&mut p, "insert.links.bookmark=Intro");
    let x = xml(&mut p);
    assert_eq!(
        x.matches("<w:bookmarkStart").count(),
        1,
        "the bookmark moved: {x}"
    );
    assert_eq!(x.matches("<w:bookmarkEnd").count(), 1);
    // An invalid name changes nothing.
    run(&mut p, "insert.links.bookmark=1 bad name");
    assert_eq!(xml(&mut p).matches("<w:bookmarkStart").count(), 1);
    assert_eq!(pane(&mut p).paragraph_text(0), Some("alpha beta"));
}

#[test]
fn comment_anchors_on_the_selection() {
    let mut p = rig("alpha beta");
    pane(&mut p).select(DocPos::new(0, 0), DocPos::new(0, 5));
    run(&mut p, "insert.comments.comment=check this");
    let doc = saved(&mut p);
    let comments = doc.comments();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].text, "check this");
    undo(&mut p);
    assert!(saved(&mut p).comments().is_empty());
}

#[test]
fn header_and_footer_parts_are_created_and_replaced() {
    let mut p = rig("body");
    run(&mut p, "insert.header-footer.header=text:Quarterly report");
    let h = part(&mut p, "word/header1.xml").expect("header part");
    assert!(h.contains("Quarterly report"), "{h}");
    assert!(xml(&mut p).contains("<w:headerReference w:type=\"default\""));
    run(&mut p, "insert.header-footer.header=text:Annual report");
    let h = part(&mut p, "word/header1.xml").unwrap();
    assert!(
        h.contains("Annual report") && !h.contains("Quarterly"),
        "replaced: {h}"
    );
    run(&mut p, "insert.header-footer.footer");
    assert!(part(&mut p, "word/footer1.xml").is_some());
    run(&mut p, "insert.header-footer.header=remove");
    assert!(part(&mut p, "word/header1.xml").is_none());
    assert!(!xml(&mut p).contains("headerReference"));
    undo(&mut p);
    assert!(part(&mut p, "word/header1.xml").is_some());
    assert_eq!(pane(&mut p).paragraph_text(0), Some("body"));
}

#[test]
fn page_number_adds_a_page_field() {
    let mut p = rig("body");
    run(&mut p, "insert.header-footer.page-number");
    let f = part(&mut p, "word/footer1.xml").expect("footer part");
    assert!(
        f.contains(" PAGE ") && f.contains("w:jc w:val=\"center\""),
        "{f}"
    );
    run(&mut p, "insert.header-footer.page-number=top-right");
    let h = part(&mut p, "word/header1.xml").expect("header part");
    assert!(
        h.contains(" PAGE ") && h.contains("w:jc w:val=\"right\""),
        "{h}"
    );
}

#[test]
fn date_and_symbols_insert_text() {
    let mut p = rig("on ");
    pane(&mut p).set_caret(DocPos::new(0, 3));
    let when = CivilDateTime::new(2026, 3, 5, 9, 30, 0).unwrap();
    pane(&mut p).insert_date_time(when, "dd/MM/yyyy");
    assert_eq!(pane(&mut p).paragraph_text(0), Some("on 05/03/2026"));
    run(&mut p, "insert.text.date-time");
    assert!(pane(&mut p).paragraph_text(0).unwrap().len() > "on 05/03/2026".len());
    run(&mut p, "insert.symbols.symbol=U+00A9");
    run(&mut p, "insert.symbols.symbol=\u{2022}");
    let t = pane(&mut p).paragraph_text(0).unwrap().to_owned();
    assert!(t.ends_with("\u{a9}\u{2022}"), "{t}");
    undo(&mut p);
    assert!(pane(&mut p).paragraph_text(0).unwrap().ends_with('\u{a9}'));
}

#[test]
fn every_wired_insert_control_answers() {
    let mut p = rig("hello");
    for id in open365_preview::ribbon::writer::insert::IDS {
        if *id == "insert.illustrations.pictures" {
            continue; // the plain id opens the file dialog
        }
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
    }
}
