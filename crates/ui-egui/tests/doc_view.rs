//! Document view tests: pages come from the layout engine, the caret and selection work on
//! layout lines, and the accessibility tree follows the layout.

use std::path::PathBuf;
use std::time::Instant;

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use open365_commands::{Command, Target};
use open365_ui_egui::{DocPos, DocView};

struct State {
    view: DocView,
    target: Target,
}

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

fn harness_for(target: Target) -> Harness<'static, State> {
    let state = State {
        view: DocView::new(),
        target,
    };
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 800.0))
        .build_ui_state(
            |ui, s: &mut State| {
                s.view.show(ui, &mut s.target);
            },
            state,
        );
    h.run();
    h
}

fn open(rel: &str) -> Harness<'static, State> {
    let bytes = std::fs::read(fixture(rel)).expect("fixture");
    harness_for(Target::open_docx(&bytes).expect("open"))
}

fn focus(h: &mut Harness<'_, State>) {
    h.get_by_role(Role::MultilineTextInput).focus();
    h.run();
}

fn click(h: &mut Harness<'_, State>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run();
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
}

/// Lines of paragraph `para` as (start story offset, end story offset), page by page.
fn para_lines(view: &DocView, para: usize) -> Vec<(usize, usize)> {
    view.layout()
        .lines()
        .filter(|(_, l)| l.paragraph == para)
        .map(|(_, l)| (l.start, l.end))
        .collect()
}

/// Glyph quads drawn last frame: text is textured quads from our own atlas, not egui's font
/// texture.
fn glyph_quads(h: &Harness<'_, State>) -> usize {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => {
                Some(m.indices.len() / 6)
            }
            _ => None,
        })
        .sum()
}

#[test]
fn pages_and_lines_come_from_the_layout_engine() {
    let h = open("docx/layout-wrap.docx");
    let layout = h.state().view.layout();
    assert!(!layout.pages.is_empty());
    assert_eq!(h.state().view.page_count(), layout.pages.len());
    assert!(layout.lines().count() > 3);
    // The caret sits on the first line of the first page, at the layout's own position.
    let (page, x, top, height) = h.state().view.caret_point().expect("caret");
    let first = &layout.pages[0].lines[0];
    assert_eq!(page, 0);
    assert!((top - first.top).abs() < 1e-9 && (height - first.height).abs() < 1e-9);
    assert!(
        (x - first.left).abs() < 0.5,
        "caret x {x} line left {}",
        first.left
    );
}

#[test]
fn rendering_covers_the_page_with_glyph_quads() {
    let h = open("docx/plain-paragraphs.docx");
    let quads = glyph_quads(&h);
    assert!(quads > 50, "glyph quads {quads}");
}

#[test]
fn down_and_up_move_by_layout_lines_and_home_end_stay_on_the_line() {
    let mut h = open("docx/layout-wrap.docx");
    focus(&mut h);
    let lines = para_lines(&h.state().view, 0);
    assert!(lines.len() >= 3, "wrap fixture should wrap: {lines:?}");
    let base = lines[0].0;
    h.state_mut().view.set_caret(DocPos::new(0, 2));
    h.key_press(Key::ArrowDown);
    h.run();
    let caret = h.state().view.caret();
    assert_eq!(caret.para, 0);
    assert!(
        caret.offset >= lines[1].0 - base && caret.offset < lines[1].1 - base,
        "caret {caret:?} not on second line {lines:?}"
    );
    h.key_press(Key::Home);
    h.run();
    assert_eq!(h.state().view.caret().offset, lines[1].0 - base);
    h.key_press(Key::End);
    h.run();
    let end = h.state().view.caret().offset;
    assert!(
        end <= lines[1].1 - base && end >= lines[1].1 - base - 1,
        "end {end}"
    );
    // The caret is still drawn on the second line.
    let (_, _, top, _) = h.state().view.caret_point().expect("caret");
    let second_top = h
        .state()
        .view
        .layout()
        .lines()
        .filter(|(_, l)| l.paragraph == 0)
        .nth(1)
        .map(|(_, l)| l.top);
    assert_eq!(Some(top), second_top);
    h.key_press(Key::ArrowUp);
    h.run();
    assert!(h.state().view.caret().offset < lines[1].0 - base);
}

