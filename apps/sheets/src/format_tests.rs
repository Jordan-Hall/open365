//! Home tab formatting controls, driven the way the shell drives them: control ids through the
//! `ShellHost`, results read back from the workbook, undo through the command layer.

use super::*;
use open365_xlsx::{
    BorderStyle as B, CellFormat, ColorSource, FillPattern, HorizontalAlignment as H, Rgb,
    Underline, VerticalAlignment as V,
};

fn state_with_workbook() -> State {
    let mut s = State::new();
    // A frozen clock: the Format Painter double click window never depends on real time.
    s.clock.frozen = Some(0.0);
    s.new_blank();
    s
}

fn host(s: &mut State) -> Host<'_> {
    Host {
        s,
        theme: Shell::new(AppKind::Sheets, Mode::system()).theme,
    }
}

fn wb(s: &State) -> &Workbook {
    s.workbook().expect("workbook")
}

fn cell(s: &State, row: u32, col: u32) -> CellFormat {
    wb(s).cell_format(0, row, col).clone()
}

fn code(s: &State, row: u32, col: u32) -> String {
    wb(s)
        .styles()
        .number_format(wb(s).cell_xf(0, row, col))
        .to_owned()
}

fn put(s: &mut State, row: u32, col: u32, v: f64) {
    s.apply(Command::XlsxSetCellValue {
        sheet: 0,
        row,
        col,
        value: open365_commands::CellEntry::Number(v),
    });
}

fn click(s: &mut State, id: &str) {
    host(s).run(id, None);
    assert!(!s.status.starts_with("Command failed"), "{}", s.status);
}

fn category(s: &mut State) -> Option<String> {
    host(s).value("home.number.number-format")
}

#[test]
fn blank_workbook_shows_the_default_font() {
    let mut s = state_with_workbook();
    let h = host(&mut s);
    assert_eq!(h.value("home.font.font").as_deref(), Some("Aptos Narrow"));
    assert_eq!(h.value("home.font.font-size").as_deref(), Some("11"));
    assert_eq!(
        h.value("home.number.number-format").as_deref(),
        Some("General")
    );
}

#[test]
fn controls_are_disabled_without_a_workbook() {
    let mut s = State::new();
    s.target = None;
    let h = host(&mut s);
    for id in [
        "home.font.bold",
        "home.number.percent-style",
        "home.styles.cell-styles",
    ] {
        assert!(!h.state(id).enabled, "{id}");
    }
    assert!(h.menu("home.font.borders").is_none());
}

#[test]
fn every_font_alignment_number_and_style_control_is_wired() {
    let mut s = state_with_workbook();
    let h = host(&mut s);
    let ids = [
        "home.clipboard.format-painter",
        "home.font.font",
        "home.font.font-size",
        "home.font.increase-font-size",
        "home.font.decrease-font-size",
        "home.font.bold",
        "home.font.italic",
        "home.font.underline",
        "home.font.borders",
        "home.font.fill-colour",
        "home.font.font-colour",
        "home.alignment.top-align",
        "home.alignment.middle-align",
        "home.alignment.bottom-align",
        "home.alignment.align-left",
        "home.alignment.centre",
        "home.alignment.align-right",
        "home.alignment.orientation",
        "home.alignment.increase-indent",
        "home.alignment.wrap-text",
        "home.number.number-format",
        "home.number.accounting-number-format",
        "home.number.percent-style",
        "home.number.comma-style",
        "home.number.increase-decimal",
        "home.number.decrease-decimal",
        "home.styles.cell-styles",
        "home.editing.clear",
    ];
    for id in ids {
        assert!(
            h.state(id).enabled || h.menu(id).is_some(),
            "{id} is not wired"
        );
    }
    // Decrease Indent has nothing to do on an unindented cell, as in Excel.
    assert!(!h.state("home.alignment.decrease-indent").enabled);
}

