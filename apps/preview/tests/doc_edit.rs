//! The Writer pane on the own UI platform, driven only through `Ui` input events (the same path
//! the window host takes): caret, selection, typing, commands with undo, formatting shortcuts,
//! saving and reopening, the clipboard, IME composition and the AccessKit text subtree.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use accesskit::{Action, ActionData, ActionRequest, Role, TreeId};
use open365_docx::DocPos;
use open365_gfx::{Assets, Color, DrawList};
use open365_preview::doc::{PathChooser, WriterPane};
use open365_preview::layout::PxRect;
use open365_preview::{AppKind, Palette, fonts};
use open365_ui_core::geom::{Point, Rect};
use open365_ui_core::{Clipboard, ImeInput, InputEvent, Key, Modifiers, PointerButton, Ui};

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

#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Option<String>>>);

impl Clipboard for Shared {
    fn get_text(&mut self) -> Option<String> {
        self.0.borrow().clone()
    }

    fn set_text(&mut self, text: &str) {
        *self.0.borrow_mut() = Some(text.to_owned());
    }
}

fn rig_with(pane: WriterPane) -> Rig {
    let f = fonts::load();
    let mut assets = Assets::new(2048);
    let font = assets
        .add_font(Arc::clone(&f.primary), f.index)
        .expect("UI font");
    let bounds = Rect::new(0.0, 0.0, AREA.w as f32 / SCALE, AREA.h as f32 / SCALE);
    let mut ui = Ui::new(bounds, SCALE, assets, font, Instant::now());
    let mut pane = pane;
    let root = ui.tree.root();
    pane.build(&mut ui, root, AREA, SCALE, 1.0);
    Rig { ui, pane }
}

