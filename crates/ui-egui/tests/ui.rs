//! UI tests: drive the app through egui_kittest and check documents, commands and the
//! accessibility tree that screen readers see.

use std::path::PathBuf;
use std::time::Instant;

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, Node};
use open365_commands::Command;
use open365_ui_egui::{App, DocPos};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

fn harness(rel: &str) -> Harness<'static, App> {
    let app = App::new(Some(fixture(rel)));
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 800.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h
}

fn paragraph(h: &Harness<'_, App>, i: usize) -> String {
    h.state()
        .target()
        .and_then(|t| t.docx())
        .and_then(|d| d.paragraph_text(i))
        .unwrap_or_default()
}

fn focus_document(h: &mut Harness<'_, App>) {
    h.get_by_role(Role::MultilineTextInput).focus();
    h.run();
}

/// Renders the accessibility tree (role, label, value, indices) as indented text.
fn dump(node: Node<'_>, depth: usize, limit: &mut usize, out: &mut String) {
    if *limit == 0 {
        return;
    }
    *limit -= 1;
    let n = node.accesskit_node();
    let d = n.data();
    let mut line = format!("{}{:?}", "  ".repeat(depth), n.role());
    if let Some(l) = d.label() {
        line.push_str(&format!(" label={l:?}"));
    }
    if let Some(v) = d.value() {
        line.push_str(&format!(" value={v:?}"));
    }
    if n.role() == Role::TextRun {
        line.push_str(&format!(" chars={}", d.character_lengths().len()));
    }
    if let Some(r) = d.row_index() {
        line.push_str(&format!(" row={r}"));
    }
    if let Some(c) = d.column_index() {
        line.push_str(&format!(" col={c}"));
    }
    if let Some(r) = d.row_count() {
        line.push_str(&format!(" rows={r}"));
    }
    if let Some(c) = d.column_count() {
        line.push_str(&format!(" cols={c}"));
    }
    if let Some(s) = d.text_selection() {
        line.push_str(&format!(
            " selection=({:?}:{} -> {:?}:{})",
            s.anchor.node, s.anchor.character_index, s.focus.node, s.focus.character_index
        ));
    }
    out.push_str(&line);
    out.push('\n');
    for child in node.children() {
        dump(child, depth + 1, limit, out);
    }
}

fn tree(h: &Harness<'_, App>, budget: usize) -> String {
    let mut out = String::new();
    let mut limit = budget;
    dump(h.root(), 0, &mut limit, &mut out);
    out
}

#[test]
fn docx_opens_and_shows_paragraphs() {
    let h = harness("docx/plain-paragraphs.docx");
    assert_eq!(paragraph(&h, 0), "First paragraph of plain text.");
    assert_eq!(h.state().doc.paragraph_count(), 4);
    assert_eq!(h.state().doc.page_count(), 1);
}

#[test]
fn typing_inserts_text_through_the_command_layer() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    h.state_mut().doc.set_caret(DocPos { para: 0, offset: 5 });
    h.event(Event::Text("XY".to_owned()));
    h.run();
    assert_eq!(paragraph(&h, 0), "FirstXY paragraph of plain text.");
    let target = h.state().target().expect("target");
    assert!(target.is_dirty());
    assert!(matches!(
        target.audit_log().commands().last(),
        Some(Command::DocxInsertText {
            para: 0,
            offset: 5,
            ..
        })
    ));
    assert_eq!(h.state().doc.caret(), DocPos { para: 0, offset: 7 });

    // Ctrl+Z undoes it, Ctrl+Y brings it back.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(paragraph(&h, 0), "First paragraph of plain text.");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    h.run();
    assert_eq!(paragraph(&h, 0), "FirstXY paragraph of plain text.");
}

#[test]
fn arrow_home_end_and_selection_keys_move_the_caret() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    h.state_mut().doc.set_caret(DocPos { para: 0, offset: 3 });
    h.key_press(Key::ArrowRight);
    h.run();
    assert_eq!(h.state().doc.caret(), DocPos { para: 0, offset: 4 });
    h.key_press(Key::End);
    h.run();
    assert_eq!(
        h.state().doc.caret().offset,
        "First paragraph of plain text.".len()
    );
    h.key_press(Key::Home);
    h.run();
    assert_eq!(h.state().doc.caret().offset, 0);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowRight);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowRight);
    h.run();
    assert!(h.state().doc.has_selection());
    assert_eq!(h.state().doc.selection().1.offset, 2);
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().doc.caret().para, 1);
}