#[test]
fn font_toggles_reflect_the_active_cell_and_undo() {
    let mut s = state_with_workbook();
    for id in ["home.font.bold", "home.font.italic", "home.font.underline"] {
        assert!(!host(&mut s).state(id).checked, "{id}");
        click(&mut s, id);
        assert!(host(&mut s).state(id).checked, "{id} after click");
    }
    let f = cell(&s, 1, 1).font;
    assert!(f.bold && f.italic && f.underline == Underline::Single);
    // Another cell is not bold, and the button follows the active cell.
    s.grid.set_active(1, 2);
    assert!(!host(&mut s).state("home.font.bold").checked);
    s.grid.set_active(1, 1);
    click(&mut s, "home.font.bold");
    assert!(!cell(&s, 1, 1).font.bold);
    s.undo();
    assert!(cell(&s, 1, 1).font.bold);
    s.redo();
    assert!(!cell(&s, 1, 1).font.bold);
}

#[test]
fn underline_menu_switches_between_single_and_double() {
    let mut s = state_with_workbook();
    click(&mut s, "fmt.underline#double");
    assert_eq!(cell(&s, 1, 1).font.underline, Underline::Double);
    assert!(host(&mut s).state("home.font.underline").checked);
    click(&mut s, "fmt.underline#single");
    assert_eq!(cell(&s, 1, 1).font.underline, Underline::Single);
    click(&mut s, "home.font.underline");
    assert_eq!(cell(&s, 1, 1).font.underline, Underline::None);
    click(&mut s, "fmt.strikethrough");
    assert!(cell(&s, 1, 1).font.strike);
}

#[test]
fn font_name_size_and_steps() {
    let mut s = state_with_workbook();
    click(&mut s, "fmt.font-name#Arial");
    click(&mut s, "fmt.font-size#14");
    assert_eq!(
        host(&mut s).value("home.font.font").as_deref(),
        Some("Arial")
    );
    assert_eq!(
        host(&mut s).value("home.font.font-size").as_deref(),
        Some("14")
    );
    click(&mut s, "home.font.increase-font-size");
    assert_eq!(cell(&s, 1, 1).font.size, 16.0);
    click(&mut s, "home.font.decrease-font-size");
    click(&mut s, "home.font.decrease-font-size");
    assert_eq!(cell(&s, 1, 1).font.size, 12.0);
    let sizes = host(&mut s).menu("home.font.font-size").expect("menu");
    assert!(sizes.iter().any(|e| e.id == "fmt.font-size#11"));
    let fonts = host(&mut s).menu("home.font.font").expect("menu");
    assert!(fonts.iter().any(|e| e.id == "fmt.font-name#Arial"));
}

#[test]
fn alignment_buttons_toggle_and_follow_the_cell() {
    let mut s = state_with_workbook();
    // Bottom Align is on in a blank cell, as in Excel.
    assert!(host(&mut s).state("home.alignment.bottom-align").checked);
    click(&mut s, "home.alignment.centre");
    assert_eq!(cell(&s, 1, 1).alignment.horizontal, H::Center);
    assert!(host(&mut s).state("home.alignment.centre").checked);
    click(&mut s, "home.alignment.align-left");
    assert_eq!(cell(&s, 1, 1).alignment.horizontal, H::Left);
    click(&mut s, "home.alignment.align-left");
    assert_eq!(cell(&s, 1, 1).alignment.horizontal, H::General);
    click(&mut s, "home.alignment.align-right");
    assert!(host(&mut s).state("home.alignment.align-right").checked);
    click(&mut s, "home.alignment.top-align");
    assert_eq!(cell(&s, 1, 1).alignment.vertical, V::Top);
    click(&mut s, "home.alignment.middle-align");
    assert_eq!(cell(&s, 1, 1).alignment.vertical, V::Center);
    assert!(host(&mut s).state("home.alignment.middle-align").checked);
    click(&mut s, "home.alignment.bottom-align");
    assert_eq!(cell(&s, 1, 1).alignment.vertical, V::Bottom);
    click(&mut s, "home.alignment.wrap-text");
    assert!(host(&mut s).state("home.alignment.wrap-text").checked);
    click(&mut s, "home.alignment.increase-indent");
    click(&mut s, "home.alignment.increase-indent");
    assert_eq!(cell(&s, 1, 1).alignment.indent, 2);
    click(&mut s, "home.alignment.decrease-indent");
    assert_eq!(cell(&s, 1, 1).alignment.indent, 1);
}

