//! Gridlines and headings from the View tab, Page Break Preview and the Page Layout view.

#![allow(clippy::unwrap_used)]

mod grid_draw_common;
use grid_draw_common::*;
use open365_preview::doc::sheetchrome;
use open365_preview::sheet_ext::view::ViewMode;

const BREAK_BLUE: [u8; 3] = [0x00, 0x4e, 0xc8];
const GRID_LINE: [u8; 3] = [0xe0, 0xe0, 0xe0];

fn filled() -> open365_preview::Preview {
    let mut p = sheets();
    for r in 1..=6u32 {
        for c in 1..=3u32 {
            p.shell
                .sheet_mut()
                .enter(r, c, &(r * c).to_string())
                .unwrap();
        }
    }
    p
}

#[test]
fn gridlines_off_removes_the_grid_lines() {
    let mut p = sheets();
    let on = count(&p.draw_list(), GRID_LINE);
    assert!(on > 0, "the light theme draws grid lines in {GRID_LINE:?}");
    let sheet = p.shell.sheet().sheet;
    p.shell.sheet_mut().ext_view.set_gridlines(sheet, false);
    let off = count(&p.draw_list(), GRID_LINE);
    assert!(off < on, "{off} < {on}");
}

#[test]
fn headings_off_moves_the_cells_to_the_edge() {
    let mut p = sheets();
    let with = p.shell.cell_centre(1, 1);
    let sheet = p.shell.sheet().sheet;
    p.shell.sheet_mut().ext_view.set_headings(sheet, false);
    let without = p.shell.cell_centre(1, 1);
    assert!(
        without.x < with.x && without.y < with.y,
        "{without:?} < {with:?}"
    );
    let _ = p.draw_list();
}

#[test]
fn page_break_preview_draws_blue_page_lines_and_the_page_number() {
    let mut p = filled();
    assert_eq!(count(&p.draw_list(), BREAK_BLUE), 0);
    p.shell
        .sheet_mut()
        .set_view_mode(ViewMode::PageBreakPreview);
    assert!(
        count(&p.draw_list(), BREAK_BLUE) > 0,
        "the print area is framed in blue"
    );
    let pages = sheetchrome::pages(p.shell.sheet());
    assert_eq!(pages.print, (1, 1, 6, 3));
    assert_eq!((pages.col_starts.len(), pages.row_starts.len()), (1, 1));
    assert_eq!(pages.number(0, 0), 1);
}

#[test]
fn a_manual_break_splits_the_pages_and_numbers_them_down_then_over() {
    let mut p = filled();
    p.shell.sheet_mut().select_cell((4, 2));
    p.shell.sheet_mut().insert_page_break();
    let pages = sheetchrome::pages(p.shell.sheet());
    assert_eq!(pages.row_starts, vec![1, 4]);
    assert_eq!(pages.col_starts, vec![1, 2]);
    assert_eq!(pages.number(0, 1), 2);
    assert_eq!(pages.number(1, 0), 3);
    p.shell
        .sheet_mut()
        .set_view_mode(ViewMode::PageBreakPreview);
    let _ = p.draw_list();
}

#[test]
fn page_layout_view_shows_page_tiles_and_a_click_picks_a_cell_on_a_tile() {
    let mut p = filled();
    p.shell.sheet_mut().set_view_mode(ViewMode::PageLayout);
    let _ = p.draw_list();
    let area = open365_preview::layout::PxRect {
        x: 0,
        y: 0,
        w: 1600,
        h: 1000,
    };
    let tiles = sheetchrome::layout_tiles(p.shell.sheet(), area, p.scale());
    assert!(!tiles.is_empty());
    let t = tiles[0];
    assert!(
        t.content.x0 > t.page.x0 && t.content.y0 > t.page.y0,
        "the margins"
    );
    let (l, tp, r, b) = p.shell.cell_dev_rect(3, 2);
    p.click((l + r) / 2.0, (tp + b) / 2.0);
    assert_eq!(p.shell.sheet().active, (3, 2));
}
