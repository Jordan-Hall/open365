//! AutoFilter buttons: drawn on the header row; a click opens the menu (sort, text or number
//! filters, the values checklist); unticking a value filters the column.

#![allow(clippy::unwrap_used)]

mod grid_draw_common;
use grid_draw_common::*;
use open365_ui_core::Key;

fn filtered() -> open365_preview::Preview {
    let mut p = sheets();
    {
        let m = p.shell.sheet_mut();
        let rows = [
            ("Name", "Qty"),
            ("pear", "3"),
            ("apple", "1"),
            ("plum", "2"),
        ];
        for (r, (name, n)) in rows.iter().enumerate() {
            m.enter(r as u32 + 1, 1, name).unwrap();
            m.enter(r as u32 + 1, 2, n).unwrap();
        }
        m.select_cell((1, 1));
        m.data_toggle_filter().unwrap();
    }
    p
}

#[test]
fn the_buttons_sit_in_the_header_cells_and_a_click_opens_the_menu() {
    let mut p = filtered();
    let marks = p.shell.sheet().grid_marks();
    assert_eq!(marks.filter.as_ref().map(|f| f.0), Some((1, 1, 4, 2)));
    let _ = p.draw_list();
    let at = in_cell_corner(&p, 1, 1, 6.0, 6.0);
    p.click(at.x, at.y);
    assert!(menu_open(&p), "the filter menu opens");
    let labels = menu_labels(&p);
    for l in [
        "Sort A to Z",
        "Sort Z to A",
        "Text Filters",
        "(Select All)",
        "apple",
        "pear",
        "plum",
    ] {
        assert!(labels.iter().any(|x| x == l), "{l} in {labels:?}");
    }
    assert!(labels.iter().any(|l| l.starts_with("Clear Filter From")));
}

#[test]
fn a_number_column_offers_number_filters_and_sorts_smallest_to_largest() {
    let mut p = filtered();
    let at = in_cell_corner(&p, 1, 2, 6.0, 6.0);
    p.click(at.x, at.y);
    let labels = menu_labels(&p);
    assert!(labels.contains(&"Number Filters".to_owned()), "{labels:?}");
    assert!(labels.contains(&"Sort Smallest to Largest".to_owned()));
}

#[test]
fn unticking_a_value_filters_the_column_and_the_menu_stays_open() {
    let mut p = filtered();
    let at = in_cell_corner(&p, 1, 1, 6.0, 6.0);
    p.click(at.x, at.y);
    pick(&mut p, "pear");
    assert_eq!(p.shell.sheet().data_filtered_columns(), vec![1]);
    let items = p.shell.sheet().data_filter_items(1);
    assert!(items.iter().any(|(v, on)| v == "pear" && !on), "{items:?}");
    assert!(menu_open(&p), "the checklist reopens, ticks updated");
    let _ = p.draw_list();
}

#[test]
fn sort_a_to_z_sorts_the_column_and_select_all_clears_the_filter() {
    let mut p = filtered();
    let at = in_cell_corner(&p, 1, 1, 6.0, 6.0);
    p.click(at.x, at.y);
    pick(&mut p, "Sort A to Z");
    assert_eq!(p.shell.sheet().display(2, 1), "apple");
    assert_eq!(p.shell.sheet().display(4, 1), "plum");
    assert_eq!(p.shell.sheet().active, (1, 1), "the selection comes back");
    p.click(at.x, at.y);
    pick(&mut p, "pear");
    assert_eq!(p.shell.sheet().data_filtered_columns(), vec![1]);
    p.key(Key::Escape);
    p.click(at.x, at.y);
    pick(&mut p, "(Select All)");
    assert!(p.shell.sheet().data_filtered_columns().is_empty());
}

#[test]
fn a_file_with_an_autofilter_draws_without_panicking() {
    let mut p = open_fixture("sortfilter-filter.xlsx");
    let marks = p.shell.sheet().grid_marks();
    if let Some((range, _)) = &marks.filter {
        assert!(range.2 >= range.0);
    }
    let _ = p.draw_list();
}