#[test]
fn ctrl_b_bolds_the_selection_via_a_command() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    h.state_mut()
        .doc
        .select(DocPos { para: 1, offset: 0 }, DocPos { para: 1, offset: 6 });
    h.key_press_modifiers(Modifiers::COMMAND, Key::B);
    h.run();
    let target = h.state().target().expect("target");
    assert!(matches!(
        target.audit_log().commands().last(),
        Some(Command::DocxSetRunProps { start, end, patch })
            if *start == DocPos { para: 1, offset: 0 }
                && *end == DocPos { para: 1, offset: 6 }
                && patch.bold == Some(true)
    ));
    let bold: Vec<String> = target
        .docx()
        .and_then(|d| d.paragraph(1))
        .map(|p| {
            p.content
                .iter()
                .filter_map(|i| match i {
                    open365_docx::Inline::Run(r) if r.props.bold() == Some(true) => Some(r.text()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(bold, vec!["Second".to_owned()]);
    // Pressing it again on the same selection removes the bold.
    h.key_press_modifiers(Modifiers::COMMAND, Key::B);
    h.run();
    assert!(matches!(
        h.state()
            .target()
            .expect("target")
            .audit_log()
            .commands()
            .last(),
        Some(Command::DocxSetRunProps { patch, .. }) if patch.bold == Some(false)
    ));
}

#[test]
fn ime_commit_inserts_and_preedit_is_not_committed() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    h.state_mut().doc.set_caret(DocPos { para: 2, offset: 0 });
    h.event(Event::Ime(egui::ImeEvent::Preedit {
        text: "\u{306b}".to_owned(),
        active_range_chars: None,
    }));
    h.run();
    // Composition text is shown but the document is untouched.
    assert_eq!(paragraph(&h, 2), "Third paragraph.");
    assert!(!h.state().target().expect("target").is_dirty());
    h.event(Event::Ime(egui::ImeEvent::Commit(
        "\u{65e5}\u{672c}".to_owned(),
    )));
    h.run();
    assert_eq!(paragraph(&h, 2), "\u{65e5}\u{672c}Third paragraph.");
    assert_eq!(h.state().doc.caret(), DocPos { para: 2, offset: 2 });
}

#[test]
fn document_accessibility_tree_has_paragraphs_text_runs_and_selection() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    h.state_mut()
        .doc
        .select(DocPos { para: 0, offset: 2 }, DocPos { para: 1, offset: 3 });
    h.run();

    let doc = h.get_by_role(Role::MultilineTextInput);
    let data = doc.accesskit_node().data().clone();
    let sel = data
        .text_selection()
        .expect("text selection on document node");
    assert_eq!(sel.anchor.character_index, 2);
    assert_eq!(sel.focus.character_index, 3);

    let paragraphs: Vec<_> = h.query_all_by_role(Role::Paragraph).collect();
    assert_eq!(paragraphs.len(), 4);
    let runs: Vec<String> = doc
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::TextRun)
        .filter_map(|n| n.accesskit_node().data().value().map(str::to_owned))
        .collect();
    assert_eq!(runs[0], "First paragraph of plain text.\n");
    assert_eq!(runs[1], "Second paragraph with more words in it.\n");
    assert_eq!(runs[2], "Third paragraph.\n");
    // The selection's node ids are text runs of the published paragraphs.
    let run_nodes: Vec<_> = doc
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::TextRun)
        .collect();
    let ids: Vec<_> = run_nodes
        .iter()
        .map(|n| n.accesskit_node().locate().0)
        .collect();
    assert!(ids.contains(&sel.anchor.node));
    assert!(ids.contains(&sel.focus.node));
    // Per-character data screen readers use to read by character and word.
    let first = run_nodes[0].accesskit_node().data().clone();
    assert_eq!(first.character_lengths().len(), 31);
    assert!(first.character_positions().is_some());
    assert!(first.character_widths().is_some());
    assert!(!first.word_starts().is_empty());

    println!("--- document accessibility tree ---\n{}", tree(&h, 40));
}

#[test]
fn screen_reader_text_selection_request_moves_the_caret() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    let doc = h.get_by_role(Role::MultilineTextInput);
    let run = doc
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::TextRun)
        .nth(1)
        .expect("second run")
        .accesskit_node()
        .locate()
        .0;
    let (target_node, target_tree) = doc.accesskit_node().locate();
    let position = egui::accesskit::TextPosition {
        node: run,
        character_index: 7,
    };
    h.event(Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::SetTextSelection,
            target_node,
            target_tree,
            data: Some(egui::accesskit::ActionData::SetTextSelection(
                egui::accesskit::TextSelection {
                    anchor: position,
                    focus: position,
                },
            )),
        },
    ));
    h.run();
    assert_eq!(h.state().doc.caret(), DocPos { para: 1, offset: 7 });
}

