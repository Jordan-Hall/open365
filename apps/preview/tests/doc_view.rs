//! Writer pane features on the own UI platform, headless: table shading and borders, pictures,
//! raised note references, zoom, the ruler, paragraph styles and find. Painting is observed
//! through `PaintStats` (what was drawn) and the draw list, not pixels.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use open365_commands::Command;
use open365_docx::DocPos;
use open365_gfx::{Assets, DrawList};
use open365_preview::doc::ruler::{RulerEdit, RulerTab};
use open365_preview::doc::{Format, PaintStats, WriterPane};
use open365_preview::layout::PxRect;
use open365_preview::{AppKind, Palette, fonts};
use open365_ui_core::geom::{Point, Rect};
use open365_ui_core::{InputEvent, Key, Modifiers, PointerButton, Ui};

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

fn fixture(name: &str) -> WriterPane {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/docx/{name}.docx"));
    WriterPane::open(&path).expect("fixture opens")
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

impl Rig {
    /// Paints one frame and returns what it drew.
    fn paint(&mut self) -> PaintStats {
        self.pane.reset_stats();
        let pal = Palette::new(AppKind::Writer, false);
        let mut list = DrawList::new(AREA.w as u32, AREA.h as u32);
        self.pane
            .paint(&mut self.ui, &mut list, &pal, Instant::now());
        self.pane.stats()
    }

    fn route(&mut self) {
        for e in self.ui.take_events() {
            self.pane.handle(&mut self.ui, &e);
        }
    }

    fn input(&mut self, ev: InputEvent) {
        self.ui.handle(ev);
        self.route();
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

    fn at(x: f32, y: f32) -> Point {
        Point::new(x / SCALE, y / SCALE)
    }

    fn click(&mut self, x: f32, y: f32) {
        let p = Self::at(x, y);
        self.input(InputEvent::PointerMoved(p));
        self.input(InputEvent::PointerDown(PointerButton::Primary, p));
        self.input(InputEvent::PointerUp(PointerButton::Primary, p));
    }
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Modifiers::default()
    }
}

#[test]
fn table_cells_draw_borders_and_shading() {
    let mut r = rig_with(fixture("table-merged-cells"));
    let s = r.paint();
    assert!(s.border_strips >= 40, "cell borders are drawn: {s:?}");

    let mut r = rig_with(fixture("rev-table-props"));
    let s = r.paint();
    assert!(s.cell_fills > 0, "shaded cells are filled: {s:?}");

    // A paragraph border is a rule.
    let mut r = rig_with(fixture("layout-frame-bdr"));
    assert!(r.paint().rules > 0);

    // No tables, no cell drawing.
    let mut r = rig_with(WriterPane::blank());
    let s = r.paint();
    assert_eq!((s.cell_fills, s.border_strips, s.rules), (0, 0, 0));
}

#[test]
fn shaded_cells_add_geometry_to_the_draw_list() {
    let pal = Palette::new(AppKind::Writer, false);
    let mut r = rig_with(fixture("rev-table-props"));
    let mut with = DrawList::new(AREA.w as u32, AREA.h as u32);
    r.pane.paint(&mut r.ui, &mut with, &pal, Instant::now());
    let mut blank = rig_with(WriterPane::blank());
    let mut without = DrawList::new(AREA.w as u32, AREA.h as u32);
    blank
        .pane
        .paint(&mut blank.ui, &mut without, &pal, Instant::now());
    assert!(with.vertices().len() > without.vertices().len());
}

#[test]
fn inline_and_floating_pictures_are_decoded_once_and_drawn() {
    let mut r = rig_with(fixture("inline-image"));
    let s = r.paint();
    assert_eq!(s.pictures, 1, "{s:?}");
    assert_eq!(s.floating_pictures, 0);
    assert_eq!(r.pane.cached_pictures(), 1);
    r.paint();
    assert_eq!(
        r.pane.cached_pictures(),
        1,
        "the second frame reuses the decode"
    );

    let mut r = rig_with(fixture("layout-float-wrap-behind-front"));
    let s = r.paint();
    assert_eq!(s.pictures, 3, "{s:?}");
    assert_eq!(s.floating_pictures, 3);
}

#[test]
fn note_references_are_raised() {
    let mut r = rig_with(fixture("footnotes-endnotes"));
    let s = r.paint();
    assert!(s.raised_runs >= 2, "{s:?}");
    let mut plain = rig_with(WriterPane::blank());
    assert_eq!(plain.paint().raised_runs, 0);
}

#[test]
fn zoom_scales_the_pages_and_keeps_the_view() {
    let mut r = rig_with(WriterPane::blank());
    let (_, _, w100, h100) = r.pane.page_rect(0).unwrap();
    assert_eq!(r.pane.zoom_percent(), 100.0);
    r.pane.set_zoom_percent(200.0);
    let (_, _, w200, h200) = r.pane.page_rect(0).unwrap();
    assert!((w200 / w100 - 2.0).abs() < 0.01 && (h200 / h100 - 2.0).abs() < 0.01);
    assert_eq!(r.pane.take_zoom_change(), Some(200.0));
    assert_eq!(r.pane.take_zoom_change(), None);

    // Limits are 10 % and 500 %.
    r.pane.set_zoom_percent(1.0);
    assert_eq!(r.pane.zoom_percent(), 10.0);
    r.pane.set_zoom_percent(9000.0);
    assert_eq!(r.pane.zoom_percent(), 500.0);

    // Ctrl+wheel zooms in steps of 10 %; the wheel alone scrolls.
    r.pane.set_zoom_percent(100.0);
    r.pane.wheel_with(2.0, false, true, 400.0);
    assert_eq!(r.pane.zoom_percent(), 120.0);
    r.pane.paste_text(&"A line of text\n".repeat(150));
    r.pane.scroll_by(-1e9);
    let before = r.pane.scroll();
    r.pane.wheel_with(-1.0, false, false, 400.0);
    assert!(r.pane.scroll() > before);
    assert_eq!(r.pane.zoom_percent(), 120.0);

    // The document point under the pointer stays under it.
    r.pane.scroll_by(300.0);
    let y = 400.0;
    let at = r.pane.hit(300.0, y);
    r.pane.wheel_with(1.0, false, true, y);
    let after = r.pane.hit(300.0, y);
    assert!(at.para.abs_diff(after.para) <= 1, "{at:?} -> {after:?}");

    // Hit testing follows the zoom.
    r.pane.set_zoom_percent(50.0);
    r.pane.scroll_by(-1e9);
    let (x, y, h) = r.pane.point_of(DocPos::new(0, 0)).unwrap();
    assert_eq!(r.pane.hit(x + 1.0, y + h / 2.0), DocPos::new(0, 0));
}

#[test]
fn ruler_shows_indents_and_tabs_and_edits_through_commands() {
    let mut r = rig_with(WriterPane::blank());
    r.pane.insert_text("Indented");
    assert!(!r.pane.ruler());
    let flat = r.paint();
    assert_eq!(flat.ruler_items, 0);
    let page_y_off = r.pane.page_rect(0).unwrap().1;

    r.pane.set_ruler(true);
    let shifted = r.pane.page_rect(0).unwrap().1;
    assert!(shifted > page_y_off + 20.0, "the band pushes the page down");
    let s = r.paint();
    // Three markers, the left box and the corner glyph are painted with no tab stops.
    assert!(s.ruler_items >= 4, "{s:?}");

    let info = r.pane.ruler_info().expect("ruler info");
    assert!((info.margin_left_pt - 72.0).abs() < 1.0, "{info:?}");
    assert_eq!(info.tabs.len(), 0);

    assert!(r.pane.apply_ruler_edit(&RulerEdit::SetIndents {
        para: 0,
        left_pt: 36.0,
        first_line_pt: -18.0,
        right_pt: 0.0,
    }));
    assert!(r.pane.apply_ruler_edit(&RulerEdit::AddTab {
        para: 0,
        pos_pt: 144.0,
        kind: RulerTab::Center,
    }));
    let info = r.pane.ruler_info().unwrap();
    assert_eq!(info.indent_left_pt, 36.0);
    assert_eq!(info.first_line_pt, -18.0);
    assert_eq!(info.tabs.len(), 1);
    assert_eq!(info.tabs[0].kind, RulerTab::Center);
    let with_tab = r.paint();
    assert_eq!(
        with_tab.ruler_items,
        s.ruler_items + 1,
        "the tab stop is painted"
    );
    assert!(r.pane.can_undo());
    r.pane.undo();
    assert_eq!(r.pane.ruler_info().unwrap().tabs.len(), 0);
}

#[test]
fn clicking_the_ruler_track_adds_a_tab_and_the_corner_cycles_the_kind() {
    let mut r = rig_with(WriterPane::blank());
    r.pane.insert_text("Text");
    r.pane.set_ruler(true);
    r.paint();
    let (px, _, _, _) = r.pane.page_rect(0).unwrap();
    let pxu = SCALE / 1.25;
    // Corner button, then a click on the track 2 cm into the text area.
    r.click(10.0 * pxu, 12.0 * pxu);
    assert_eq!(r.pane.ruler_tab_kind(), RulerTab::Center);
    let x = px + (72.0 + 56.7) * (96.0 / 72.0) * SCALE;
    r.click(x, 12.0 * pxu);
    let info = r.pane.ruler_info().unwrap();
    assert_eq!(info.tabs.len(), 1, "{info:?}");
    assert_eq!(info.tabs[0].kind, RulerTab::Center);
    // A click on the ruler does not move the text caret.
    assert_eq!(r.pane.caret(), DocPos::new(0, 4));
}

#[test]
fn paragraph_styles_apply_through_commands() {
    let mut r = rig_with(WriterPane::blank());
    r.pane.insert_text("Title text");
    r.pane.enter(false);
    r.pane.insert_text("Body");
    r.pane.set_caret(DocPos::new(0, 3));
    assert_eq!(r.pane.style_state().as_deref(), Some("Normal"));
    let normal_h = r.pane.point_of(DocPos::new(0, 0)).unwrap().2;

    assert_eq!(
        Format::from_ribbon("home.styles.heading-1"),
        Some(Format::Style("Heading1".into()))
    );
    r.pane
        .format(Format::from_ribbon("home.styles.heading-1").unwrap());
    assert_eq!(r.pane.style_state().as_deref(), Some("Heading1"));
    let heading_h = r.pane.point_of(DocPos::new(0, 0)).unwrap().2;
    assert!(
        heading_h > normal_h,
        "Heading 1 is taller: {heading_h} vs {normal_h}"
    );
    let heading = r
        .pane
        .access()
        .unwrap()
        .nodes
        .iter()
        .any(|(_, n)| n.role() == accesskit::Role::Heading);
    assert!(heading, "the paragraph is exposed as a heading");
    // Only the caret's paragraph changed, and it is undoable.
    r.pane.set_caret(DocPos::new(1, 0));
    assert_eq!(r.pane.style_state().as_deref(), Some("Normal"));
    r.pane.undo();
    r.pane.set_caret(DocPos::new(0, 0));
    assert_eq!(r.pane.style_state().as_deref(), Some("Normal"));
    r.pane.redo();

    // Ctrl+Alt+2 applies Heading 2 to a selection of both paragraphs.
    r.pane.select_all();
    r.key_with(
        Key::Char('2'),
        Modifiers {
            ctrl: true,
            alt: true,
            ..Modifiers::default()
        },
    );
    r.pane.set_caret(DocPos::new(1, 0));
    assert_eq!(r.pane.style_state().as_deref(), Some("Heading2"));
    // Style commands are document edits, not share commands.
    assert!(
        r.pane
            .apply_share(Command::DocxSetParagraphStyle {
                para: 0,
                style_id: "Normal".into()
            })
            .is_err()
    );
}

#[test]
fn find_selects_matches_with_next_and_previous() {
    let mut r = rig_with(WriterPane::blank());
    r.pane.paste_text("alpha beta\nBeta gamma beta\nbetamax");
    r.pane.set_caret(DocPos::new(0, 0));
    r.key_with(Key::Char('f'), ctrl());
    assert!(r.pane.find_is_open());
    for c in "beta".chars() {
        r.input(InputEvent::Text(c.to_string()));
    }
    assert_eq!(r.pane.find_query(), "beta");
    assert_eq!(
        r.pane.find_count(),
        4,
        "case-insensitive: beta, Beta, beta, betamax"
    );
    assert_eq!(r.pane.find_status(), "1 of 4");
    assert_eq!(r.pane.selected_text(), "beta");
    assert_eq!(r.pane.selection().0, DocPos::new(0, 6));

    r.key_with(Key::Enter, Modifiers::default());
    assert_eq!(r.pane.selection().0, DocPos::new(1, 0));
    assert_eq!(r.pane.find_status(), "2 of 4");
    r.pane.find_next();
    r.pane.find_next();
    assert_eq!(r.pane.selection().0, DocPos::new(2, 0));
    r.pane.find_next();
    assert_eq!(
        r.pane.selection().0,
        DocPos::new(0, 6),
        "next wraps to the first match"
    );
    r.key_with(
        Key::Enter,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(
        r.pane.selection().0,
        DocPos::new(2, 0),
        "previous wraps to the last"
    );
    r.pane.find_prev();
    assert_eq!(r.pane.selection().0, DocPos::new(1, 11));

    // Highlights are painted; match case narrows; whole word excludes betamax.
    let s = r.paint();
    assert_eq!(s.find_marks, 4);
    r.pane.set_find_match_case(true);
    assert_eq!(r.pane.find_count(), 3);
    r.pane.set_find_match_case(false);
    r.pane.set_find_whole_word(true);
    assert_eq!(r.pane.find_count(), 3);
    r.pane.set_find_whole_word(false);

    // Edits refresh the matches; Backspace edits the query; Escape closes without touching text.
    r.pane.set_caret(DocPos::new(2, 0));
    r.pane.paste_text("beta ");
    assert_eq!(r.pane.find_count(), 5);
    r.key_with(Key::Backspace, Modifiers::default());
    assert_eq!(r.pane.find_query(), "bet");
    r.key_with(Key::Escape, Modifiers::default());
    assert!(!r.pane.find_is_open());
    assert_eq!(r.paint().find_marks, 0);
    assert!(r.pane.text().contains("beta betamax"));

    // No match: nothing moves and the status says so.
    r.pane.set_find_query("zzz");
    assert_eq!(r.pane.find_status(), "No results");
    assert!(!r.pane.find_next());
}
