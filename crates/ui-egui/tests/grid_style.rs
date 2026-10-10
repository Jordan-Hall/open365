//! Cell text behaviour in the grid: General alignment, overflow, `####`, wrapping, checked
//! against Excel's `Range.Text` (`fixtures/reference/xlsx/grid-styles/styles.json`) and through
//! the cells the grid reports having drawn, with the AccessKit nodes still published.

use std::path::PathBuf;

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use open365_ui_egui::{App, PaintedCell};
use open365_xlsx::HorizontalAlignment;

fn harness(scale: f32) -> Harness<'static, App> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/xlsx/grid-styles.xlsx");
    let mut app = App::new(Some(path));
    app.grid.select_sheet(2);
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 2400.0))
        .with_pixels_per_point(scale)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.run();
    h
}

fn painted(h: &Harness<'_, App>, row: u32, col: u32) -> PaintedCell {
    h.state()
        .grid
        .painted()
        .into_iter()
        .find(|p| p.row == row && p.col == col)
        .unwrap_or_else(|| panic!("cell {row},{col} not drawn"))
}

#[test]
fn general_alignment_follows_value_type() {
    let h = harness(1.25);
    assert_eq!(painted(&h, 9, 7).horizontal, HorizontalAlignment::Left);
    assert_eq!(painted(&h, 10, 7).horizontal, HorizontalAlignment::Right);
    assert_eq!(painted(&h, 11, 7).horizontal, HorizontalAlignment::Center);
    assert_eq!(painted(&h, 12, 7).horizontal, HorizontalAlignment::Center);
    let p = painted(&h, 10, 7);
    assert!(p.bounds.right() <= p.cell.right());
    let t = painted(&h, 9, 7);
    assert!(t.bounds.left() >= t.cell.left());
}

#[test]
fn text_overflows_into_empty_neighbours_and_stops_at_filled_ones() {
    let h = harness(1.25);
    let free = painted(&h, 1, 1);
    assert!(free.clip.right() > free.cell.right(), "A1 spills right");
    let blocked = painted(&h, 2, 1);
    assert_eq!(
        blocked.clip.right(),
        blocked.cell.right(),
        "B2 is not empty"
    );
    let right = painted(&h, 3, 4);
    assert!(
        right.clip.left() < right.cell.left(),
        "right-aligned spills left"
    );
    let centre = painted(&h, 4, 4);
    assert!(centre.clip.left() < centre.cell.left() && centre.clip.right() > centre.cell.right());
    let wrapped = painted(&h, 22, 7);
    assert!(wrapped.lines > 1);
    assert_eq!(wrapped.clip, wrapped.cell, "wrapped text does not overflow");
}

#[test]
fn displayed_text_equals_excel_range_text() {
    let h = harness(1.25); // the oracle ran at 120 dpi
    let json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/reference/xlsx/grid-styles/styles.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let cells = json["sheets"][2]["cells"].as_object().unwrap();
    let (mut ok, mut total) = (0, 0);
    let mut bad = Vec::new();
    for (addr, info) in cells {
        let want = info["text"].as_str().unwrap();
        let (row, col) = open365_xlsx::parse_cell_ref(addr).unwrap();
        // Wrapped text, rotated text and formatted dates differ in representation.
        if want.is_empty() || want.contains('\n') || info["align"]["wrap"] == true {
            continue;
        }
        let Some(p) = h
            .state()
            .grid
            .painted()
            .into_iter()
            .find(|p| p.row == row && p.col == col)
        else {
            continue;
        };
        total += 1;
        if p.text == want {
            ok += 1;
        } else {
            bad.push(format!("{addr}: ours {:?} Excel {want:?}", p.text));
        }
    }
    eprintln!("text parity: {ok}/{total}");
    // Known, explained differences: E27 is stacked text (the grid joins one character per
    // line, Range.Text is the plain string); F50 is the en-US short date against the en-GB
    // oracle.
    bad.retain(|b| !["E27:", "F50:"].iter().any(|k| b.starts_with(k)));
    assert!(
        bad.is_empty(),
        "text parity {ok}/{total}:\n{}",
        bad.join("\n")
    );
}

#[test]
fn hashes_and_accesskit_values_differ() {
    let h = harness(1.0);
    let p = painted(&h, 30, 2);
    assert!(p.hashes && p.text.chars().all(|c| c == '#'));
    // The screen reader still gets the number.
    let node = h
        .query_all_by_role(Role::Cell)
        .find(|c| {
            let d = c.accesskit_node().data().clone();
            d.row_index() == Some(29) && d.column_index() == Some(1)
        })
        .expect("B30 published");
    assert_eq!(node.accesskit_node().data().value(), Some("1234567"));
}