fn rig() -> Rig {
    rig_with(WriterPane::blank())
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("open365-doc-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

impl Rig {
    fn route(&mut self) {
        for e in self.ui.take_events() {
            self.pane.handle(&mut self.ui, &e);
        }
    }

    fn input(&mut self, ev: InputEvent) {
        self.ui.handle(ev);
        self.route();
    }

    fn typed(&mut self, s: &str) {
        self.input(InputEvent::Text(s.to_owned()));
    }

    fn key_with(&mut self, key: Key, m: Modifiers) {
        self.input(InputEvent::Key {
            key,
            pressed: true,
            modifiers: m,
        });
        self.input(InputEvent::Key {
            key,
            pressed: false,
            modifiers: m,
        });
    }

    fn key(&mut self, key: Key) {
        self.key_with(key, Modifiers::default());
    }

    fn ctrl(&mut self, c: char) {
        self.key_with(
            Key::Char(c),
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        );
    }

    fn shift(&mut self, key: Key) {
        self.key_with(
            key,
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        );
    }

    fn ctrl_key(&mut self, key: Key) {
        self.key_with(
            key,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        );
    }

    /// Logical pointer position over the canvas-local device point.
    fn at(x: f32, y: f32) -> Point {
        Point::new(x / SCALE, y / SCALE)
    }

    fn click_pos(&mut self, pos: DocPos) {
        let (x, y, h) = self.pane.point_of(pos).expect("position on a page");
        let p = Self::at(x, y + h / 2.0);
        self.input(InputEvent::PointerMoved(p));
        self.input(InputEvent::PointerDown(PointerButton::Primary, p));
        self.input(InputEvent::PointerUp(PointerButton::Primary, p));
    }

    fn text(&self) -> String {
        self.pane.text()
    }
}

fn long_text(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("Line {i} of the test document"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn pages_are_white_on_the_canvas_with_word_page_gap() {
    let mut r = rig();
    r.pane.paste_text(&long_text(120));
    assert!(r.pane.page_count() >= 2, "120 lines span several pages");
    assert!(
        r.pane.scroll() > 0.0,
        "the view followed the caret to the end"
    );
    r.pane.scroll_by(-1e9);
    let (x0, y0, w0, h0) = r.pane.page_rect(0).unwrap();
    let (x1, y1, w1, _) = r.pane.page_rect(1).unwrap();
    let s = SCALE * 96.0 / 72.0;
    // The blank document is A4 (595.3 pt wide).
    assert!((w0 / s - 595.3).abs() < 0.5, "page width {} pt", w0 / s);
    assert!((x0 - x1).abs() < 0.01 && (w0 - w1).abs() < 0.01);
    assert!(
        ((y1 - (y0 + h0)) - 10.8 * s).abs() < 0.5,
        "page gap is 10.8 pt"
    );
    assert!(
        (y0 - 12.6 * s).abs() < 0.5,
        "the first page starts 12.6 pt down"
    );

    let pal = Palette::new(AppKind::Writer, false);
    let mut list = DrawList::new(AREA.w as u32, AREA.h as u32);
    r.ui.assets.begin_frame();
    r.pane.paint(&mut r.ui, &mut list, &pal, Instant::now());
    assert!(!list.vertices().is_empty(), "pages and glyphs are drawn");
    // The page fill and the canvas colour both reach the draw list; nothing else is a border.
    let colours: Vec<[u8; 4]> = list.vertices().iter().map(|v| v.color).collect();
    let rgba = |c: Color| [c.r, c.g, c.b, c.a];
    assert!(colours.contains(&rgba(pal.page)));
    assert!(colours.contains(&rgba(pal.canvas)));
}

#[test]
fn click_places_the_caret_and_arrows_home_end_move_it() {
    let mut r = rig();
    r.typed("hello world");
    assert_eq!(r.pane.caret(), DocPos::new(0, 11));
    r.key(Key::Home);
    assert_eq!(r.pane.caret(), DocPos::new(0, 0));
    r.key(Key::Right);
    r.key(Key::Right);
    assert_eq!(r.pane.caret(), DocPos::new(0, 2));
    r.key(Key::Left);
    assert_eq!(r.pane.caret(), DocPos::new(0, 1));
    r.key(Key::End);
    assert_eq!(r.pane.caret(), DocPos::new(0, 11));
    r.click_pos(DocPos::new(0, 5));
    assert_eq!(r.pane.caret(), DocPos::new(0, 5));
    assert!(!r.pane.has_selection());
    // Up and down keep the column across paragraphs.
    r.pane.paste_text("\nsecond paragraph here");
    r.click_pos(DocPos::new(1, 4));
    r.key(Key::Up);
    assert_eq!(r.pane.caret().para, 0);
    r.key(Key::Down);
    assert_eq!(r.pane.caret().para, 1);
}

#[test]
fn ctrl_arrows_move_by_word_and_shift_selects() {
    let mut r = rig();
    r.typed("alpha beta gamma");
    r.key(Key::Home);
    r.ctrl_key(Key::Right);
    assert_eq!(r.pane.caret(), DocPos::new(0, 6));
    r.ctrl_key(Key::Right);
    assert_eq!(r.pane.caret(), DocPos::new(0, 11));
    r.ctrl_key(Key::Left);
    assert_eq!(r.pane.caret(), DocPos::new(0, 6));
    r.shift(Key::Right);
    r.shift(Key::Right);
    assert_eq!(r.pane.selection(), (DocPos::new(0, 6), DocPos::new(0, 8)));
    assert_eq!(r.pane.selected_text(), "be");
    r.shift(Key::End);
    assert_eq!(r.pane.selected_text(), "beta gamma");
    // An arrow without shift collapses to the selection edge.
    r.key(Key::Left);
    assert!(!r.pane.has_selection());
    assert_eq!(r.pane.caret(), DocPos::new(0, 6));
}

#[test]
fn page_up_and_page_down_move_a_viewport() {
    let mut r = rig();
    r.pane.paste_text(&long_text(150));
    r.ctrl_key(Key::Home);
    assert_eq!(r.pane.caret(), DocPos::new(0, 0));
    r.key(Key::PageDown);
    let down = r.pane.caret();
    assert!(down.para > 10, "page down moved {} paragraphs", down.para);
    assert!(r.pane.scroll() > 0.0, "the view follows the caret");
    r.key(Key::PageDown);
    assert!(r.pane.caret().para > down.para);
    r.key(Key::PageUp);
    r.key(Key::PageUp);
    assert_eq!(r.pane.caret().para, 0);
    r.ctrl_key(Key::End);
    assert_eq!(r.pane.caret().para, 149);
}

#[test]
fn mouse_drag_selects_a_range() {
    let mut r = rig();
    r.typed("one two three four");
    let (x0, y0, h) = r.pane.point_of(DocPos::new(0, 4)).unwrap();
    let (x1, _, _) = r.pane.point_of(DocPos::new(0, 13)).unwrap();
    let a = Rig::at(x0, y0 + h / 2.0);
    let b = Rig::at(x1, y0 + h / 2.0);
    r.input(InputEvent::PointerMoved(a));
    r.input(InputEvent::PointerDown(PointerButton::Primary, a));
    r.input(InputEvent::PointerMoved(b));
    r.pane.drag(&mut r.ui);
    r.input(InputEvent::PointerUp(PointerButton::Primary, b));
    assert_eq!(r.pane.selected_text(), "two three");
    assert_eq!(r.pane.anchor(), DocPos::new(0, 4));
    // Releasing ends the drag: further moves do not change the selection.
    r.input(InputEvent::PointerMoved(Rig::at(x0, y0 + h / 2.0)));
    r.pane.drag(&mut r.ui);
    assert_eq!(r.pane.selected_text(), "two three");
    // A double click selects the word.
    r.click_pos(DocPos::new(0, 1));
    r.click_pos(DocPos::new(0, 1));
    assert_eq!(r.pane.selected_text(), "one");
}

#[test]
fn typing_backspace_delete_enter_and_tab_go_through_commands() {
    let mut r = rig();
    r.typed("Hello");
    r.typed(", world");
    assert_eq!(r.text(), "Hello, world");
    r.key(Key::Backspace);
    assert_eq!(r.text(), "Hello, worl");
    r.key(Key::Home);
    r.key(Key::Delete);
    assert_eq!(r.text(), "ello, worl");
    r.key(Key::Enter);
    assert_eq!(r.pane.paragraph_count(), 2);
    assert_eq!(r.pane.caret(), DocPos::new(1, 0));
    r.typed("x");
    r.key(Key::Tab);
    // A tab is one character: the em space in the paragraph model.
    assert_eq!(r.pane.paragraph_text(1), Some("x\u{2003}ello, worl"));
    // Backspace at a paragraph start joins it with the previous (empty) one.
    r.key(Key::Home);
    r.key(Key::Backspace);
    assert_eq!(r.pane.paragraph_count(), 1);
    assert_eq!(r.text(), "x\u{2003}ello, worl");
    // Typing over a selection replaces it.
    r.ctrl('A');
    r.typed("new");
    assert_eq!(r.text(), "new");
    assert!(r.pane.is_dirty());
}

#[test]
fn undo_and_redo_restore_the_text() {
    let mut r = rig();
    r.typed("abc");
    r.key(Key::Enter);
    r.typed("def");
    assert_eq!(r.text(), "abc\ndef");
    r.ctrl('Z');
    assert_eq!(r.text(), "abc\n");
    r.ctrl('Z');
    assert_eq!(r.text(), "abc");
    r.ctrl('Z');
    assert_eq!(r.text(), "");
    assert!(!r.pane.undo(), "nothing left to undo");
    r.ctrl('Y');
    r.ctrl('Y');
    r.ctrl('Y');
    assert_eq!(r.text(), "abc\ndef");
    assert!(r.pane.can_undo());
}

#[test]
fn ctrl_b_i_u_format_the_selection_and_undo_removes_it() {
    let mut r = rig();
    r.typed("format me");
    r.ctrl('A');
    for (c, which) in [
        ('B', open365_preview::doc::Toggle::Bold),
        ('I', open365_preview::doc::Toggle::Italic),
        ('U', open365_preview::doc::Toggle::Underline),
    ] {
        assert!(!r.pane.toggle_state(which));
        r.ctrl(c);
        assert!(r.pane.toggle_state(which), "ctrl+{c} turns it on");
        r.ctrl(c);
        assert!(!r.pane.toggle_state(which), "ctrl+{c} again turns it off");
        r.ctrl(c);
    }
    r.ctrl('Z');
    assert!(!r.pane.toggle_state(open365_preview::doc::Toggle::Underline));
    // With no selection the toggle applies to the text typed next.
    r.key(Key::End);
    r.ctrl('B');
    r.typed("!");
    assert_eq!(r.text(), "format me!");
    r.key(Key::Left);
    r.key(Key::Left);
    r.ctrl('A');
    r.ctrl('B');
    let saved = tmp("format.docx");
    assert!(r.pane.save_to(saved.clone()));
    let again = WriterPane::open(&saved).unwrap();
    assert_eq!(again.text(), "format me!");
}

#[test]
fn save_reopen_and_compare_the_text() {
    let mut r = rig();
    r.typed("First paragraph");
    r.key(Key::Enter);
    r.typed("Second paragraph with a tab:");
    r.key(Key::Tab);
    r.typed("done");
    let expected = r.text();
    let path = tmp("saved.docx");
    assert!(r.pane.save_to(path.clone()));
    assert!(!r.pane.is_dirty());
    assert_eq!(r.pane.path(), Some(path.as_path()));

    let reopened = WriterPane::open(&path).expect("reopens");
    assert_eq!(reopened.text(), expected);
    assert_eq!(reopened.paragraph_count(), 2);

    // Ctrl+S saves an edit of the reopened document in place.
    let mut r2 = rig_with(reopened);
    r2.ctrl_key(Key::End);
    r2.typed(" and more");
    r2.ctrl('S');
    assert!(!r2.pane.is_dirty());
    let third = WriterPane::open(&path).unwrap();
    assert!(third.text().ends_with("done and more"), "{}", third.text());
    assert_eq!(third.paragraph_text(0), Some("First paragraph"));
}

static SAVE_AS: OnceLock<PathBuf> = OnceLock::new();

fn pick() -> Option<PathBuf> {
    SAVE_AS.get().cloned()
}

#[test]
fn ctrl_s_on_an_untitled_document_asks_where_to_save() {
    let target = tmp("untitled-save-as");
    SAVE_AS.set(target.clone()).unwrap();
    let mut r = rig();
    r.pane.set_path_chooser(pick as PathChooser);
    r.typed("untitled text");
    assert!(r.pane.path().is_none());
    r.ctrl('S');
    let saved = target.with_extension("docx");
    assert_eq!(r.pane.path(), Some(saved.as_path()));
    assert_eq!(WriterPane::open(&saved).unwrap().text(), "untitled text");
}

#[test]
fn clipboard_copy_cut_and_paste_plain_text() {
    let clip = Shared::default();
    let mut r = rig();
    r.pane.set_clipboard(Box::new(clip.clone()));
    r.typed("one two three");
    r.key(Key::Home);
    r.ctrl_key(Key::Right);
    r.shift(Key::End);
    r.ctrl('C');
    assert_eq!(clip.0.borrow().as_deref(), Some("two three"));
    assert_eq!(r.text(), "one two three", "copy leaves the text");
    r.ctrl('X');
    assert_eq!(r.text(), "one ");
    assert_eq!(clip.0.borrow().as_deref(), Some("two three"));
    r.key(Key::Home);
    r.ctrl('V');
    assert_eq!(r.text(), "two threeone ");
    // Multi-line text pastes as paragraphs; one undo takes it back.
    *clip.0.borrow_mut() = Some("a\r\nb\nc".to_owned());
    r.ctrl_key(Key::End);
    r.ctrl('V');
    assert_eq!(r.pane.paragraph_count(), 3);
    assert_eq!(r.text(), "two threeone a\nb\nc");
    r.ctrl('Z');
    assert_eq!(r.text(), "two threeone ");
}

#[test]
fn ime_composition_shows_at_the_caret_and_commits_as_text() {
    let mut r = rig();
    r.typed("ab");
    assert!(r.ui.wants_ime(), "the canvas asks for the input method");
    let area = r.ui.ime_cursor_area().expect("candidate window position");
    assert!(area.y1 > area.y0);
    let before = area.x0;
    r.input(InputEvent::Ime(ImeInput::Enabled));
    r.input(InputEvent::Ime(ImeInput::Preedit {
        text: "ni".to_owned(),
        cursor: Some((2, 2)),
    }));
    assert_eq!(r.pane.preedit(), Some("ni"));
    assert_eq!(r.text(), "ab", "the preedit is not in the document yet");
    // The candidate window follows the preedit.
    let moved = r.ui.ime_cursor_area().unwrap().x0;
    assert!(moved > before, "{moved} > {before}");
    r.input(InputEvent::Ime(ImeInput::Commit(
        "\u{4f60}\u{597d}".to_owned(),
    )));
    assert_eq!(r.pane.preedit(), None);
    assert_eq!(r.text(), "ab\u{4f60}\u{597d}");
    // The composition draws without panicking.
    r.input(InputEvent::Ime(ImeInput::Preedit {
        text: "x".to_owned(),
        cursor: None,
    }));
    let pal = Palette::new(AppKind::Writer, true);
    let mut list = DrawList::new(AREA.w as u32, AREA.h as u32);
    r.ui.assets.begin_frame();
    r.pane.paint(&mut r.ui, &mut list, &pal, Instant::now());
    assert!(!list.is_empty());
}

#[test]
fn accessibility_exposes_a_text_input_with_the_selection() {
    let mut r = rig();
    r.typed("hello there");
    r.key(Key::Enter);
    r.typed("second line");
    r.click_pos(DocPos::new(0, 1));
    r.shift(Key::Right);
    r.shift(Key::Right);
    let acc = r.pane.access().expect("access tree");
    assert_eq!(acc.node.role(), Role::MultilineTextInput);
    let sel = acc.node.text_selection().expect("selection exposed");
    assert_eq!(sel.anchor.character_index, 1);
    assert_eq!(sel.focus.character_index, 3);
    assert_eq!(sel.anchor.node, sel.focus.node);
    let run = acc
        .nodes
        .iter()
        .find(|(id, _)| *id == sel.anchor.node)
        .map(|(_, n)| n)
        .expect("the selection points at a published run");
    assert_eq!(run.role(), Role::TextRun);
    assert_eq!(run.value(), Some("hello there\n"));
    assert_eq!(run.character_lengths().len(), 12);
    let paragraphs = acc
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::Paragraph)
        .count();
    assert_eq!(paragraphs, 2);

    // A screen reader moves the selection and replaces text through actions.
    let focus = acc.node.text_selection().unwrap().focus.node;
    let request = ActionRequest {
        action: Action::SetTextSelection,
        target_tree: TreeId::ROOT,
        target_node: open365_ui_core::access::ak_id(r.pane.canvas().unwrap()),
        data: Some(ActionData::SetTextSelection(accesskit::TextSelection {
            anchor: accesskit::TextPosition {
                node: focus,
                character_index: 6,
            },
            focus: accesskit::TextPosition {
                node: focus,
                character_index: 11,
            },
        })),
    };
    assert!(r.pane.accessibility_action(&request));
    assert_eq!(r.pane.selected_text(), "there");
    let replace = ActionRequest {
        action: Action::ReplaceSelectedText,
        target_tree: request.target_tree,
        target_node: request.target_node,
        data: Some(ActionData::Value("friends".into())),
    };
    assert!(r.pane.accessibility_action(&replace));
    assert_eq!(r.pane.paragraph_text(0), Some("hello friends"));
}

#[test]
fn caret_blinks_and_hides_while_a_range_is_selected() {
    let mut r = rig();
    r.typed("blink");
    let t0 = Instant::now();
    let pal = Palette::new(AppKind::Writer, false);
    let count = |r: &mut Rig, now: Instant| {
        let mut list = DrawList::new(AREA.w as u32, AREA.h as u32);
        r.ui.assets.begin_frame();
        r.pane.paint(&mut r.ui, &mut list, &pal, now);
        list.vertices().len()
    };
    let on = count(&mut r, t0);
    let off = count(&mut r, t0 + std::time::Duration::from_millis(600));
    assert!(
        on > off,
        "the caret quad is missing in the hidden half ({on} vs {off})"
    );
    assert!(r.pane.caret_on(t0, r.ui.settings.caret_blink));
}

#[test]
fn a_real_docx_opens_lays_out_and_paints() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/docx/layout-hf-basic.docx");
    let pane = WriterPane::open(&path).expect("fixture opens");
    assert!(pane.paragraph_count() > 1);
    assert!(pane.page_count() >= 1);
    assert!(pane.title().starts_with("layout-hf-basic.docx"));
    let first_text = pane.text();
    assert!(!first_text.trim().is_empty());
    let mut r = rig_with(pane);
    let pal = Palette::new(AppKind::Writer, false);
    let mut list = DrawList::new(AREA.w as u32, AREA.h as u32);
    r.ui.assets.begin_frame();
    r.pane.paint(&mut r.ui, &mut list, &pal, Instant::now());
    assert!(list.vertices().len() > 100, "glyphs are drawn");
    // Clicking in the first line places the caret in paragraph 0 or the first text paragraph.
    r.click_pos(DocPos::new(0, 0));
    assert_eq!(r.pane.caret().offset, 0);
}

#[test]
fn find_box_shows_a_blinking_caret_that_restarts_on_typing() {
    let mut r = rig();
    r.typed("abc");
    let click = r.pane.canvas();
    r.ui.set_focus(click);
    r.pane.find_open();
    let pal = Palette::new(AppKind::Writer, false);
    let blink = r.ui.settings.caret_blink.expect("default blink");
    let count = |r: &mut Rig, now: Instant| {
        let mut list = DrawList::new(AREA.w as u32, AREA.h as u32);
        r.ui.assets.begin_frame();
        r.pane.paint(&mut r.ui, &mut list, &pal, now);
        list.vertices().len()
    };
    let t0 = Instant::now();
    r.input(InputEvent::Tick(t0));
    r.input(InputEvent::Text("b".to_owned()));
    let typed_at = Instant::now();
    let on = count(&mut r, typed_at);
    let off = count(
        &mut r,
        typed_at + blink + std::time::Duration::from_millis(5),
    );
    assert!(
        on > off,
        "the find box caret is missing in the hidden half ({on} vs {off})"
    );
    assert!(
        r.ui.next_wake().is_some(),
        "a wake-up is scheduled for the next blink edge"
    );
}
