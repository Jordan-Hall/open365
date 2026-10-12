//! Sheets Home tab: every wired control runs through `Shell::run_command` and changes the sheet.

#![allow(clippy::unwrap_used)]

use open365_preview::ribbon::Handled;
use open365_preview::sheet::{MemoryClipboard, SheetModel};
use open365_preview::{AppKind, Preview};
use open365_xlsx::{HorizontalAlignment, Underline, VerticalAlignment};

fn rig() -> Preview {
    let mut p = Preview::new(AppKind::Sheets, true, 1600, 1000, 1.25);
    p.shell
        .sheet_mut()
        .set_clipboard(MemoryClipboard::default());
    p
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn sheet(p: &mut Preview) -> &mut SheetModel {
    p.shell.sheet_mut()
}

fn select(p: &mut Preview, a: (u32, u32), b: (u32, u32)) {
    let s = sheet(p);
    s.select_cell(a);
    s.extend_to(b);
}

#[test]
fn copy_cut_and_paste_move_cells() {
    let mut p = rig();
    sheet(&mut p).enter(1, 1, "hello").unwrap();
    sheet(&mut p).enter(1, 2, "=2*3").unwrap();
    select(&mut p, (1, 1), (1, 2));
    run(&mut p, "home.clipboard.copy");
    sheet(&mut p).select_cell((4, 1));
    run(&mut p, "home.clipboard.paste");
    assert_eq!(sheet(&mut p).source(4, 1), "hello");
    assert_eq!(sheet(&mut p).source(4, 2), "=2*3");
    // Cut empties the source on paste; paste over data does not ask.
    select(&mut p, (4, 1), (4, 2));
    run(&mut p, "home.clipboard.cut");
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "home.clipboard.paste");
    assert_eq!(sheet(&mut p).source(4, 1), "");
    assert_eq!(sheet(&mut p).source(1, 1), "hello");
    // Paste Values turns the formula into its result.
    select(&mut p, (1, 1), (1, 2));
    run(&mut p, "home.clipboard.copy");
    sheet(&mut p).select_cell((6, 1));
    run(&mut p, "home.clipboard.paste.values");
    assert_eq!(sheet(&mut p).source(6, 2), "6");
}

#[test]
fn format_painter_copies_formats_on_the_second_click() {
    let mut p = rig();
    sheet(&mut p).enter(1, 1, "a").unwrap();
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "home.font.bold");
    run(&mut p, "home.clipboard.format-painter");
    sheet(&mut p).select_cell((3, 3));
    run(&mut p, "home.clipboard.format-painter");
    assert!(sheet(&mut p).home_cell_format(3, 3).unwrap().font.bold);
}

#[test]
fn font_controls_change_the_selection_and_undo() {
    let mut p = rig();
    select(&mut p, (1, 1), (2, 2));
    run(&mut p, "home.font.bold");
    assert!(sheet(&mut p).home_cell_format(2, 2).unwrap().font.bold);
    run(&mut p, "home.font.bold");
    assert!(
        !sheet(&mut p).home_cell_format(1, 1).unwrap().font.bold,
        "second press clears"
    );
    run(&mut p, "home.font.italic");
    assert!(sheet(&mut p).home_cell_format(1, 2).unwrap().font.italic);
    run(&mut p, "home.font.underline");
    assert_eq!(
        sheet(&mut p).home_cell_format(1, 1).unwrap().font.underline,
        Underline::Single
    );
    run(&mut p, "home.font.underline.double");
    assert_eq!(
        sheet(&mut p).home_cell_format(1, 1).unwrap().font.underline,
        Underline::Double
    );
    run(&mut p, "home.font.font=Arial");
    assert_eq!(
        sheet(&mut p).home_cell_format(2, 1).unwrap().font.name,
        "Arial"
    );
    run(&mut p, "home.font.font-size=18");
    assert_eq!(
        sheet(&mut p).home_cell_format(2, 1).unwrap().font.size,
        18.0
    );
    run(&mut p, "home.font.increase-font-size");
    assert_eq!(
        sheet(&mut p).home_cell_format(1, 1).unwrap().font.size,
        20.0
    );
    run(&mut p, "home.font.decrease-font-size");
    run(&mut p, "home.font.decrease-font-size");
    assert_eq!(
        sheet(&mut p).home_cell_format(1, 1).unwrap().font.size,
        16.0
    );
    assert!(sheet(&mut p).undo());
    assert_eq!(
        sheet(&mut p).home_cell_format(1, 1).unwrap().font.size,
        18.0
    );
}

