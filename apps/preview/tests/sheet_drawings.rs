//! Sheets pictures, shapes and text boxes in the preview: edits of a selected object (each one undo
//! step), pointer drags (one command on release), and what the grid draws (shape, text, handles).

#![allow(clippy::unwrap_used)]

mod grid_draw_common;
use grid_draw_common::*;
use open365_commands::objects_ops::{ObjectEdit, ObjectOrder, ObjectPlacement};
use open365_drawing::geom::P;
use open365_drawing::handles::{Handle, handle_point};
use open365_preview::Preview;
use open365_preview::sheet::SheetModel;

const HANDLE_GREY: [u8; 3] = [0x6e, 0x6e, 0x6e];

fn model(p: &mut Preview) -> &mut SheetModel {
    p.shell.sheet_mut()
}

fn with_rect(p: &mut Preview) -> u32 {
    model(p).select_cell((3, 3));
    model(p).insert_shape("rect").unwrap()
}

#[test]
fn every_edit_is_one_undo_step_and_touches_only_its_object() {
    let mut p = sheets();
    let a = with_rect(&mut p);
    model(&mut p).select_cell((10, 3));
    let b = model(&mut p).insert_shape("ellipse").unwrap();
    let b_before = model(&mut p).drawing_object(b).unwrap();
    let edits = [
        ObjectEdit::Frame {
            x: 50.0,
            y: 40.0,
            width: 130.0,
            height: 70.0,
        },
        ObjectEdit::Rotate { degrees: 20.0 },
        ObjectEdit::Fill {
            color: "FFC000".to_owned(),
        },
        ObjectEdit::Line {
            color: Some("accent2".to_owned()),
            width: Some(3.0),
            dash: Some("dash".to_owned()),
        },
        ObjectEdit::Text {
            text: "Quarter\nresults".to_owned(),
        },
        ObjectEdit::Placement {
            mode: ObjectPlacement::Free,
        },
        ObjectEdit::Name {
            name: "Banner".to_owned(),
            alt: None,
        },
    ];
    for edit in edits {
        let before = model(&mut p).drawing_object(a).unwrap();
        model(&mut p).drawing_edit(a, edit.clone()).unwrap();
        let after = model(&mut p).drawing_object(a).unwrap();
        // Fill and outline show in the file, not in the info: compare the saved bytes too.
        assert!(model(&mut p).undo());
        assert_eq!(
            model(&mut p).drawing_object(a).unwrap(),
            before,
            "undo {edit:?}"
        );
        assert!(model(&mut p).redo());
        assert_eq!(
            model(&mut p).drawing_object(a).unwrap(),
            after,
            "redo {edit:?}"
        );
    }
    let o = model(&mut p).drawing_object(a).unwrap();
    assert_eq!(o.name, "Banner");
    assert_eq!(o.text.as_deref(), Some("Quarter\nresults"));
    assert_eq!(o.frame.rot_deg, 20.0);
    assert_eq!(o.placement, ObjectPlacement::Free);
    assert!((o.frame.x - 50.0).abs() < 1.0 && (o.frame.w - 130.0).abs() < 1.0);
    assert_eq!(model(&mut p).drawing_object(b).unwrap(), b_before);
}

#[test]
fn order_and_delete() {
    let mut p = sheets();
    let a = with_rect(&mut p);
    model(&mut p).select_cell((9, 3));
    let b = model(&mut p).insert_text_box().unwrap();
    let ids = |p: &mut Preview| {
        model(p)
            .drawing_objects()
            .iter()
            .map(|o| o.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&mut p), [a, b]);
    model(&mut p)
        .drawing_arrange(a, ObjectOrder::Front)
        .unwrap();
    assert_eq!(ids(&mut p), [b, a]);
    assert!(model(&mut p).undo());
    assert_eq!(ids(&mut p), [a, b]);
    // Delete removes the selected object; a second Delete has nothing to do.
    model(&mut p).drawing_select(Some(b));
    assert!(model(&mut p).drawing_delete_selected());
    assert_eq!(ids(&mut p), [a]);
    assert_eq!(model(&mut p).drawing_selected(), None);
    assert!(!model(&mut p).drawing_delete_selected());
    assert!(model(&mut p).undo());
    assert_eq!(ids(&mut p), [a, b]);
    assert!(model(&mut p).drawing_delete(99).is_err());
}