#[test]
fn orientation_menu_sets_and_clears_the_angle() {
    let mut s = state_with_workbook();
    click(&mut s, "fmt.rotate#45");
    assert_eq!(cell(&s, 1, 1).alignment.rotation_degrees(), Some(45));
    click(&mut s, "fmt.rotate#45");
    assert_eq!(cell(&s, 1, 1).alignment.rotation_degrees(), Some(0));
    click(&mut s, "fmt.rotate#-90");
    assert_eq!(cell(&s, 1, 1).alignment.rotation_degrees(), Some(-90));
    click(&mut s, "fmt.rotate#stacked");
    assert_eq!(cell(&s, 1, 1).alignment.rotation_degrees(), None);
}

#[test]
fn colours_remember_the_last_choice() {
    let mut s = state_with_workbook();
    click(&mut s, "home.font.fill-colour");
    assert_eq!(cell(&s, 1, 1).fill.pattern, FillPattern::Solid);
    s.grid.set_active(2, 1);
    click(&mut s, "fmt.fill#rgb:00B050");
    s.grid.set_active(3, 1);
    // The button now applies the colour last chosen from the menu.
    click(&mut s, "home.font.fill-colour");
    assert_eq!(cell(&s, 3, 1).fill, cell(&s, 2, 1).fill);
    click(&mut s, "fmt.fill#none");
    assert_eq!(cell(&s, 3, 1).fill.pattern, FillPattern::None);
    click(&mut s, "fmt.font-colour#theme:4:0.39998");
    assert!(matches!(
        cell(&s, 3, 1).font.color.source,
        ColorSource::Theme(4)
    ));
    click(&mut s, "fmt.font-colour#auto");
    assert_eq!(cell(&s, 3, 1).font.color.source, ColorSource::Auto);
    // Red is the font colour button's first colour; the menu choice above set it to automatic.
    click(&mut s, "home.font.font-colour");
    assert_eq!(cell(&s, 3, 1).font.color.source, ColorSource::Auto);
    click(&mut s, "fmt.font-colour#rgb:FF0000");
    assert_eq!(
        cell(&s, 3, 1).font.color.source,
        ColorSource::Rgb(Rgb { r: 255, g: 0, b: 0 })
    );
    let menu = host(&mut s).menu("home.font.fill-colour").expect("menu");
    assert!(menu.iter().any(|e| e.id == "fmt.fill#none"));
    // The picker is a real menu: theme colour rows, tints, standard colours.
    let swatches: Vec<&Vec<Vec<open365_ui_shell::Swatch>>> = menu
        .iter()
        .filter_map(|e| match &e.kind {
            open365_ui_shell::EntryKind::Swatches { rows, .. } => Some(rows),
            _ => None,
        })
        .collect();
    assert_eq!(swatches.len(), 3, "theme row, tints, standard colours");
    assert_eq!(swatches[0][0].len(), 10);
    assert_eq!(swatches[1].len(), 5);
    click(&mut s, "fmt.fill#more");
}

#[test]
fn colour_buttons_start_with_excels_defaults() {
    let mut s = state_with_workbook();
    click(&mut s, "home.font.font-colour");
    assert_eq!(
        cell(&s, 1, 1).font.color.source,
        ColorSource::Rgb(Rgb { r: 255, g: 0, b: 0 })
    );
    s.grid.set_active(2, 1);
    click(&mut s, "home.font.fill-colour");
    assert_eq!(
        wb(&s).styles().resolve_color(&cell(&s, 2, 1).fill.fg),
        Rgb {
            r: 255,
            g: 255,
            b: 0
        }
    );
}

