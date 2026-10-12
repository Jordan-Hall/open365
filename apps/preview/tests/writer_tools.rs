//! Writer pane tools, headless: the vertical ruler, Find and Replace (Ctrl+H) and the
//! accessibility nodes of pictures. Painting is observed through `PaintStats`.

use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use open365_commands::Target;
use open365_docx::DocPos;
use open365_gfx::{Assets, DrawList};
use open365_preview::doc::{PaintStats, WriterPane};
use open365_preview::layout::PxRect;
use open365_preview::{AppKind, Palette, fonts};
use open365_ui_core::geom::Rect;
use open365_ui_core::{InputEvent, Key, Modifiers, Ui};

const SCALE: f32 = 1.25;
const AREA: PxRect = PxRect {
    x: 0,
    y: 0,
    w: 1250,
    h: 800,
};

struct Rig {
    ui: Ui,
    pane: WriterPane,
}

fn rig_with(mut pane: WriterPane) -> Rig {
    let f = fonts::load();
    let mut assets = Assets::new(2048);
    let font = assets
        .add_font(Arc::clone(&f.primary), f.index)
        .expect("UI font");
    let bounds = Rect::new(0.0, 0.0, AREA.w as f32 / SCALE, AREA.h as f32 / SCALE);
    let mut ui = Ui::new(bounds, SCALE, assets, font, Instant::now());
    let root = ui.tree.root();
    pane.build(&mut ui, root, AREA, SCALE, 1.0);
    Rig { ui, pane }
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/docx/{name}.docx"));
    std::fs::read(path).expect("fixture reads")
}

fn pane_of(bytes: &[u8]) -> WriterPane {
    WriterPane::new(Target::open_docx(bytes).expect("document opens"))
}

impl Rig {
    fn paint(&mut self) -> PaintStats {
        self.pane.reset_stats();
        let pal = Palette::new(AppKind::Writer, false);
        let mut list = DrawList::new(AREA.w as u32, AREA.h as u32);
        self.pane
            .paint(&mut self.ui, &mut list, &pal, Instant::now());
        self.pane.stats()
    }

    fn input(&mut self, ev: InputEvent) {
        self.ui.handle(ev);
        for e in self.ui.take_events() {
            self.pane.handle(&mut self.ui, &e);
        }
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.input(InputEvent::Text(c.to_string()));
        }
    }
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Modifiers::default()
    }
}

fn plain() -> Modifiers {
    Modifiers::default()
}

// ---- vertical ruler -------------------------------------------------------------------------

#[test]
fn vertical_ruler_follows_the_view_ruler_toggle_and_shows_the_margins() {
    let mut r = rig_with(WriterPane::blank());
    r.pane.insert_text("Text");
    let off = r.paint();
    assert_eq!(off.vruler_items, 0);
    let x_off = r.pane.page_rect(0).unwrap().0;

    r.pane.set_ruler(true);
    let on = r.paint();
    assert_eq!(on.vruler_items, 2, "top and bottom margin edges: {on:?}");
    let x_on = r.pane.page_rect(0).unwrap().0;
    assert!(
        x_on > x_off + 10.0,
        "the band moves the page right ({x_off} -> {x_on})"
    );
    let v = r.pane.vruler_info().expect("vertical ruler");
    assert!((v.margin_top_pt - 72.0).abs() < 1.0, "{v:?}");
    assert!((v.margin_bottom_pt - 72.0).abs() < 1.0, "{v:?}");
    assert!(v.rows.is_empty());

    r.pane.set_ruler(false);
    assert_eq!(r.paint().vruler_items, 0);
    assert!((r.pane.page_rect(0).unwrap().0 - x_off).abs() < 0.5);
}

#[test]
fn vertical_ruler_follows_the_caret_page() {
    let mut r = rig_with(WriterPane::blank());
    r.pane.paste_text(&"line\n".repeat(150));
    assert!(r.pane.page_count() > 1);
    r.pane.set_ruler(true);
    r.pane.set_caret(DocPos::new(0, 0));
    assert_eq!(r.pane.vruler_info().unwrap().page, 0);
    r.pane.set_caret(DocPos::new(149, 0));
    assert_eq!(r.pane.vruler_info().unwrap().page, r.pane.page_count() - 1);
}

