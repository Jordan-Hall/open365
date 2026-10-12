//! Floating objects on the grid: the objects of `drawing-basic` (Excel-made) are painted at the
//! position and size their anchors give, in the same pixel geometry as the cells.

use std::path::PathBuf;

use egui::vec2;
use egui_kittest::Harness;
use open365_ui_egui::App;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn harness(name: &str) -> Harness<'static, App> {
    let app = App::new(Some(root().join(format!("fixtures/xlsx/{name}.xlsx"))));
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 1100.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.run();
    h
}

#[test]
fn objects_are_painted_where_their_anchors_put_them() {
    let h = harness("drawing-basic");
    let objects = h.state().grid.drawn_objects();
    let names: Vec<&str> = objects.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["Pic 1", "Rect 1", "Chart 1", "Rect rot"]);
    let cell = |row, col| {
        h.state()
            .grid
            .painted()
            .into_iter()
            .find(|p| p.row == row && p.col == col)
            .expect("cell painted")
            .cell
    };
    // Pic 1: 90 x 60 pt (120 x 80 egui points), 7.5 pt (10) right and 4.25 pt (5.67 -> 5 or 6
    // whole rows of pixels) below the top-left of B2.
    let b2 = cell(2, 2);
    let (_, pic) = &objects[0];
    assert!((pic.width() - 120.0).abs() < 1.01, "{pic:?}");
    assert!((pic.height() - 80.0).abs() < 1.01, "{pic:?}");
    assert!(
        (pic.min.x - (b2.min.x + 10.0)).abs() < 1.01,
        "{pic:?} vs {b2:?}"
    );
    assert!(
        (pic.min.y - (b2.min.y + 5.67)).abs() < 1.01,
        "{pic:?} vs {b2:?}"
    );
}