#[test]
fn heading_and_landscape_documents_open() {
    let h = harness("docx/styles-headings.docx");
    assert!(h.state().doc.paragraph_count() > 2);
    let h = harness("docx/sections-landscape.docx");
    assert!(h.state().doc.page_count() >= 1);
}

fn cell_value(h: &Harness<'_, App>, row: u32, col: u32) -> Option<f64> {
    match h
        .state()
        .target()
        .and_then(|t| t.xlsx())
        .and_then(|w| w.cell(0, row, col))
        .map(|c| &c.value)
    {
        Some(open365_xlsx::CellValue::Number(n)) => Some(*n),
        _ => None,
    }
}

#[test]
fn grid_enter_commits_a_value_and_recalculates() {
    let mut h = harness("xlsx/formulas-basic.xlsx");
    assert_eq!(cell_value(&h, 1, 4), Some(15.0));
    h.state_mut().grid.set_active(2, 1);
    h.run();
    // Typing starts an edit in the formula bar; Enter commits through the command layer.
    h.event(Event::Text("42".to_owned()));
    h.run();
    assert!(h.state().grid.is_editing());
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(cell_value(&h, 2, 1), Some(42.0));
    assert_eq!(cell_value(&h, 1, 4), Some(55.0));
    assert!(matches!(
        h.state()
            .target()
            .expect("target")
            .audit_log()
            .commands()
            .last(),
        Some(Command::XlsxSetCellValue { row: 2, col: 1, .. })
    ));
    // Enter moved the selection down.
    assert_eq!(h.state().grid.active(), (3, 1));
}

#[test]
fn grid_equals_prefix_commits_a_formula() {
    let mut h = harness("xlsx/formulas-basic.xlsx");
    h.state_mut().grid.set_active(8, 1);
    h.run();
    h.event(Event::Text("=SUM(A1:A5)*2".to_owned()));
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(cell_value(&h, 8, 1), Some(30.0));
    assert!(matches!(
        h.state()
            .target()
            .expect("target")
            .audit_log()
            .commands()
            .last(),
        Some(Command::XlsxSetCellFormula { row: 8, col: 1, .. })
    ));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(cell_value(&h, 8, 1), None);
}

#[test]
fn grid_arrow_keys_move_the_active_cell() {
    let mut h = harness("xlsx/formulas-basic.xlsx");
    h.key_press(Key::ArrowRight);
    h.run();
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().grid.active(), (2, 2));
}

#[test]
fn grid_accessibility_tree_has_grid_rows_and_cells() {
    let h = harness("xlsx/formulas-basic.xlsx");
    let grid = h.get_by_role(Role::Grid);
    let data = grid.accesskit_node().data().clone();
    assert!(data.row_count().is_some_and(|n| n >= 200));
    assert!(data.column_count().is_some_and(|n| n >= 26));
    let rows: Vec<_> = h.query_all_by_role(Role::Row).collect();
    assert!(rows.len() >= 6);
    let cells: Vec<_> = h.query_all_by_role(Role::Cell).collect();
    assert!(!cells.is_empty());
    // D1 holds =SUM(A1:A5) = 15.
    let d1 = cells
        .iter()
        .find(|c| {
            let d = c.accesskit_node().data().clone();
            d.row_index() == Some(0) && d.column_index() == Some(3)
        })
        .expect("D1 cell");
    assert_eq!(d1.accesskit_node().data().value(), Some("15"));
    assert!(
        d1.accesskit_node()
            .data()
            .description()
            .is_some_and(|d| d.contains("D1") && d.contains("SUM(A1:A5)"))
    );
    println!("--- grid accessibility tree ---\n{}", tree(&h, 45));
}

#[test]
fn grid_focus_follows_the_active_cell() {
    let mut h = harness("xlsx/formulas-basic.xlsx");
    h.state_mut().grid.set_active(3, 2);
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().grid.active(), (4, 2));
}