#[test]
fn vertical_ruler_marks_the_rows_of_the_caret_table() {
    let mut r = rig_with(pane_of(&fixture_bytes("table-merged-cells")));
    r.pane.set_ruler(true);
    let mut best = None;
    for p in 0..r.pane.paragraph_count() {
        r.pane.set_caret(DocPos::new(p, 0));
        if let Some(v) = r.pane.vruler_info().filter(|v| !v.rows.is_empty()) {
            best = Some(v);
            break;
        }
    }
    let v = best.expect("a paragraph inside the table");
    assert!(v.rows.len() >= 2, "{v:?}");
    assert!(
        v.rows.windows(2).all(|w| w[0].0 < w[1].0),
        "rows run down the page: {v:?}"
    );
    let s = r.paint();
    assert_eq!(s.vruler_items, 2 + v.rows.len() + 1, "{s:?}");
}

// ---- Find and Replace -----------------------------------------------------------------------

fn replace_rig(text: &str) -> Rig {
    let mut r = rig_with(WriterPane::blank());
    r.pane.insert_text(text);
    r.pane.set_caret(DocPos::new(0, 0));
    r.pane.key(Key::Char('h'), ctrl());
    assert!(r.pane.replace_is_open());
    r
}

fn set_fields(r: &mut Rig, find: &str, with: &str) {
    r.type_text(find);
    r.pane.key(Key::Tab, plain());
    r.type_text(with);
    r.pane.key(Key::Tab, plain());
}

#[test]
fn replace_all_is_one_undo_step_and_reports_the_count() {
    let mut r = replace_rig("foo Foo foobar foo");
    set_fields(&mut r, "foo", "x");
    assert_eq!(r.pane.find_query(), "foo");
    assert_eq!(r.pane.replacement(), "x");
    assert_eq!(r.pane.replace_all(), 4);
    assert_eq!(r.pane.text(), "x X xbar x");
    assert_eq!(
        r.pane.replace_message(),
        Some("All done. We made 4 replacements.")
    );
    assert!(r.pane.undo());
    assert_eq!(
        r.pane.text(),
        "foo Foo foobar foo",
        "one undo step restores all four"
    );
}

#[test]
fn replace_all_honours_match_case_and_whole_words() {
    let mut r = replace_rig("foo Foo foobar foo");
    set_fields(&mut r, "foo", "x");
    r.pane.set_find_match_case(true);
    assert_eq!(r.pane.replace_all(), 3);
    r.pane.undo();
    r.pane.set_find_whole_word(true);
    assert_eq!(r.pane.replace_all(), 2);
    assert_eq!(r.pane.text(), "x Foo foobar x");
    assert_eq!(
        r.pane.replace_message(),
        Some("All done. We made 2 replacements.")
    );
}

#[test]
fn replace_all_with_wildcards_and_special_codes() {
    let mut r = replace_rig("cat cot cut cart");
    set_fields(&mut r, "c?t", "[^&]");
    r.pane.set_find_wildcards(true);
    assert_eq!(r.pane.replace_all(), 3);
    assert_eq!(r.pane.text(), "[cat] [cot] [cut] cart");

    // An invalid expression changes nothing and says so.
    let mut r = replace_rig("a b");
    r.type_text("[");
    r.pane.set_find_wildcards(true);
    assert_eq!(r.pane.replace_all(), 0);
    assert_eq!(r.pane.text(), "a b");
    assert!(r.pane.replace_message().unwrap().contains("Pattern Match"));
}

#[test]
fn replace_all_without_matches_says_so() {
    let mut r = replace_rig("alpha");
    set_fields(&mut r, "zzz", "y");
    assert_eq!(r.pane.replace_all(), 0);
    assert!(
        r.pane
            .replace_message()
            .unwrap()
            .contains("found no matches")
    );
    // No undo step was recorded: the one undo takes back the typed text.
    r.pane.undo();
    assert_eq!(r.pane.text(), "");
}

