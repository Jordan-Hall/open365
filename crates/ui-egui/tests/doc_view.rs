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
    // One frame each: the blinking caret asks for repaints, and a press held across them is no click.
    h.step();
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
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
    let relayout = h.state().view.last_layout_ms;
    let layout_budget = if cfg!(debug_assertions) { 400.0 } else { 16.0 };
    assert!(
        relayout < layout_budget,
        "relayout after a keystroke {relayout} ms over {layout_budget} ms"
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

#[test]
fn pictures_are_laid_out_and_published_as_image_nodes() {
    let h = open("docx/inline-image.docx");
    let layout = h.state().view.layout();
    assert_eq!(layout.pages[0].images.len(), 1);
    let images: Vec<_> = h.query_all_by_role(Role::Image).collect();
    assert_eq!(images.len(), 1, "one image node");
}

#[test]
fn export_pdf_writes_a_tagged_pdf_of_the_laid_out_document() {
    let mut h = open("docx/plain-paragraphs.docx");
    let State { view, target } = h.state_mut();
    let out = view
        .export_pdf(target, &open365_pdf::PdfMetadata::default())
        .expect("export");
    assert!(out.bytes.starts_with(b"%PDF-1.7"), "not a PDF");
    assert!(
        out.bytes.windows(15).any(|w| w == b"/StructTreeRoot"),
        "not tagged"
    );
}

#[test]
fn print_hands_the_pdf_to_the_operating_system_by_path_not_script_text() {
    let path = PathBuf::from("C:/tmp/a b; calc.pdf");
    let cmd = open365_ui_egui::print_command(&path);
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if cfg!(windows) {
        assert!(
            !args.iter().any(|a| a.contains("calc.pdf")),
            "path leaked into the script: {args:?}"
        );
        assert_eq!(cmd.get_program().to_string_lossy(), "powershell");
        assert!(
            cmd.get_envs()
                .any(|(k, v)| k == "OPEN365_PRINT_FILE" && v == Some(path.as_os_str()))
        );
    } else {
        assert_eq!(cmd.get_program().to_string_lossy(), "lp");
    }
}

/// Undo or redo through the view, then one frame: (milliseconds in the command layer,
/// milliseconds of layout in the frame).
fn history_step(h: &mut Harness<'_, State>, undo: bool) -> (f32, f32) {
    let started = Instant::now();
    let State { view, target } = h.state_mut();
    let result = if undo {
        view.undo(target)
    } else {
        view.redo(target)
    };
    result.expect("history");
    let command_ms = started.elapsed().as_secs_f32() * 1000.0;
    h.run();
    (command_ms, h.state().view.last_layout_ms)
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Undo and redo on a 300-page document: layout time per step. Prints the timings.
#[test]
fn undo_and_redo_use_the_incremental_layout() {
    let mut h = harness_for(long_document(2600));
    focus(&mut h);
    let pages = h.state().view.page_count();
    h.state_mut().view.set_caret(DocPos::new(1300, 3));
    let (mut undo_cmd, mut undo_layout) = (Vec::new(), Vec::new());
    let (mut redo_cmd, mut redo_layout) = (Vec::new(), Vec::new());
    for _ in 0..5 {
        h.event(Event::Text("Z".to_owned()));
        h.run();
        let (c, l) = history_step(&mut h, true);
        undo_cmd.push(c);
        undo_layout.push(l);
        let stats = h.state().view.layout_stats();
        assert!(!stats.full, "undo laid everything out again: {stats:?}");
        assert!(
            stats.blocks_laid_out <= 3,
            "undo laid out {} blocks",
            stats.blocks_laid_out
        );
        let (c, l) = history_step(&mut h, false);
        redo_cmd.push(c);
        redo_layout.push(l);
        history_step(&mut h, true);
    }
    println!(
        "{pages}-page document: undo command {:.1} ms + layout {:.1} ms; redo command {:.1} ms + layout {:.1} ms",
        median(undo_cmd),
        median(undo_layout.clone()),
        median(redo_cmd),
        median(redo_layout)
    );
    assert_eq!(
        h.state()
            .target
            .docx()
            .and_then(|d| d.paragraph_text(1300))
            .map(|t| t.starts_with("The ")),
        Some(true),
        "undo restored the text"
    );
}

/// Undoing a paste that added paragraphs and redoing it gives the layout a full pass gives.
#[test]
fn undo_after_a_multi_paragraph_paste_matches_a_fresh_layout() {
    let mut h = open("docx/plain-paragraphs.docx");
    let fresh = open("docx/plain-paragraphs.docx");
    focus(&mut h);
    let before = h.state().view.paragraph_count();
    h.event(Event::Paste("one\ntwo\nthree\n".repeat(40)));
    h.run();
    assert!(h.state().view.paragraph_count() > before + 100);
    let pasted = h.state().view.layout().clone();
    let State { view, target } = h.state_mut();
    view.undo(target).expect("undo");
    h.run();
    assert_eq!(h.state().view.paragraph_count(), before);
    assert_eq!(h.state().view.layout(), fresh.state().view.layout());
    let State { view, target } = h.state_mut();
    view.redo(target).expect("redo");
    h.run();
    assert_eq!(h.state().view.layout(), &pasted);
}

/// Glyph quads drawn last frame whose colour is not fully opaque: dimmed text.
fn dimmed_quads(h: &Harness<'_, State>) -> usize {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => Some(
                m.vertices
                    .chunks(4)
                    .filter(|q| q[0].color.a() < 255)
                    .count(),
            ),
            _ => None,
        })
        .sum()
}

