//! Ribbon formatting controls drive the Writer pane through the command layer.

use open365_docx::Alignment;
use open365_preview::doc::{Toggle, WriterPane};
use open365_preview::{AppKind, Preview};
use open365_ui_core::{InputEvent, Key, Modifiers};

fn center(p: &Preview, id: &str) -> (f32, f32) {
    let (_, r) = p
        .shell
        .placed_controls()
        .into_iter()
        .find(|(k, _)| k == id)
        .unwrap_or_else(|| panic!("control {id} not placed"));
    (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0)
}

fn rig() -> Preview {
    let mut p = Preview::new(AppKind::Writer, true, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text("hello world");
    pane.select_all();
    p.shell.set_writer(&mut p.ui, pane);
    p
}

#[test]
fn bold_and_alignment_buttons_edit_the_document() {
    let mut p = rig();
    let (x, y) = center(&p, "home.font.bold");
    p.click(x, y);
    let w = p.shell.writer().expect("pane");
    assert!(
        w.toggle_state(Toggle::Bold),
        "bold applied to the selection"
    );
    assert!(w.can_undo());
    let (x, y) = center(&p, "home.paragraph.centre");
    p.click(x, y);
    assert_eq!(
        p.shell.writer().and_then(WriterPane::alignment_state),
        Some(Alignment::Center)
    );
    // Undo reverts the alignment through the command history.
    p.input(InputEvent::Key {
        key: Key::Char('z'),
        pressed: true,
        modifiers: Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    });
    assert_eq!(
        p.shell.writer().and_then(WriterPane::alignment_state),
        Some(Alignment::Left)
    );
}

#[test]
fn size_and_font_come_from_the_ribbon_ids() {
    let mut p = rig();
    p.shell
        .writer_mut()
        .expect("pane")
        .format(open365_preview::doc::Format::from_ribbon("home.font.font-size=18").expect("size"));
    assert_eq!(
        p.shell
            .writer()
            .and_then(WriterPane::font_state)
            .map(|f| f.1),
        Some(18.0)
    );
    p.shell
        .writer_mut()
        .expect("pane")
        .format(open365_preview::doc::Format::from_ribbon("home.font.font=Arial").expect("font"));
    assert_eq!(
        p.shell
            .writer()
            .and_then(WriterPane::font_state)
            .map(|f| f.0),
        Some("Arial".to_owned())
    );
}

#[test]
fn the_vertical_scroll_bar_scrolls_the_document() {
    let mut p = Preview::new(AppKind::Writer, true, 1600, 1000, 1.25);
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/docx/layout-table-pages.docx");
    let pane = WriterPane::open(&fixture).expect("fixture");
    p.shell.set_writer(&mut p.ui, pane);
    let w = p.shell.writer().expect("pane");
    assert!(w.page_count() > 1, "needs several pages");
    let (top, h) = w.thumb().expect("a thumb for a long document");
    assert!(top < 1.0 && h > 0.0);
    let canvas = w.canvas().expect("canvas");
    let r = p.ui.tree.abs_rect(canvas);
    let s = p.scale();
    // Press near the bottom of the track: the document scrolls a long way down.
    let (x, y) = ((r.x + r.w) * s - 4.0, (r.y + r.h) * s - 6.0);
    p.click(x, y);
    let w = p.shell.writer().expect("pane");
    assert!(w.scroll() > 100.0, "scrolled to {}", w.scroll());
    assert!(w.thumb().expect("thumb").0 > top);
}

fn key(p: &mut Preview, c: char, shift: bool) {
    p.input(InputEvent::Key {
        key: Key::Char(c),
        pressed: true,
        modifiers: Modifiers {
            ctrl: true,
            shift,
            ..Modifiers::default()
        },
    });
}

#[test]
fn ctrl_equals_toggles_subscript_and_ctrl_shift_equals_superscript() {
    let mut p = rig();
    let w = p.shell.writer_mut().expect("pane");
    w.key(
        Key::Char('='),
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    );
    assert!(w.toggle_state(Toggle::Sub) && !w.toggle_state(Toggle::Sup));
    // Superscript replaces subscript on the same text.
    w.key(
        Key::Char('+'),
        Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        },
    );
    assert!(w.toggle_state(Toggle::Sup) && !w.toggle_state(Toggle::Sub));
    // The same key again goes back to normal text.
    w.key(
        Key::Char('+'),
        Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        },
    );
    assert!(!w.toggle_state(Toggle::Sup) && !w.toggle_state(Toggle::Sub));
    assert!(w.can_undo());
}

#[test]
fn ruler_checkbox_and_find_button_drive_the_pane() {
    // Wide enough that the Editing group is not collapsed into one button.
    let mut p = Preview::new(AppKind::Writer, true, 2400, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text("hello world");
    p.shell.set_writer(&mut p.ui, pane);
    let view = p
        .shell
        .spec()
        .tabs
        .iter()
        .position(|t| t.id == "view")
        .expect("View tab");
    p.shell.select_tab(&mut p.ui, view);
    assert!(!p.shell.writer().unwrap().ruler());
    let (x, y) = center(&p, "view.show.ruler");
    p.click(x, y);
    assert!(
        p.shell.writer().unwrap().ruler(),
        "Ruler checkbox shows the ruler"
    );
    let (x, y) = center(&p, "view.show.ruler");
    p.click(x, y);
    assert!(!p.shell.writer().unwrap().ruler());
    let home = p
        .shell
        .spec()
        .tabs
        .iter()
        .position(|t| t.id == "home")
        .unwrap();
    p.shell.select_tab(&mut p.ui, home);
    assert!(!p.shell.writer().unwrap().find_is_open());
    let (x, y) = center(&p, "home.editing.find");
    p.click(x - 20.0, y);
    assert!(
        p.shell.writer().unwrap().find_is_open(),
        "Find opens the find box"
    );
    p.shell.writer_mut().unwrap().find_close();
    key(&mut p, 'f', false);
    assert!(
        p.shell.writer().unwrap().find_is_open(),
        "Ctrl+F opens Find"
    );
}