#[test]
fn fill_font_colour_and_borders() {
    let mut p = rig();
    select(&mut p, (1, 1), (1, 1));
    run(&mut p, "home.font.fill-colour");
    let f = sheet(&mut p).home_cell_format(1, 1).unwrap();
    assert!(
        matches!(f.fill.pattern, open365_xlsx::FillPattern::Solid),
        "{:?}",
        f.fill
    );
    run(&mut p, "home.font.fill-colour.auto");
    let f = sheet(&mut p).home_cell_format(1, 1).unwrap();
    assert!(!matches!(f.fill.pattern, open365_xlsx::FillPattern::Solid));
    run(&mut p, "home.font.font-colour.rgb-00B050");
    let dbg = format!(
        "{:?}",
        sheet(&mut p).home_cell_format(1, 1).unwrap().font.color
    );
    assert!(dbg.contains("g: 176"), "{dbg}");
    run(&mut p, "home.font.borders.all");
    let b = sheet(&mut p).home_cell_format(1, 1).unwrap().border;
    assert_eq!(b.left.style, open365_xlsx::BorderStyle::Thin);
    run(&mut p, "home.font.borders.none");
    let b = sheet(&mut p).home_cell_format(1, 1).unwrap().border;
    assert_eq!(b.left.style, open365_xlsx::BorderStyle::None);
    // The button applies the last choice.
    run(&mut p, "home.font.borders.top");
    run(&mut p, "home.font.borders.none");
    run(&mut p, "home.font.borders");
    assert_eq!(
        sheet(&mut p)
            .home_cell_format(1, 1)
            .unwrap()
            .border
            .top
            .style,
        open365_xlsx::BorderStyle::None
    );
}

#[test]
fn alignment_buttons_toggle_and_merge_centre_merges() {
    let mut p = rig();
    sheet(&mut p).enter(1, 1, "x").unwrap();
    select(&mut p, (1, 1), (1, 1));
    run(&mut p, "home.alignment.align-left");
    assert_eq!(
        sheet(&mut p)
            .home_cell_format(1, 1)
            .unwrap()
            .alignment
            .horizontal,
        HorizontalAlignment::Left
    );
    run(&mut p, "home.alignment.align-left");
    assert_eq!(
        sheet(&mut p)
            .home_cell_format(1, 1)
            .unwrap()
            .alignment
            .horizontal,
        HorizontalAlignment::General,
        "pressing it again undoes it"
    );
    run(&mut p, "home.alignment.centre");
    run(&mut p, "home.alignment.align-right");
    assert_eq!(
        sheet(&mut p)
            .home_cell_format(1, 1)
            .unwrap()
            .alignment
            .horizontal,
        HorizontalAlignment::Right
    );
    run(&mut p, "home.alignment.top-align");
    assert_eq!(
        sheet(&mut p)
            .home_cell_format(1, 1)
            .unwrap()
            .alignment
            .vertical,
        VerticalAlignment::Top
    );
    run(&mut p, "home.alignment.middle-align");
    run(&mut p, "home.alignment.bottom-align");
    assert_eq!(
        sheet(&mut p)
            .home_cell_format(1, 1)
            .unwrap()
            .alignment
            .vertical,
        VerticalAlignment::Bottom
    );
    run(&mut p, "home.alignment.increase-indent");
    run(&mut p, "home.alignment.increase-indent");
    run(&mut p, "home.alignment.decrease-indent");
    assert_eq!(
        sheet(&mut p)
            .home_cell_format(1, 1)
            .unwrap()
            .alignment
            .indent,
        1
    );
    run(&mut p, "home.alignment.wrap-text");
    assert!(
        sheet(&mut p)
            .home_cell_format(1, 1)
            .unwrap()
            .alignment
            .wrap_text
    );
    run(&mut p, "home.alignment.orientation.rotate-up");
    assert_eq!(
        sheet(&mut p)
            .home_cell_format(1, 1)
            .unwrap()
            .alignment
            .text_rotation,
        90
    );

    select(&mut p, (3, 1), (3, 3));
    run(&mut p, "home.alignment.merge-centre");
    assert_eq!(sheet(&mut p).home_merged(), vec![(3, 1, 3, 3)]);
    assert_eq!(
        sheet(&mut p)
            .home_cell_format(3, 1)
            .unwrap()
            .alignment
            .horizontal,
        HorizontalAlignment::Center
    );
    run(&mut p, "home.alignment.merge-centre");
    assert!(sheet(&mut p).home_merged().is_empty(), "the button toggles");
    select(&mut p, (5, 1), (6, 2));
    run(&mut p, "home.alignment.merge-centre.across");
    assert_eq!(sheet(&mut p).home_merged().len(), 2);
    run(&mut p, "home.alignment.merge-centre.unmerge");
    assert!(sheet(&mut p).home_merged().is_empty());
}