fn glyphs(lines: &[open365_layout::Line]) -> usize {
    lines
        .iter()
        .flat_map(|l| &l.runs)
        .map(|r| r.glyphs.len())
        .sum()
}

#[test]
fn headers_and_footers_are_drawn_dimmed_and_published_read_only() {
    let mut h = open("docx/layout-hf-basic.docx");
    // Small enough that every page is on screen.
    h.state_mut().view.set_zoom(Some(0.25));
    h.run();
    h.run();
    let pages = h.state().view.layout().pages.clone();
    assert!(pages.len() <= 3, "{} pages", pages.len());
    let page = pages[0].clone();
    assert!(
        !page.header.is_empty() && !page.footer.is_empty(),
        "fixture has furniture"
    );
    let furniture: usize = pages
        .iter()
        .map(|p| glyphs(&p.header) + glyphs(&p.footer))
        .sum();
    // Blank glyphs (spaces) have no quad, so the dimmed quads are at most the glyph count.
    let dimmed = dimmed_quads(&h);
    assert!(
        dimmed > furniture / 2 && dimmed <= furniture,
        "{dimmed} dimmed quads for {furniture} header and footer glyphs"
    );
    assert!(glyph_quads(&h) > dimmed, "the body is drawn in full");

    // Both are regions of read-only static text, not part of the editable text.
    let header = h
        .query_all_by_label("Page header")
        .next()
        .expect("header region");
    assert_eq!(header.accesskit_node().role(), Role::Header);
    assert!(header.accesskit_node().data().is_read_only());
    let footer = h
        .query_all_by_label("Page footer")
        .next()
        .expect("footer region");
    assert_eq!(footer.accesskit_node().role(), Role::Footer);
    let want: String = page
        .header
        .iter()
        .map(|l| l.text.trim())
        .collect::<Vec<_>>()
        .join("");
    let lines: Vec<_> = header
        .children_recursive()
        .filter(|c| c.accesskit_node().role() == Role::Label)
        .collect();
    let got: Vec<String> = lines
        .iter()
        .filter_map(|c| c.accesskit_node().value())
        .collect();
    assert!(!got.is_empty(), "header lines are published");
    assert!(
        want.starts_with(got[0].trim()) || got.join("").trim().starts_with(want.trim()),
        "{got:?} vs {want:?}"
    );
    for line in lines {
        assert!(line.accesskit_node().data().is_read_only());
    }
}

#[test]
fn footnote_lines_are_drawn_in_full_and_published() {
    let mut h = open("docx/layout-fn-basic.docx");
    h.state_mut().view.set_zoom(Some(0.25));
    h.run();
    h.run();
    let pages = h.state().view.layout().pages.clone();
    assert!(pages.len() <= 3, "{} pages", pages.len());
    assert!(glyphs(&pages[0].notes) > 0, "fixture has footnote text");
    let furniture: usize = pages
        .iter()
        .map(|p| glyphs(&p.header) + glyphs(&p.footer))
        .sum();
    assert_eq!(
        dimmed_quads(&h),
        furniture,
        "only header and footer are dimmed"
    );
    let notes: Vec<_> = h.query_all_by_label("Footnotes").collect();
    assert!(!notes.is_empty(), "footnote regions are published");
    for region in notes {
        assert!(region.accesskit_node().data().is_read_only());
        assert!(region.children_recursive().count() > 0);
    }
}