#[test]
fn replace_finds_first_then_replaces_and_moves_on() {
    let mut r = replace_rig("foo and foo");
    set_fields(&mut r, "foo", "bar");
    // The selection is not a match yet, so the first Replace only finds.
    assert!(!r.pane.replace_one());
    assert_eq!(r.pane.selected_text(), "foo");
    assert_eq!(r.pane.text(), "foo and foo");
    assert!(r.pane.replace_one());
    assert_eq!(r.pane.text(), "bar and foo");
    assert_eq!(r.pane.selected_text(), "foo", "the next match is selected");
    assert_eq!(r.pane.selection().0, DocPos::new(0, 8));
    assert!(r.pane.replace_one());
    assert_eq!(r.pane.text(), "bar and bar");
    // Each Replace is its own undo step.
    r.pane.undo();
    assert_eq!(r.pane.text(), "bar and foo");
    r.pane.undo();
    assert_eq!(r.pane.text(), "foo and foo");
}

#[test]
fn enter_finds_next_and_escape_closes_the_dialog() {
    let mut r = replace_rig("a foo b foo");
    r.type_text("foo");
    r.pane.key(Key::Enter, plain());
    assert_eq!(r.pane.selection().0, DocPos::new(0, 2));
    r.pane.key(Key::Enter, plain());
    assert_eq!(r.pane.selection().0, DocPos::new(0, 8));
    let s = r.paint();
    assert_eq!(s.find_marks, 2);
    r.pane.key(Key::Escape, plain());
    assert!(!r.pane.replace_is_open());
    assert!(!r.pane.find_is_open());
}

// ---- picture accessibility ------------------------------------------------------------------

/// The fixture with `descr` on its picture's `wp:docPr`.
fn with_alt_text(alt: &str) -> Vec<u8> {
    let src = fixture_bytes("inline-image");
    let mut zin = zip::ZipArchive::new(Cursor::new(src)).expect("zip");
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).expect("zip");
        let name = f.name().expect("zip").into_owned();
        let mut data = Vec::new();
        f.read_to_end(&mut data).expect("zip");
        if name == "word/document.xml" {
            let xml = String::from_utf8(data).expect("zip");
            assert!(xml.contains("name=\"Picture 1\""));
            data = xml
                .replace(
                    "name=\"Picture 1\"",
                    &format!("name=\"Picture 1\" descr=\"{alt}\""),
                )
                .into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default())
            .expect("zip");
        out.write_all(&data).expect("zip");
    }
    out.finish().expect("zip").into_inner()
}

#[test]
fn pictures_expose_their_alt_text_to_screen_readers() {
    let mut r = rig_with(pane_of(&with_alt_text("A red square")));
    r.paint();
    let a = r.pane.access().expect("access");
    let pic = a
        .nodes
        .iter()
        .find(|(_, n)| n.role() == accesskit::Role::Image)
        .expect("an Image node for the picture");
    assert_eq!(pic.1.label(), Some("A red square"));
    assert_eq!(pic.1.description(), Some("A red square"));
    assert!(pic.1.bounds().is_some_and(|b| b.x1 > b.x0 && b.y1 > b.y0));
    assert!(
        a.node.children().contains(&pic.0),
        "the document node owns it"
    );
}

#[test]
fn a_picture_without_alt_text_is_still_announced() {
    let mut r = rig_with(pane_of(&fixture_bytes("inline-image")));
    r.paint();
    let a = r.pane.access().expect("access");
    let pic = a
        .nodes
        .iter()
        .find(|(_, n)| n.role() == accesskit::Role::Image)
        .expect("Image node");
    assert_eq!(pic.1.label(), Some("Picture"));
    assert_eq!(pic.1.description(), None);
}

// ---- wrap question and Replace codes ----------------------------------------------------------