#[test]
fn dragging_the_body_moves_the_object_in_one_step() {
    let mut p = sheets();
    let id = with_rect(&mut p);
    let f = model(&mut p).drawing_object(id).unwrap().frame;
    let (cx, cy) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
    assert!(model(&mut p).drawing_pointer_down(cx, cy, false));
    assert!(model(&mut p).drawing_pointer_move(cx + 20.0, cy + 10.0, false));
    // Nothing is applied until the button comes up.
    assert_eq!(model(&mut p).drawing_object(id).unwrap().frame, f);
    assert!(model(&mut p).drawing_pointer_up(cx + 30.0, cy + 20.0, false));
    let g = model(&mut p).drawing_object(id).unwrap().frame;
    assert!(
        (g.x - f.x - 30.0).abs() < 1.0 && (g.y - f.y - 20.0).abs() < 1.0,
        "{g:?}"
    );
    assert!((g.w - f.w).abs() < 1.0 && (g.h - f.h).abs() < 1.0);
    assert!(model(&mut p).undo());
    assert_eq!(model(&mut p).drawing_object(id).unwrap().frame, f);
}

#[test]
fn a_click_selects_without_changing_and_empty_space_deselects() {
    let mut p = sheets();
    let id = with_rect(&mut p);
    model(&mut p).drawing_select(None);
    let f = model(&mut p).drawing_object(id).unwrap().frame;
    let (cx, cy) = (f.x + 5.0, f.y + 5.0);
    assert!(model(&mut p).drawing_pointer_down(cx, cy, false));
    assert_eq!(model(&mut p).drawing_selected(), Some(id));
    assert!(model(&mut p).drawing_pointer_up(cx + 1.0, cy, false));
    assert_eq!(model(&mut p).drawing_object(id).unwrap().frame, f);
    // The click was no edit: the only undo step left is the insert.
    assert!(model(&mut p).undo());
    assert!(model(&mut p).drawing_objects().is_empty());
    assert!(!model(&mut p).drawing_pointer_down(900.0, 900.0, false));
}

#[test]
fn dragging_a_handle_resizes_and_the_rotation_handle_rotates() {
    let mut p = sheets();
    let id = with_rect(&mut p);
    let f = model(&mut p).drawing_object(id).unwrap().frame;
    let corner = handle_point(&f, Handle::SouthEast);
    assert!(model(&mut p).drawing_pointer_down(corner.x, corner.y, false));
    assert!(model(&mut p).drawing_pointer_up(corner.x + 40.0, corner.y + 10.0, false));
    let g = model(&mut p).drawing_object(id).unwrap().frame;
    assert!(
        (g.x - f.x).abs() < 1.0 && (g.y - f.y).abs() < 1.0,
        "the opposite corner stays: {g:?}"
    );
    assert!(
        (g.w - f.w - 40.0).abs() < 1.0 && (g.h - f.h - 10.0).abs() < 1.0,
        "{g:?}"
    );

    let top = handle_point(&g, Handle::Rotate);
    assert!(model(&mut p).drawing_pointer_down(top.x, top.y, false));
    // Pull the handle to the right of the centre: a quarter turn clockwise.
    let c = P::new(g.x + g.w / 2.0, g.y + g.h / 2.0);
    assert!(model(&mut p).drawing_pointer_up(c.x + 100.0, c.y, false));
    let r = model(&mut p).drawing_object(id).unwrap().frame;
    assert!((r.rot_deg - 90.0).abs() < 0.01, "{}", r.rot_deg);
    assert!((r.w - g.w).abs() < 1.0);
    assert!(model(&mut p).undo());
    assert_eq!(model(&mut p).drawing_object(id).unwrap().frame.rot_deg, 0.0);
    // Escape forgets a drag.
    assert!(model(&mut p).drawing_pointer_down(g.x + 5.0, g.y + 5.0, false));
    model(&mut p).drawing_pointer_move(g.x + 80.0, g.y + 80.0, false);
    model(&mut p).drawing_pointer_cancel();
    assert!(!model(&mut p).drawing_pointer_up(g.x + 80.0, g.y + 80.0, false));
    assert_eq!(model(&mut p).drawing_object(id).unwrap().frame, g);
}