#[test]
fn border_menu_draws_and_removes_lines() {
    let mut s = state_with_workbook();
    s.grid.select_range((2, 2), (3, 3));
    click(&mut s, "fmt.border#all");
    let b = cell(&s, 2, 2).border;
    assert_eq!((b.left.style, b.bottom.style), (B::Thin, B::Thin));
    click(&mut s, "fmt.border#thick-outside");
    assert_eq!(cell(&s, 2, 2).border.top.style, B::Medium);
    click(&mut s, "fmt.border#none");
    let b = cell(&s, 2, 2).border;
    assert_eq!(
        (b.left.style, b.top.style, b.right.style),
        (B::None, B::None, B::None)
    );
    // The button applies the item chosen last.
    s.grid.set_active(5, 5);
    click(&mut s, "fmt.border#top-double-bottom");
    s.grid.set_active(7, 5);
    click(&mut s, "home.font.borders");
    assert_eq!(cell(&s, 7, 5).border.bottom.style, B::Double);
    assert_eq!(cell(&s, 7, 5).border.top.style, B::Thin);
    s.undo();
    assert_eq!(cell(&s, 7, 5).border.bottom.style, B::None);
    assert_eq!(
        host(&mut s).menu("home.font.borders").expect("menu").len(),
        13
    );
}

#[test]
fn number_group_applies_formats_and_styles() {
    let mut s = state_with_workbook();
    put(&mut s, 1, 1, 1234.5);
    click(&mut s, "fmt.numfmt#percentage");
    assert_eq!(category(&mut s).as_deref(), Some("Percentage"));
    click(&mut s, "fmt.numfmt#general");
    click(&mut s, "home.number.percent-style");
    assert_eq!(cell(&s, 1, 1).num_fmt_id, 9);
    click(&mut s, "home.number.comma-style");
    assert_eq!(cell(&s, 1, 1).num_fmt_id, 43);
    click(&mut s, "home.number.accounting-number-format");
    assert_eq!(cell(&s, 1, 1).num_fmt_id, 44);
    assert_eq!(category(&mut s).as_deref(), Some("Accounting"));
    click(&mut s, "fmt.numfmt#number");
    assert_eq!(code(&s, 1, 1), "0.00");
    click(&mut s, "home.number.increase-decimal");
    assert_eq!(code(&s, 1, 1), "0.000");
    click(&mut s, "home.number.decrease-decimal");
    click(&mut s, "home.number.decrease-decimal");
    assert_eq!(code(&s, 1, 1), "0.0");
    s.undo();
    assert_eq!(code(&s, 1, 1), "0.00");
    assert!(
        host(&mut s)
            .menu("home.number.number-format")
            .expect("menu")
            .len()
            >= 11
    );
    click(&mut s, "fmt.accounting#gbp");
    assert_eq!(category(&mut s).as_deref(), Some("Accounting"));
}

#[test]
fn cell_styles_gallery_lists_the_built_in_styles_and_applies_one() {
    let mut s = state_with_workbook();
    let names = wb(&s).styles().cell_style_names();
    for n in [
        "Normal",
        "Good",
        "Bad",
        "Neutral",
        "Heading 1",
        "Title",
        "Total",
        "Percent",
        "20% - Accent1",
    ] {
        assert!(names.iter().any(|x| x == n), "{n}");
    }
    let groups = format_ui::style_groups(&names);
    assert_eq!(
        groups.iter().map(|(_, v)| v.len()).sum::<usize>(),
        names.len()
    );
    assert_eq!(host(&mut s).gallery("home.styles.cell-styles").len(), 4);
    let gallery = host(&mut s).menu("home.styles.cell-styles").expect("menu");
    assert!(gallery.iter().any(|e| matches!(
        &e.kind,
        open365_ui_shell::EntryKind::Tiles { tiles, .. }
            if tiles.iter().any(|t| t.id == "home.styles.cell-styles#Good")
    )));
    click(&mut s, "home.styles.cell-styles#Good");
    assert_eq!(cell(&s, 1, 1).fill.pattern, FillPattern::Solid);
    let name = wb(&s).styles().cell_style_name(wb(&s).cell_xf(0, 1, 1));
    assert_eq!(name.as_deref(), Some("Good"));
    s.undo();
    assert_eq!(cell(&s, 1, 1).fill.pattern, FillPattern::None);
}