#[test]
fn find_next_at_the_end_asks_before_continuing_at_the_beginning() {
    let mut r = replace_rig("foo a foo b foo");
    r.type_text("foo");
    // Start just after the second match's first letter.
    r.pane.set_caret(DocPos::new(0, 7));
    assert!(r.pane.replace_find_next());
    assert_eq!(r.pane.selection().0, DocPos::new(0, 12));
    // No more matches ahead: the question, and the selection stays.
    assert!(!r.pane.replace_find_next());
    assert!(r.pane.replace_wrap_prompt());
    assert_eq!(r.pane.selection().0, DocPos::new(0, 12));
    r.paint();
    // No answers the question and changes nothing.
    r.pane.key(Key::Escape, plain());
    assert!(!r.pane.replace_wrap_prompt());
    assert!(r.pane.replace_is_open());
    assert_eq!(r.pane.selection().0, DocPos::new(0, 12));
    // Yes continues at the beginning, and only up to where the search began.
    assert!(!r.pane.replace_find_next());
    assert!(r.pane.replace_wrap_prompt());
    assert!(r.pane.replace_wrap_answer(true));
    assert_eq!(r.pane.selection().0, DocPos::new(0, 0));
    assert!(r.pane.replace_find_next());
    assert_eq!(r.pane.selection().0, DocPos::new(0, 6));
    // Back at where it began (the match at 12 was shown already): nothing is left.
    assert!(!r.pane.replace_find_next());
    assert!(!r.pane.replace_wrap_prompt());
    assert!(
        r.pane
            .replace_message()
            .unwrap()
            .contains("finished searching")
    );
}

#[test]
fn find_next_from_the_start_does_not_ask() {
    let mut r = replace_rig("foo and foo");
    r.type_text("foo");
    assert!(r.pane.replace_find_next());
    assert!(r.pane.replace_find_next());
    assert!(!r.pane.replace_find_next());
    assert!(!r.pane.replace_wrap_prompt() || r.pane.selection().0 == DocPos::new(0, 8));
}

#[test]
fn replace_expands_the_codes_replace_all_knows() {
    let mut r = replace_rig("foo");
    set_fields(&mut r, "foo", "[^&^t^^]");
    assert!(!r.pane.replace_one());
    assert!(r.pane.replace_one());
    // The pane's text shows a tab as an em space.
    assert_eq!(r.pane.text(), "[foo\u{2003}^]");

    // A line break comes out as a break inside the paragraph, as in Replace All.
    let mut r = replace_rig("foo");
    set_fields(&mut r, "foo", "a^lb");
    r.pane.replace_one();
    assert!(r.pane.replace_one());
    assert_eq!(r.pane.paragraph_count(), 1);
    assert_eq!(r.pane.text(), "a\nb");

    // A code the command layer does not know is refused, as in Replace All.
    let mut r = replace_rig("foo");
    set_fields(&mut r, "foo", "a^zb");
    r.pane.replace_one();
    assert!(!r.pane.replace_one());
    assert_eq!(r.pane.text(), "foo");
    assert!(r.pane.replace_message().is_some());
}

#[test]
fn text_boxes_expose_their_text_to_screen_readers() {
    let mut r = rig_with(pane_of(&fixture_bytes("layout-txbx-basic")));
    r.paint();
    let a = r.pane.access().expect("access");
    let boxed: Vec<_> = a
        .nodes
        .iter()
        .filter(|(_, n)| n.label() == Some("Text Box"))
        .collect();
    assert!(!boxed.is_empty(), "the fixture has text boxes");
    let mut with_text = 0;
    for (_, node) in boxed {
        for kid in node.children() {
            let run = a
                .nodes
                .iter()
                .find(|(id, _)| id == kid)
                .expect("child node exists");
            assert_eq!(run.1.role(), accesskit::Role::TextRun);
            if run.1.value().is_some_and(|v| v.trim().len() > 1) {
                with_text += 1;
            }
        }
    }
    assert!(with_text > 0, "text children carry the box's words");
}