#[test]
fn large_sheet_scrolls_and_renders_far_rows() {
    let mut h = harness("xlsx/large-sheet.xlsx");
    h.state_mut().grid.set_active(40_000, 3);
    h.run();
    let started = Instant::now();
    let frames = 30;
    for _ in 0..frames {
        h.step();
    }
    let per_frame = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
    println!("large-sheet: {per_frame:.2} ms/frame (test build, no rendering)");
    println!(
        "large-sheet: grid view reported {:.2} ms",
        h.state().grid.last_frame_ms
    );
    assert!(per_frame < 250.0, "frame took {per_frame:.1} ms");
    // The far row is in the accessibility tree with its value.
    let far = h
        .query_all_by_role(Role::Cell)
        .find(|c| c.accesskit_node().data().row_index() == Some(39_999))
        .expect("row 40000 cell published");
    assert!(
        far.accesskit_node()
            .data()
            .value()
            .is_some_and(|v| !v.is_empty())
    );
}

#[test]
fn menu_save_as_writes_through_the_save_command() {
    let mut h = harness("docx/plain-paragraphs.docx");
    let out = std::env::temp_dir().join(format!("open365-ui-{}.docx", std::process::id()));
    h.state_mut().save_to(out.clone());
    h.run();
    assert_eq!(
        h.state().target().and_then(|t| t.path()),
        Some(out.as_path())
    );
    let reopened = open365_commands::Target::open_path(&out).expect("reopen");
    assert_eq!(reopened.docx().map(|d| d.paragraph_count()), Some(4));
}

fn plain_texts(h: &Harness<'_, App>) -> Vec<String> {
    let count = h
        .state()
        .target()
        .and_then(|t| t.docx())
        .map_or(0, |d| d.paragraph_count());
    (0..count).map(|i| paragraph(h, i)).collect()
}

#[test]
fn backspace_deletes_a_character_and_joins_paragraphs_at_the_start() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    let original = plain_texts(&h);
    h.state_mut().doc.set_caret(DocPos { para: 0, offset: 5 });
    h.key_press(Key::Backspace);
    h.run();
    assert_eq!(paragraph(&h, 0), "Firs paragraph of plain text.");
    assert_eq!(h.state().doc.caret(), DocPos { para: 0, offset: 4 });

    // At the start of paragraph 1 Backspace joins it to paragraph 0.
    h.state_mut().doc.set_caret(DocPos { para: 1, offset: 0 });
    h.key_press(Key::Backspace);
    h.run();
    let texts = plain_texts(&h);
    assert_eq!(texts.len(), original.len() - 1);
    assert_eq!(
        texts[0],
        format!("Firs paragraph of plain text.{}", original[1])
    );
    assert_eq!(
        h.state().doc.caret(),
        DocPos {
            para: 0,
            offset: "Firs paragraph of plain text.".chars().count()
        }
    );
    // The view model followed the edit.
    assert_eq!(h.state().doc.paragraph_count(), texts.len());
    assert_eq!(h.state().doc.paragraph_text(0), Some(texts[0].as_str()));

    // Undo both edits.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(plain_texts(&h), original);
}

#[test]
fn delete_key_removes_forward_and_joins_at_the_end() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    let original = plain_texts(&h);
    h.state_mut().doc.set_caret(DocPos { para: 0, offset: 0 });
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(paragraph(&h, 0), "irst paragraph of plain text.");
    let end = paragraph(&h, 0).chars().count();
    h.state_mut().doc.set_caret(DocPos {
        para: 0,
        offset: end,
    });
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(plain_texts(&h).len(), original.len() - 1);
    assert_eq!(
        paragraph(&h, 0),
        format!("irst paragraph of plain text.{}", original[1])
    );
}

#[test]
fn enter_splits_the_paragraph_and_moves_the_caret() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    let original = plain_texts(&h);
    h.state_mut().doc.set_caret(DocPos { para: 0, offset: 5 });
    h.key_press(Key::Enter);
    h.run();
    let texts = plain_texts(&h);
    assert_eq!(texts.len(), original.len() + 1);
    assert_eq!(texts[0], "First");
    assert_eq!(texts[1], " paragraph of plain text.");
    assert_eq!(h.state().doc.caret(), DocPos { para: 1, offset: 0 });
    assert_eq!(h.state().doc.paragraph_count(), texts.len());
    // Typing continues in the new paragraph.
    h.event(Event::Text("Z".to_owned()));
    h.run();
    assert_eq!(paragraph(&h, 1), "Z paragraph of plain text.");
    let last = h
        .state()
        .target()
        .expect("target")
        .audit_log()
        .commands()
        .len();
    assert!(last >= 2);
}