#[test]
fn number_formats_follow_the_locale() {
    let mut p = rig();
    sheet(&mut p).enter(1, 1, "1234.5").unwrap();
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "home.number.number-format=Number");
    assert_eq!(sheet(&mut p).display(1, 1), "1234.50");
    run(&mut p, "home.number.number-format=Currency");
    assert_eq!(sheet(&mut p).display(1, 1), "$1,234.50");
    run(&mut p, "home.number.accounting-number-format");
    assert!(sheet(&mut p).display(1, 1).contains("1,234.50"));
    run(&mut p, "home.number.number-format=Percentage");
    assert_eq!(sheet(&mut p).display(1, 1), "123450.00%");
    run(&mut p, "home.number.percent-style");
    assert_eq!(sheet(&mut p).display(1, 1), "123450%");
    run(&mut p, "home.number.number-format=General");
    run(&mut p, "home.number.comma-style");
    assert_eq!(sheet(&mut p).display(1, 1).trim(), "1,234.50");
    run(&mut p, "home.number.increase-decimal");
    assert_eq!(sheet(&mut p).display(1, 1).trim(), "1,234.500");
    run(&mut p, "home.number.decrease-decimal");
    run(&mut p, "home.number.decrease-decimal");
    assert_eq!(sheet(&mut p).display(1, 1).trim(), "1,234.5");
    run(&mut p, "home.number.number-format=Short Date");
    assert_eq!(sheet(&mut p).display(1, 1), "5/18/1903");
    run(&mut p, "home.number.number-format=Text");
    assert_eq!(sheet(&mut p).home_format_code(1, 1), "@");
}

#[test]
fn cell_styles_apply_by_name() {
    let mut p = rig();
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "home.styles.cell-styles.good");
    let f = sheet(&mut p).home_cell_format(1, 1).unwrap();
    assert!(matches!(f.fill.pattern, open365_xlsx::FillPattern::Solid));
    run(&mut p, "home.styles.cell-styles.heading-1");
    assert!(sheet(&mut p).home_cell_format(1, 1).unwrap().font.bold);
    assert!(sheet(&mut p).undo());
}

#[test]
fn insert_delete_and_sizes() {
    let mut p = rig();
    sheet(&mut p).enter(2, 1, "keep").unwrap();
    sheet(&mut p).select_rows(2, 3, false);
    run(&mut p, "home.cells.insert");
    assert_eq!(
        sheet(&mut p).source(4, 1),
        "keep",
        "two rows were inserted above"
    );
    run(&mut p, "home.cells.delete");
    assert_eq!(sheet(&mut p).source(2, 1), "keep");
    sheet(&mut p).select_cols(1, 1, false);
    run(&mut p, "home.cells.insert");
    assert_eq!(sheet(&mut p).source(2, 2), "keep");
    run(&mut p, "home.cells.delete.columns");
    assert_eq!(sheet(&mut p).source(2, 1), "keep");

    sheet(&mut p).select_rows(5, 6, false);
    run(&mut p, "home.cells.format.row-height=30");
    assert_eq!(sheet(&mut p).home_row_state(6).0, Some(30.0));
    run(&mut p, "home.cells.format.hide-rows");
    assert!(sheet(&mut p).home_row_state(5).1);
    run(&mut p, "home.cells.format.unhide-rows");
    assert!(!sheet(&mut p).home_row_state(5).1);
    sheet(&mut p).select_cell((1, 3));
    run(&mut p, "home.cells.format.column-width=20");
    let (w, _) = sheet(&mut p).home_col_state(3).unwrap();
    assert!((20.0..21.0).contains(&w), "{w}");
    run(&mut p, "home.cells.format.hide-columns");
    assert!(sheet(&mut p).home_col_state(3).unwrap().1);
}

#[test]
fn auto_sum_menu_and_fill() {
    let mut p = rig();
    for (r, v) in [(1, "2"), (2, "4"), (3, "9")] {
        sheet(&mut p).enter(r, 1, v).unwrap();
    }
    sheet(&mut p).select_cell((4, 1));
    run(&mut p, "home.editing.auto-sum");
    assert_eq!(sheet(&mut p).source(4, 1), "=SUM(A1:A3)");
    assert_eq!(sheet(&mut p).display(4, 1), "15");
    sheet(&mut p).select_cell((5, 1));
    run(&mut p, "home.editing.auto-sum.max");
    assert_eq!(sheet(&mut p).source(5, 1), "=MAX(A1:A4)");

    sheet(&mut p).enter(1, 3, "7").unwrap();
    select(&mut p, (1, 3), (3, 3));
    run(&mut p, "home.editing.fill.down");
    assert_eq!(sheet(&mut p).source(3, 3), "7");
    sheet(&mut p).enter(8, 1, "=B8+1").unwrap();
    sheet(&mut p).enter(10, 1, "z").unwrap();
    select(&mut p, (8, 1), (10, 1));
    run(&mut p, "home.editing.fill.up");
    assert_eq!(sheet(&mut p).source(8, 1), "z");
    sheet(&mut p).enter(12, 1, "r").unwrap();
    select(&mut p, (12, 1), (12, 3));
    run(&mut p, "home.editing.fill.right");
    assert_eq!(sheet(&mut p).source(12, 3), "r");
    sheet(&mut p).enter(14, 3, "l").unwrap();
    select(&mut p, (14, 1), (14, 3));
    run(&mut p, "home.editing.fill.left");
    assert_eq!(sheet(&mut p).source(14, 1), "l");
}