#[test]
fn page_down_moves_the_caret_about_a_screen() {
    let mut h = open("docx/layout-pagination.docx");
    focus(&mut h);
    h.state_mut().view.set_caret(DocPos::new(0, 0));
    h.run();
    h.key_press(Key::PageDown);
    h.run();
    let after = h.state().view.caret();
    assert!(after.para > 0 || after.offset > 20, "caret {after:?}");
    h.key_press(Key::PageUp);
    h.run();
    assert_eq!(h.state().view.caret(), DocPos::new(0, 0));
}

#[test]
fn click_places_the_caret_and_drag_selects() {
    let mut h = open("docx/plain-paragraphs.docx");
    focus(&mut h);
    let view = &h.state().view;
    let line = &view.layout().pages[0].lines[1];
    let p = view.page_point_to_screen(0, line.left + 40.0, line.top + line.height / 2.0);
    let expected = view.position_at(p);
    click(&mut h, p);
    assert_eq!(h.state().view.caret(), expected);
    assert!(h.state().view.caret().para >= 1);

    // Drag from inside the first line to inside the third.
    let view = &h.state().view;
    let l0 = &view.layout().pages[0].lines[0];
    let l2 = &view.layout().pages[0].lines[2];
    let a = view.page_point_to_screen(0, l0.left + 10.0, l0.top + 2.0);
    let b = view.page_point_to_screen(0, l2.left + 30.0, l2.top + 2.0);
    h.event(Event::PointerMoved(a));
    h.event(Event::PointerButton {
        pos: a,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run();
    h.event(Event::PointerMoved(b));
    h.run();
    h.event(Event::PointerMoved(b + vec2(1.0, 0.0)));
    h.run();
    h.event(Event::PointerButton {
        pos: b,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    let view = &h.state().view;
    assert!(view.has_selection());
    let (s, e) = view.selection();
    assert_eq!(s.para, 0);
    assert!(e.para >= 1);
}

#[test]
fn ime_preedit_is_drawn_inline_and_pushes_the_caret() {
    let mut h = open("docx/plain-paragraphs.docx");
    focus(&mut h);
    h.state_mut().view.set_caret(DocPos::new(0, 5));
    h.run();
    let before = h.state().view.caret_point().expect("caret").1;
    h.event(Event::Ime(egui::ImeEvent::Preedit {
        text: "wide".to_owned(),
        active_range_chars: None,
    }));
    h.run();
    let after = h.state().view.caret_point().expect("caret").1;
    assert!(after > before + 10.0, "caret moved {before} -> {after}");
    // Nothing was committed to the document.
    assert_eq!(
        h.state()
            .target
            .docx()
            .and_then(|d| d.paragraph_text(0))
            .as_deref(),
        Some("First paragraph of plain text.")
    );
}

#[test]
fn text_runs_follow_layout_lines() {
    let mut h = open("docx/layout-wrap.docx");
    focus(&mut h);
    let lines = para_lines(&h.state().view, 0);
    let doc = h.get_by_role(Role::MultilineTextInput);
    let runs: Vec<_> = doc
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::TextRun)
        .collect();
    // One run per layout line of the first paragraph (the caret paragraph is published).
    let first_para_runs: Vec<_> = runs.iter().take(lines.len()).collect();
    assert_eq!(first_para_runs.len(), lines.len());
    let mut text = String::new();
    for run in &first_para_runs {
        let d = run.accesskit_node().data().clone();
        let value = d.value().expect("value").to_owned();
        assert_eq!(d.character_lengths().len(), value.chars().count());
        let positions = d.character_positions().expect("positions");
        let widths = d.character_widths().expect("widths");
        assert_eq!(positions.len(), widths.len());
        assert!(
            positions.windows(2).all(|w| w[1] >= w[0]),
            "monotonic positions"
        );
        assert!(widths.iter().all(|w| *w >= 0.0));
        text.push_str(&value);
    }
    assert_eq!(
        text.chars().filter(|c| *c != '\n').count(),
        h.state().view.paragraph_text(0).unwrap().chars().count()
    );
    // Word starts come from the layout's characters.
    let d = first_para_runs[0].accesskit_node().data().clone();
    assert!(d.word_starts().len() > 3);
}

#[test]
fn caret_and_selection_nodes_track_layout_lines() {
    let mut h = open("docx/layout-wrap.docx");
    focus(&mut h);
    let lines = para_lines(&h.state().view, 0);
    let base = lines[0].0;
    // Caret on the second layout line.
    h.state_mut()
        .view
        .set_caret(DocPos::new(0, lines[1].0 - base + 3));
    h.run();
    let doc = h.get_by_role(Role::MultilineTextInput);
    let sel = doc
        .accesskit_node()
        .data()
        .text_selection()
        .expect("selection");
    assert_eq!(sel.focus.character_index, 3);
    let runs: Vec<_> = doc
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::TextRun)
        .collect();
    assert_eq!(runs[1].accesskit_node().locate().0, sel.focus.node);
}

fn long_document(paragraphs: usize) -> Target {
    let bytes = std::fs::read(fixture("docx/plain-paragraphs.docx")).expect("fixture");
    let mut target = Target::open_docx(&bytes).expect("open");
    let sentence =
        "The quick brown fox jumps over the lazy dog while office files fluff difficult fixtures. ";
    let text = vec![sentence.repeat(6); paragraphs].join("\n");
    target
        .apply(Command::DocxReplaceRange {
            start: DocPos::new(0, 0),
            end: DocPos::new(0, 0),
            text,
        })
        .expect("insert");
    target
}

#[test]
fn three_hundred_pages_scroll_with_a_small_paint_budget() {
    let started = Instant::now();
    let mut h = harness_for(long_document(2600));
    let first_frame = started.elapsed();
    focus(&mut h);
    let pages = h.state().view.page_count();
    assert!(pages >= 300, "only {pages} pages");
    println!(
        "300-page document: {pages} pages, first frame {first_frame:?}, layout {:.1} ms",
        h.state().view.last_layout_ms
    );

    // Jump to the middle, then walk the caret down a page at a time, measuring paint.
    h.state_mut().view.set_caret(DocPos::new(1300, 0));
    h.run();
    let mut samples = Vec::new();
    for _ in 0..30 {
        h.key_press(Key::PageDown);
        h.run();
        samples.push(h.state().view.last_paint_ms);
    }
    samples.sort_by(f32::total_cmp);
    let median = samples[samples.len() / 2];
    let worst = samples[samples.len() - 1];
    println!("paint of the visible region: median {median:.2} ms, worst {worst:.2} ms");
    // Only the visible pages' paragraphs are painted.
    assert!(h.state().view.painted_paragraph_count() > 0);
    let quads = glyph_quads(&h);
    println!("glyph quads on the visible pages: {quads}");
    assert!(quads > 500, "glyph quads {quads}");
    assert!(h.state().view.painted_paragraph_count() < 80);
    let budget = if cfg!(debug_assertions) { 400.0 } else { 8.0 };
    assert!(median < budget, "median paint {median} ms over {budget} ms");

    // Editing relays out through the command layer.
    let before = h.state().view.layout().pages.len();
    h.state_mut().view.set_caret(DocPos::new(1300, 3));
    h.event(Event::Text("Z".to_owned()));
    h.run();
    println!(
        "relayout after one keystroke: {:.1} ms",
        h.state().view.last_layout_ms
    );
    assert!(h.state().view.layout().pages.len() >= before);
    assert!(
        h.state()
            .target
            .docx()
            .and_then(|d| d.paragraph_text(1300))
            .is_some_and(|t| t.starts_with("TheZ")),
        "typed text reached the document"
    );
}