#[test]
fn clear_formats_and_whole_column_formatting() {
    let mut s = state_with_workbook();
    s.grid.select_columns(2, 3);
    click(&mut s, "home.font.bold");
    // Blank cells of the column take the format, far below any stored cell.
    assert!(cell(&s, 500, 2).font.bold && cell(&s, 9, 3).font.bold);
    assert!(!cell(&s, 9, 4).font.bold);
    click(&mut s, "fmt.clear-formats");
    assert!(!cell(&s, 500, 2).font.bold);
    s.undo();
    assert!(cell(&s, 500, 2).font.bold);
    let menu = host(&mut s).menu("home.editing.clear").expect("menu");
    assert_eq!(menu[0].id, "fmt.clear-formats");
}

#[test]
fn format_painter_single_click_applies_once() {
    let mut s = state_with_workbook();
    click(&mut s, "home.font.bold");
    click(&mut s, "home.font.italic");
    click(&mut s, "home.clipboard.format-painter");
    assert!(host(&mut s).state("home.clipboard.format-painter").checked);
    s.grid.set_active(3, 3);
    s.fmt.painter_pressed = true;
    let cmd = s
        .fmt
        .finish_paint(s.grid.sheet(), s.grid.selection())
        .expect("armed");
    s.apply(cmd);
    assert!(cell(&s, 3, 3).font.bold && cell(&s, 3, 3).font.italic);
    assert!(!s.fmt.painter_active());
    assert!(!host(&mut s).state("home.clipboard.format-painter").checked);
    s.undo();
    assert!(!cell(&s, 3, 3).font.bold);
}

#[test]
fn format_painter_double_click_locks_until_escape() {
    let mut s = state_with_workbook();
    click(&mut s, "home.font.bold");
    click(&mut s, "home.clipboard.format-painter");
    click(&mut s, "home.clipboard.format-painter");
    assert!(matches!(s.fmt.painter, format_ui::Painter::Locked(_)));
    for row in [3, 5] {
        s.grid.set_active(row, 3);
        let cmd = s.fmt.finish_paint(0, s.grid.selection()).expect("still on");
        s.apply(cmd);
        assert!(cell(&s, row, 3).font.bold);
        assert!(s.fmt.painter_active());
    }
    s.fmt.cancel_painter();
    assert!(s.fmt.finish_paint(0, s.grid.selection()).is_none());
}

#[test]
fn format_painter_click_after_the_window_through_the_host_switches_it_off() {
    let mut s = state_with_workbook();
    click(&mut s, "home.clipboard.format-painter");
    s.clock.frozen = Some(0.6);
    click(&mut s, "home.clipboard.format-painter");
    assert!(!s.fmt.painter_active());
}

#[test]
fn format_painter_second_slow_click_switches_it_off() {
    let mut ui = FormatUi::default();
    let s = state_with_workbook();
    let sel = s.sel();
    ui.run("home.clipboard.format-painter", s.workbook(), sel, 0.0);
    assert!(ui.painter_active());
    ui.run("home.clipboard.format-painter", s.workbook(), sel, 2.0);
    assert!(!ui.painter_active());
}