#[test]
fn clear_menu_clears_contents_formats_or_all() {
    let mut p = rig();
    sheet(&mut p).enter(1, 1, "5").unwrap();
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "home.font.bold");
    run(&mut p, "home.editing.clear.formats");
    assert!(!sheet(&mut p).home_cell_format(1, 1).unwrap().font.bold);
    assert_eq!(sheet(&mut p).source(1, 1), "5");
    run(&mut p, "home.font.bold");
    run(&mut p, "home.editing.clear.contents");
    assert_eq!(sheet(&mut p).source(1, 1), "");
    assert!(sheet(&mut p).home_cell_format(1, 1).unwrap().font.bold);
    sheet(&mut p).enter(1, 1, "5").unwrap();
    run(&mut p, "home.editing.clear.all");
    assert_eq!(sheet(&mut p).source(1, 1), "");
    assert!(!sheet(&mut p).home_cell_format(1, 1).unwrap().font.bold);
    assert!(sheet(&mut p).undo(), "Clear All is one undo step");
    assert_eq!(sheet(&mut p).source(1, 1), "5");
    assert!(sheet(&mut p).home_cell_format(1, 1).unwrap().font.bold);
}

#[test]
fn sort_and_filter() {
    let mut p = rig();
    for (r, (n, v)) in [("name", "n"), ("c", "3"), ("a", "1"), ("b", "2")]
        .into_iter()
        .enumerate()
    {
        sheet(&mut p).enter(r as u32 + 1, 1, n).unwrap();
        sheet(&mut p).enter(r as u32 + 1, 2, v).unwrap();
    }
    // A lone cell sorts its region by its column; the header row stays.
    sheet(&mut p).select_cell((2, 1));
    run(&mut p, "home.editing.sort-filter.az");
    let col: Vec<String> = (1..=4).map(|r| sheet(&mut p).source(r, 1)).collect();
    assert_eq!(col, ["name", "a", "b", "c"]);
    assert_eq!(sheet(&mut p).source(2, 2), "1", "the row moved as a whole");
    run(&mut p, "home.editing.sort-filter.za");
    assert_eq!(sheet(&mut p).source(2, 1), "c");
    assert!(sheet(&mut p).undo());
    assert_eq!(sheet(&mut p).source(2, 1), "a");
    run(&mut p, "home.editing.sort-filter.custom=B:desc");
    assert_eq!(sheet(&mut p).source(2, 1), "c");

    assert!(!sheet(&mut p).home_has_filter());
    run(&mut p, "home.editing.sort-filter.filter");
    assert!(sheet(&mut p).home_has_filter());
    run(&mut p, "home.editing.sort-filter.clear");
    run(&mut p, "home.editing.sort-filter.reapply");
    run(&mut p, "home.editing.sort-filter.filter");
    assert!(!sheet(&mut p).home_has_filter());
}

#[test]
fn find_and_go_to_select_cells() {
    let mut p = rig();
    sheet(&mut p).enter(3, 2, "needle").unwrap();
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "home.editing.find-select.find=needle");
    assert_eq!(sheet(&mut p).active, (3, 2));
    run(&mut p, "home.editing.find-select.goto=B2:D5");
    assert_eq!(sheet(&mut p).range(), (2, 2, 5, 4));
}

#[test]
fn conditional_formatting_and_tables_are_not_wired() {
    let mut p = rig();
    for id in [
        "home.styles.conditional-formatting",
        "home.styles.format-as-table",
    ] {
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::No, "{id}");
    }
}

#[test]
fn every_claimed_home_id_answers_yes_alone_and_with_a_value() {
    let home = open365_preview::ribbon::modules(AppKind::Sheets)
        .iter()
        .find(|m| m.prefix == "home")
        .unwrap();
    let mut p = rig();
    for id in home.ids {
        for form in [(*id).to_owned(), format!("{id}=12")] {
            eprintln!("running {form}");
            assert_eq!(
                p.shell.run_command(&mut p.ui, &form),
                Handled::Yes,
                "{form}"
            );
        }
    }
}