#[test]
fn the_grid_draws_the_shape_its_text_and_the_selection_handles() {
    let mut p = sheets();
    let blank = p.draw_list();
    let id = with_rect(&mut p);
    model(&mut p).drawing_select(None);
    let plain = p.draw_list();
    assert!(
        plain.vertices().len() > blank.vertices().len(),
        "the shape is drawn"
    );
    assert_eq!(
        count(&plain, HANDLE_GREY),
        0,
        "no handles when nothing is selected"
    );
    model(&mut p).drawing_select(Some(id));
    assert!(
        count(&p.draw_list(), HANDLE_GREY) > 0,
        "handles on the selected object"
    );
    // Text adds glyphs.
    let before = p.draw_list().vertices().len();
    model(&mut p)
        .drawing_edit(
            id,
            ObjectEdit::Text {
                text: "Hello".to_owned(),
            },
        )
        .unwrap();
    assert!(p.draw_list().vertices().len() > before, "the text is drawn");
    // A drag shows the frame it will end in.
    let f = model(&mut p).drawing_object(id).unwrap().frame;
    let at = handle_point(&f, Handle::SouthEast);
    assert!(model(&mut p).drawing_pointer_down(at.x, at.y, false));
    model(&mut p).drawing_pointer_move(at.x + 60.0, at.y + 60.0, false);
    assert!(count(&p.draw_list(), HANDLE_GREY) > 0);
    model(&mut p).drawing_pointer_cancel();
}

#[test]
fn a_device_point_maps_back_to_points_from_a1() {
    use open365_preview::sheet::Geo;
    use open365_preview::sheet_ext::drawing::pointer_to_sheet_pt;
    let mut p = sheets();
    let m = model(&mut p);
    for top_left in [(1, 1), (12, 4)] {
        let geo = m.geometry();
        let g = Geo::new(
            (0.0, 0.0, 1600.0, 1000.0),
            (50.0, 25.0),
            1.25,
            geo.clone(),
            top_left,
        );
        let (l, t, r, b) = g.cell(top_left.0 + 3, top_left.1 + 2);
        let (x, y) = pointer_to_sheet_pt(&g, m, (l + r) / 2.0, (t + b) / 2.0).unwrap();
        let k = 72.0 / 96.0;
        let (c, rw) = (top_left.1 + 2, top_left.0 + 3);
        let (x0, x1) = (geo.col_x(c) as f64 * k, geo.col_x(c + 1) as f64 * k);
        let (y0, y1) = (geo.row_y(rw) as f64 * k, geo.row_y(rw + 1) as f64 * k);
        assert!(x > x0 && x < x1, "{top_left:?}: {x} in {x0}..{x1}");
        assert!(y > y0 && y < y1, "{top_left:?}: {y} in {y0}..{y1}");
    }
    let geo = m.geometry();
    let g = Geo::new((0.0, 0.0, 1600.0, 1000.0), (50.0, 25.0), 1.25, geo, (1, 1));
    assert!(
        pointer_to_sheet_pt(&g, m, 10.0, 10.0).is_none(),
        "over the headers"
    );
}