#[test]
fn pictures_in_headers_and_footers_resolve_through_their_own_part() {
    let dir = fixture("external/ms-templates");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("no external templates; skipped");
        return;
    };
    let mut checked = 0;
    for path in entries.flatten().map(|e| e.path()) {
        if path.extension().and_then(|e| e.to_str()) != Some("docx") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(target) = Target::open_docx(&bytes) else {
            continue;
        };
        let mut h = harness_for(target);
        let layout = h.state().view.layout();
        let furniture_images = layout
            .pages
            .iter()
            .map(|p| p.header_images.len() + p.footer_images.len() + p.note_images.len())
            .sum::<usize>();
        if furniture_images == 0 {
            continue;
        }
        h.state_mut().view.set_zoom(Some(0.5));
        h.run();
        h.run();
        let page = h.state().view.layout().pages[0].clone();
        let want = page.images.len()
            + page.header_images.len()
            + page.note_images.len()
            + page.footer_images.len();
        let drawn = h.query_all_by_role(Role::Image).count();
        assert!(drawn >= page.header_images.len() + page.footer_images.len());
        assert!(
            drawn <= want,
            "{}: {drawn} nodes for {want} pictures",
            path.display()
        );
        // A picture resolved from a header part is a texture, not the grey placeholder frame.
        let textured = h
            .output()
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.color == egui::Color32::from_gray(160)))
            .count();
        assert_eq!(
            textured,
            0,
            "{}: a picture was not resolved",
            path.display()
        );
        checked += 1;
        if checked == 3 {
            break;
        }
    }
    eprintln!("templates with furniture pictures checked: {checked}");
}

/// Paragraphs inside block-level content controls are addressable: a layout line's
/// `paragraph` is the `Document::paragraph` index, typing changes that paragraph, and the saved
/// file reopens with the edit (`layout-sdt`).
#[test]
fn typing_in_a_block_content_control_edits_that_paragraph() {
    let mut h = open("docx/layout-sdt.docx");
    focus(&mut h);
    let count = h.state().target.docx().expect("docx").paragraph_count();
    // The layout never addresses a paragraph past the document's.
    let lines: Vec<(usize, String)> = h
        .state()
        .view
        .layout()
        .lines()
        .map(|(_, l)| (l.paragraph, l.text.to_string()))
        .collect();
    assert!(lines.iter().all(|(p, _)| *p < count));
    let (para, _) = lines
        .iter()
        .find(|(_, t)| t.starts_with("Block control paragraph"))
        .cloned()
        .expect("line of the control paragraph");
    assert_eq!(
        h.state().target.docx().and_then(|d| d.paragraph_text(para)),
        Some("Block control paragraph".to_owned())
    );
    h.state_mut().view.set_caret(DocPos::new(para, 5));
    h.event(Event::Text("Z".to_owned()));
    h.run();
    assert_eq!(
        h.state().target.docx().and_then(|d| d.paragraph_text(para)),
        Some("BlockZ control paragraph".to_owned())
    );
    let others: Vec<String> = (0..count)
        .filter(|i| *i != para)
        .filter_map(|i| h.state().target.docx().and_then(|d| d.paragraph_text(i)))
        .collect();
    assert!(others.iter().all(|t| !t.contains('Z')));
    let saved = h.state_mut().target.snapshot_bytes().expect("save");
    let reopened = Target::open_docx(&saved).expect("reopen");
    assert_eq!(
        reopened.docx().and_then(|d| d.paragraph_text(para)),
        Some("BlockZ control paragraph".to_owned())
    );
    // The control is still a control: the paragraph stays inside the w:sdt.
    let doc = reopened.docx().expect("docx");
    assert!(
        doc.body()
            .blocks
            .iter()
            .any(|b| matches!(b, open365_docx::Block::Sdt(_)))
    );
}