#[test]
fn keyboard_shortcuts_map_to_controls() {
    use format_ui::shortcut_id;
    let ctrl = Modifiers::COMMAND;
    let shifted = Modifiers::COMMAND | Modifiers::SHIFT;
    let id = |k, p, m| shortcut_id(k, p, m);
    assert_eq!(
        id(Key::B, Some(Key::B), ctrl).as_deref(),
        Some("home.font.bold")
    );
    assert_eq!(id(Key::I, None, ctrl).as_deref(), Some("home.font.italic"));
    assert_eq!(
        id(Key::U, None, ctrl).as_deref(),
        Some("home.font.underline")
    );
    assert_eq!(
        id(Key::Num5, None, ctrl).as_deref(),
        Some("fmt.strikethrough")
    );
    assert_eq!(id(Key::Num1, None, ctrl), None);
    assert_eq!(id(Key::B, None, Modifiers::NONE), None);
    for (physical, name) in [
        (Key::Backtick, "general"),
        (Key::Num1, "number"),
        (Key::Num2, "time"),
        (Key::Num3, "date"),
        (Key::Num4, "currency"),
        (Key::Num5, "percent"),
        (Key::Num6, "scientific"),
        (Key::Num7, "outline"),
        (Key::Minus, "no-outline"),
    ] {
        assert_eq!(
            id(Key::A, Some(physical), shifted),
            Some(format!("fmt.number-shortcut#{name}")),
        );
    }
}

#[test]
fn key_presses_run_the_shortcuts_and_leave_other_keys() {
    let ctx = egui::Context::default();
    let key = |key: Key, physical: Key, modifiers: Modifiers| egui::Event::Key {
        key,
        physical_key: Some(physical),
        pressed: true,
        repeat: false,
        modifiers,
    };
    let input = egui::RawInput {
        events: vec![
            key(Key::B, Key::B, Modifiers::COMMAND),
            key(
                Key::Exclamationmark,
                Key::Num1,
                Modifiers::COMMAND | Modifiers::SHIFT,
            ),
            key(Key::A, Key::A, Modifiers::NONE),
        ],
        ..egui::RawInput::default()
    };
    let mut ids = Vec::new();
    let mut left = 0;
    let mut out = ctx.run_ui(input, |ui| {
        ids = format_shortcuts(ui.ctx());
        left = ui.ctx().input(|i| i.events.len());
    });
    out.textures_delta.clear();
    assert_eq!(ids, ["home.font.bold", "fmt.number-shortcut#number"]);
    assert_eq!(left, 1);
}

#[test]
fn number_shortcuts_apply_the_excel_formats() {
    let mut s = state_with_workbook();
    s.grid.select_range((1, 1), (2, 2));
    for (name, id) in [
        ("number", 4),
        ("time", 18),
        ("date", 15),
        ("currency", 7),
        ("percent", 9),
        ("scientific", 11),
        ("general", 0),
    ] {
        click(&mut s, &format!("fmt.number-shortcut#{name}"));
        assert_eq!(cell(&s, 2, 2).num_fmt_id, id, "{name}");
    }
    click(&mut s, "fmt.number-shortcut#outline");
    assert_eq!(cell(&s, 1, 1).border.left.style, B::Thin);
    assert_eq!(cell(&s, 2, 2).border.bottom.style, B::Thin);
    click(&mut s, "fmt.number-shortcut#no-outline");
    assert_eq!(cell(&s, 1, 1).border.left.style, B::None);
}

fn put_text(s: &mut State, row: u32, col: u32, t: &str) {
    s.apply(Command::XlsxSetCellValue {
        sheet: 0,
        row,
        col,
        value: open365_commands::CellEntry::Text(t.into()),
    });
}

fn merged(s: &State) -> Vec<(u32, u32, u32, u32)> {
    wb(s)
        .sheet(0)
        .map(open365_xlsx::Worksheet::merged_cells)
        .unwrap_or_default()
}