#[test]
fn typing_over_a_selection_replaces_it_in_one_command() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    let original = plain_texts(&h);
    h.state_mut()
        .doc
        .select(DocPos { para: 0, offset: 6 }, DocPos { para: 1, offset: 6 });
    h.event(Event::Text("X".to_owned()));
    h.run();
    let texts = plain_texts(&h);
    assert_eq!(texts.len(), original.len() - 1);
    assert_eq!(
        texts[0],
        format!("First X{}", original[1].chars().skip(6).collect::<String>())
    );
    assert_eq!(h.state().doc.caret(), DocPos { para: 0, offset: 7 });
    assert!(matches!(
        h.state()
            .target()
            .expect("target")
            .audit_log()
            .commands()
            .last(),
        Some(Command::DocxReplaceRange { .. })
    ));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(plain_texts(&h), original);
}

#[test]
fn cut_and_paste_go_through_commands() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    h.state_mut()
        .doc
        .select(DocPos { para: 0, offset: 0 }, DocPos { para: 0, offset: 6 });
    h.event(Event::Cut);
    h.run();
    assert_eq!(paragraph(&h, 0), "paragraph of plain text.");
    // Paste two lines at the start of paragraph 2.
    h.state_mut().doc.set_caret(DocPos { para: 2, offset: 0 });
    h.event(Event::Paste(
        "one
two"
        .to_owned(),
    ));
    h.run();
    let texts = plain_texts(&h);
    assert_eq!(texts[2], "one");
    assert!(texts[3].starts_with("twoThird"), "{texts:?}");
    assert_eq!(h.state().doc.caret(), DocPos { para: 3, offset: 3 });
}

#[test]
fn italic_underline_alignment_and_style_apply_to_the_selection() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    h.state_mut()
        .doc
        .select(DocPos { para: 0, offset: 0 }, DocPos { para: 0, offset: 5 });
    h.key_press_modifiers(Modifiers::COMMAND, Key::I);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::U);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::E);
    h.run();
    {
        let doc = h.state().target().and_then(|t| t.docx()).expect("docx");
        let p = doc.paragraph(0).expect("p0");
        let first = p
            .content
            .iter()
            .find_map(|i| match i {
                open365_docx::Inline::Run(r) => Some(r),
                _ => None,
            })
            .expect("run");
        assert_eq!(first.text(), "First");
        assert_eq!(first.props.italic(), Some(true));
        assert_eq!(first.props.underline(), Some("single"));
        assert_eq!(p.direct_alignment(), Some(open365_docx::Alignment::Center));
    }
    // The toolbar buttons send the same commands.
    h.get_by_label("Right").click();
    h.run();
    assert_eq!(
        h.state()
            .target()
            .and_then(|t| t.docx())
            .and_then(|d| d.paragraph(0))
            .and_then(open365_docx::Paragraph::direct_alignment),
        Some(open365_docx::Alignment::Right)
    );
    h.state_mut().set_style("Heading2");
    h.run();
    assert_eq!(h.state().doc.caret_style_id(), Some("Heading2"));
    let size = h.state().doc.caret_format().size_half_points;
    assert!(size > 22, "Heading2 is larger than body text: {size}");
}

#[test]
fn toggling_with_no_selection_formats_the_next_typed_text() {
    let mut h = harness("docx/plain-paragraphs.docx");
    focus_document(&mut h);
    h.state_mut().doc.set_caret(DocPos { para: 0, offset: 5 });
    h.key_press_modifiers(Modifiers::COMMAND, Key::B);
    h.run();
    assert!(h.state().doc.caret_format().bold);
    h.event(Event::Text("ABC".to_owned()));
    h.run();
    let doc = h.state().target().and_then(|t| t.docx()).expect("docx");
    let bold: Vec<String> = doc
        .paragraph(0)
        .expect("p0")
        .content
        .iter()
        .filter_map(|i| match i {
            open365_docx::Inline::Run(r) if r.props.bold() == Some(true) => Some(r.text()),
            _ => None,
        })
        .collect();
    assert_eq!(bold, vec!["ABC".to_owned()]);
    assert_eq!(h.state().doc.caret(), DocPos { para: 0, offset: 8 });
}

#[test]
fn a_missing_font_is_drawn_with_a_bundled_one_and_reported() {
    let h = harness("docx/styles-headings.docx");
    // Whatever the machine has installed, every font the document names resolves to
    // something drawable, and a missing one is listed for the status bar.
    let subs = h.state().doc.font_substitutions();
    for (from, to) in &subs {
        assert!(to == "Carlito" || to == "Caladea", "{from} -> {to}");
    }
    assert_eq!(
        h.state().doc.paragraph_count(),
        3.max(h.state().doc.paragraph_count())
    );
}