#[test]
fn merge_menu_lists_the_four_items_and_the_button_toggles() {
    let mut s = state_with_workbook();
    let ids: Vec<String> = host(&mut s)
        .menu("home.alignment.merge-centre")
        .expect("menu")
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(
        ids,
        [
            "fmt.merge#center",
            "fmt.merge#across",
            "fmt.merge#cells",
            "fmt.merge#unmerge"
        ]
    );
    assert!(!host(&mut s).state("fmt.merge#unmerge").enabled);

    // Empty cells merge without a question and the button centres them.
    s.grid.select_range((2, 2), (2, 4));
    click(&mut s, "home.alignment.merge-centre");
    assert!(s.prompt.is_none());
    assert_eq!(merged(&s), vec![(2, 2, 2, 4)]);
    assert_eq!(cell(&s, 2, 2).alignment.horizontal, H::Center);
    assert!(host(&mut s).state("home.alignment.merge-centre").checked);
    assert!(host(&mut s).state("fmt.merge#unmerge").enabled);

    // Pressed again it splits the merge and keeps the centring.
    click(&mut s, "home.alignment.merge-centre");
    assert!(merged(&s).is_empty());
    assert_eq!(cell(&s, 2, 2).alignment.horizontal, H::Center);

    // Merge Across and Merge Cells from the menu; one undo each.
    s.grid.select_range((5, 1), (6, 3));
    click(&mut s, "fmt.merge#across");
    assert_eq!(merged(&s), vec![(5, 1, 5, 3), (6, 1, 6, 3)]);
    click(&mut s, "fmt.merge#cells");
    assert_eq!(merged(&s), vec![(5, 1, 6, 3)]);
    click(&mut s, "fmt.merge#unmerge");
    assert!(merged(&s).is_empty());
    s.undo();
    assert_eq!(merged(&s), vec![(5, 1, 6, 3)]);
}

#[test]
fn merging_values_asks_first_and_keeps_only_the_top_left() {
    let mut s = state_with_workbook();
    put_text(&mut s, 3, 1, "keep");
    put_text(&mut s, 3, 2, "lose");
    s.grid.select_range((3, 1), (3, 2));
    click(&mut s, "fmt.merge#cells");
    let (text, _) = s.prompt.as_ref().expect("prompt");
    assert_eq!(
        *text,
        "Merging cells only keeps the upper-left value and discards other values."
    );
    assert!(merged(&s).is_empty(), "nothing happens before OK");
    // OK runs the command the prompt holds.
    let (_, command) = s.prompt.take().expect("prompt");
    s.apply(command);
    assert_eq!(merged(&s), vec![(3, 1, 3, 2)]);
    assert!(
        wb(&s)
            .cell(0, 3, 2)
            .is_none_or(|c| c.value.as_text().is_none())
    );
    assert_eq!(
        wb(&s).cell(0, 3, 1).and_then(|c| c.value.as_text()),
        Some("keep")
    );
    // A selection holding one value needs no question.
    put_text(&mut s, 8, 1, "only");
    s.grid.select_range((8, 1), (8, 2));
    click(&mut s, "fmt.merge#cells");
    assert!(s.prompt.is_none());
}

/// The Cell Styles gallery tiles show each style's real fill, text colour and border, as Excel's
/// gallery does (`fixtures/reference/ui/excel-popups.json`: Neutral ffeb9c on 9c5700, Heading 1 with a
/// 3 u bottom rule, Output with a 3f3f3f frame).
#[test]
fn cell_style_tiles_show_the_style_look() {
    let mut s = state_with_workbook();
    let entries = host(&mut s)
        .menu("home.styles.cell-styles")
        .expect("gallery menu");
    let tile = |name: &str| {
        entries
            .iter()
            .find_map(|e| match &e.kind {
                open365_ui_shell::EntryKind::Tiles { tiles, .. } => {
                    tiles.iter().find(|t| t.label == name).cloned()
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("tile {name}"))
    };
    let neutral = tile("Neutral");
    assert_eq!(neutral.fill, Some([0xff, 0xeb, 0x9c]));
    assert_eq!(neutral.ink, Some([0x9c, 0x57, 0x00]));
    let normal = tile("Normal");
    assert_eq!(normal.fill, Some([255; 3]));
    assert!(normal.borders.iter().all(Option::is_none));
    let heading = tile("Heading 1");
    assert_eq!(heading.borders[3].map(|b| b.1), Some(3));
    assert!(heading.text_pct > 100);
    let output = tile("Output");
    assert!(output.borders.iter().all(Option::is_some));
    assert_eq!(output.borders[0].map(|b| b.0), Some([0x3f, 0x3f, 0x3f]));
}
