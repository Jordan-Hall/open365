//! Inserting and editing worksheet pictures, shapes and text boxes. Element shapes and names follow what
//! Excel wrote (`fixtures/reference/xlsx/drawing-insert/insert.json`).

mod write_support;

use open365_drawing::Kind;
use open365_xlsx::{Anchor, EditAs, GridMetrics, Workbook};
use write_support::{fixture, has_part, reopen, text_of};

/// A real 96x64 PNG, from an Excel-written fixture.
fn png() -> Vec<u8> {
    let wb = fixture("drawing-basic");
    wb.part_bytes("xl/media/image1.png")
        .expect("image")
        .to_vec()
}

fn blank() -> Workbook {
    fixture("formulas-basic")
}

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="32" viewBox="0 0 48 32"><rect width="48" height="32" fill="#2a6"/></svg>"##;

#[test]
fn a_picture_lands_on_the_cell_at_its_natural_size() {
    let mut wb = blank();
    let id = wb.insert_picture(0, 5, 4, &png(), None).expect("inserts");
    assert_eq!(id, 2);
    let objs = wb.sheet_drawing_objects(0);
    assert_eq!(objs.len(), 1);
    let o = &objs[0];
    assert_eq!(o.object.name, "Picture 1");
    assert!(matches!(o.object.kind, Kind::Picture(_)));
    let Anchor::TwoCell { from, edit_as, .. } = o.anchor else {
        panic!("a two-cell anchor");
    };
    assert_eq!(
        (from.col, from.row, from.col_off, from.row_off),
        (3, 4, 0, 0)
    );
    assert_eq!(edit_as, EditAs::OneCell);
    let m = GridMetrics::for_styles(wb.styles(), 96);
    let r = o.anchor.rect_pt(wb.sheet(0).expect("sheet"), m);
    // 96 x 64 pixels at 96 dpi: 72 x 48 points (Excel: 72.01 x 48.01 for this file's resolution).
    assert!(
        (r.width - 72.0).abs() < 1.0 && (r.height - 48.0).abs() < 1.0,
        "{r:?}"
    );

    let drawing = text_of(&mut wb, "xl/drawings/drawing1.xml");
    assert!(
        drawing.contains("<xdr:twoCellAnchor editAs=\"oneCell\">"),
        "{drawing}"
    );
    assert!(
        drawing.contains("<a:picLocks noChangeAspect=\"1\"/>"),
        "{drawing}"
    );
    assert!(drawing.contains("r:embed=\"rId1\""), "{drawing}");
    let ct = text_of(&mut wb, "[Content_Types].xml");
    assert!(
        ct.contains("Extension=\"png\" ContentType=\"image/png\""),
        "{ct}"
    );
    let back = reopen(&mut wb);
    assert_eq!(back.sheet_drawing_objects(0).len(), 1);
}

#[test]
fn an_svg_comes_with_a_png_fallback() {
    let mut wb = blank();
    wb.insert_picture(0, 2, 2, SVG, None).expect("inserts");
    assert!(has_part(&mut wb, "xl/media/image1.png"));
    assert!(has_part(&mut wb, "xl/media/image2.svg"));
    let drawing = text_of(&mut wb, "xl/drawings/drawing1.xml");
    assert!(drawing.contains("name=\"Graphic 1\""), "{drawing}");
    assert!(drawing.contains("svgBlip"), "{drawing}");
    assert!(drawing.contains("r:embed=\"rId2\""), "{drawing}");
    let o = &wb.sheet_drawing_objects(0)[0];
    let m = GridMetrics::for_styles(wb.styles(), 96);
    let r = o.anchor.rect_pt(wb.sheet(0).expect("sheet"), m);
    // 48 x 32 CSS pixels: 36 x 24 points, as Excel sizes this SVG.
    assert!(
        (r.width - 36.0).abs() < 0.5 && (r.height - 24.0).abs() < 0.5,
        "{r:?}"
    );
    reopen(&mut wb);
}

#[test]
fn not_a_picture_is_refused_and_changes_nothing() {
    let mut wb = blank();
    assert!(wb.insert_picture(0, 1, 1, b"just text", None).is_err());
    assert!(wb.insert_picture(0, 0, 1, &png(), None).is_err());
    assert!(wb.sheet_drawing_objects(0).is_empty());
    assert!(!has_part(&mut wb, "xl/drawings/drawing1.xml"));
}

#[test]
fn shapes_and_text_boxes_take_excels_default_elements() {
    let mut wb = blank();
    let rect = wb
        .insert_shape(0, 5, 4, "rect", false, Some((100.0, 60.0)))
        .expect("rect");
    let oval = wb
        .insert_shape(0, 7, 4, "ellipse", false, None)
        .expect("oval");
    let tb = wb
        .insert_shape(0, 9, 4, "rect", true, Some((100.0, 60.0)))
        .expect("text box");
    let line = wb
        .insert_shape(0, 12, 4, "line", false, Some((100.0, 60.0)))
        .expect("line");
    assert_eq!((rect, oval, tb, line), (2, 3, 4, 5));
    let names: Vec<String> = wb
        .sheet_drawing_objects(0)
        .iter()
        .map(|o| o.object.name.clone())
        .collect();
    assert_eq!(
        names,
        ["Rectangle 1", "Oval 2", "TextBox 3", "Straight Connector 4"]
    );
    let drawing = text_of(&mut wb, "xl/drawings/drawing1.xml");
    // The rectangle: style references, text body anchored top, as measured.
    assert!(
        drawing
            .contains("<a:lnRef idx=\"2\"><a:schemeClr val=\"accent1\"><a:shade val=\"15000\"/>"),
        "{drawing}"
    );
    assert!(
        drawing.contains("<a:fillRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:fillRef>"),
        "{drawing}"
    );
    assert!(drawing.contains("prst=\"ellipse\""), "{drawing}");
    // The text box: txBox flag, white fill, thin grey outline.
    assert!(drawing.contains("<xdr:cNvSpPr txBox=\"1\"/>"), "{drawing}");
    assert!(
        drawing.contains("<a:ln w=\"9525\" cmpd=\"sng\">"),
        "{drawing}"
    );
    assert!(drawing.contains("<xdr:cxnSp macro=\"\">"), "{drawing}");
    // Size 100 x 60 points: 1 270 000 x 762 000 EMU, the values Excel wrote.
    assert!(
        drawing.contains("<a:ext cx=\"1270000\" cy=\"762000\"/>"),
        "{drawing}"
    );
    let back = reopen(&mut wb);
    assert_eq!(back.sheet_drawing_objects(0).len(), 4);
    assert!(
        wb.insert_shape(0, 1, 1, "noSuchShape", false, None)
            .is_err()
    );
    assert!(
        wb.insert_shape(0, 1, 1, "rect", false, Some((0.0, 0.0)))
            .is_err()
    );
}

#[test]
fn edits_change_only_their_object() {
    let mut wb = blank();
    let a = wb
        .insert_shape(0, 2, 2, "rect", false, Some((100.0, 50.0)))
        .expect("a");
    let b = wb
        .insert_shape(0, 8, 2, "ellipse", false, Some((60.0, 60.0)))
        .expect("b");
    let before_b = wb.sheet_drawing_objects(0)[1].clone();

    wb.set_drawing_frame(0, a, 40.0, 30.0, 120.0, 80.0)
        .expect("frame");
    wb.set_drawing_rotation(0, a, 45.0).expect("rotate");
    wb.set_drawing_flip(0, a, true, false).expect("flip");
    wb.set_drawing_fill(0, a, "FF0000").expect("fill");
    wb.set_drawing_line(0, a, Some("accent2"), Some(3.0), Some("dash"))
        .expect("line");
    wb.set_drawing_text(0, a, "Total\nsales & costs")
        .expect("text");
    wb.set_drawing_name(0, a, "Hero", Some("A red box"))
        .expect("name");

    let objs = wb.sheet_drawing_objects(0);
    let oa = &objs[0];
    assert_eq!(oa.object.name, "Hero");
    assert_eq!(oa.object.descr.as_deref(), Some("A red box"));
    assert_eq!(oa.object.xfrm.rot, 45 * 60_000);
    assert!(oa.object.xfrm.flip_h && !oa.object.xfrm.flip_v);
    let m = GridMetrics::for_styles(wb.styles(), 96);
    let r = oa.anchor.rect_pt(wb.sheet(0).expect("sheet"), m);
    assert!(
        (r.x - 40.0).abs() < 1.0 && (r.y - 30.0).abs() < 1.0,
        "{r:?}"
    );
    assert!(
        (r.width - 120.0).abs() < 1.0 && (r.height - 80.0).abs() < 1.0,
        "{r:?}"
    );
    // The other object is exactly as it was.
    assert_eq!(objs[1], before_b);

    let drawing = text_of(&mut wb, "xl/drawings/drawing1.xml");
    assert!(
        drawing.contains("<a:solidFill><a:srgbClr val=\"FF0000\"/></a:solidFill>"),
        "{drawing}"
    );
    assert!(drawing.contains("<a:ln w=\"38100\"><a:solidFill><a:schemeClr val=\"accent2\"/></a:solidFill><a:prstDash val=\"dash\"/></a:ln>"), "{drawing}");
    assert!(
        drawing.contains("<a:t>Total</a:t>") && drawing.contains("sales &amp; costs"),
        "{drawing}"
    );
    assert!(
        drawing.contains("rot=\"2700000\"") && drawing.contains("flipH=\"1\""),
        "{drawing}"
    );
    // Fill sits after the geometry and before the outline.
    let geom = drawing.find("</a:prstGeom>").expect("geometry");
    let fill = drawing.find("<a:solidFill><a:srgbClr").expect("fill");
    let line = drawing.find("<a:ln w=\"38100\"").expect("line");
    assert!(geom < fill && fill < line, "{drawing}");

    // No fill and no outline, then back to a rotation of zero.
    wb.set_drawing_fill(0, a, "none").expect("no fill");
    wb.set_drawing_line(0, a, Some("none"), None, None)
        .expect("no line");
    wb.set_drawing_rotation(0, a, 0.0).expect("unrotate");
    let drawing = text_of(&mut wb, "xl/drawings/drawing1.xml");
    assert!(drawing.contains("<a:noFill/>"), "{drawing}");
    assert!(!drawing.contains("rot="), "{drawing}");
    assert!(wb.set_drawing_fill(0, a, "not-a-colour").is_err());
    assert!(wb.set_drawing_frame(0, 99, 0.0, 0.0, 1.0, 1.0).is_err());
    let _ = b;
    reopen(&mut wb);
}

#[test]
fn arrange_reorders_the_anchors() {
    let mut wb = blank();
    let ids: Vec<u32> = (0..3)
        .map(|i| {
            wb.insert_shape(0, 2 + i * 3, 2, "rect", false, Some((50.0, 30.0)))
                .expect("shape")
        })
        .collect();
    let order = |wb: &Workbook| -> Vec<u32> {
        wb.sheet_drawing_objects(0)
            .iter()
            .map(|o| o.object.id)
            .collect()
    };
    assert_eq!(order(&wb), ids);
    wb.arrange_drawing(0, ids[0], "front").expect("front");
    assert_eq!(order(&wb), [ids[1], ids[2], ids[0]]);
    wb.arrange_drawing(0, ids[0], "backward").expect("backward");
    assert_eq!(order(&wb), [ids[1], ids[0], ids[2]]);
    wb.arrange_drawing(0, ids[0], "back").expect("back");
    assert_eq!(order(&wb), [ids[0], ids[1], ids[2]]);
    wb.arrange_drawing(0, ids[0], "forward").expect("forward");
    assert_eq!(order(&wb), [ids[1], ids[0], ids[2]]);
    assert!(wb.arrange_drawing(0, ids[0], "sideways").is_err());
    reopen(&mut wb);
}

#[test]
fn deleting_a_picture_takes_its_parts_and_keeps_shared_ones() {
    let mut wb = blank();
    let p1 = wb.insert_picture(0, 2, 2, &png(), None).expect("p1");
    let p2 = wb.insert_picture(0, 12, 2, &png(), None).expect("p2");
    assert!(has_part(&mut wb, "xl/media/image1.png") && has_part(&mut wb, "xl/media/image2.png"));
    wb.delete_drawing(0, p1).expect("delete");
    assert!(!has_part(&mut wb, "xl/media/image1.png"));
    assert!(has_part(&mut wb, "xl/media/image2.png"));
    assert_eq!(wb.sheet_drawing_objects(0).len(), 1);
    wb.delete_drawing(0, p2).expect("delete last");
    assert!(!has_part(&mut wb, "xl/media/image2.png"));
    assert!(wb.sheet_drawing_objects(0).is_empty());
    assert!(wb.delete_drawing(0, p2).is_err());
    let back = reopen(&mut wb);
    assert!(back.sheet_drawing_objects(0).is_empty());
}

#[test]
fn placement_modes_rewrite_the_anchor() {
    let mut wb = blank();
    let id = wb
        .insert_shape(0, 3, 3, "rect", false, Some((80.0, 40.0)))
        .expect("shape");
    for mode in [EditAs::OneCell, EditAs::Absolute, EditAs::TwoCell] {
        wb.set_drawing_placement(0, id, mode).expect("placement");
        let o = &wb.sheet_drawing_objects(0)[0];
        assert!(matches!(o.anchor, Anchor::TwoCell { edit_as, .. } if edit_as == mode));
    }
    let drawing = text_of(&mut wb, "xl/drawings/drawing1.xml");
    assert!(
        !drawing.contains("editAs"),
        "move and size is the default: {drawing}"
    );
}

/// Writes a workbook with every kind of object for `tools/office/office-oracle.ps1 -Mode verify`:
/// `OPEN365_DRAWING_SAMPLE=out.xlsx cargo test -p open365-xlsx --test write_drawings -- --ignored sample`.
#[test]
#[ignore = "writes a file for the Excel oracle"]
fn sample_for_the_excel_oracle() {
    let Some(path) = std::env::var_os("OPEN365_DRAWING_SAMPLE") else {
        return;
    };
    let mut wb = blank();
    wb.insert_picture(0, 2, 2, &png(), None).expect("png");
    wb.insert_picture(0, 2, 8, SVG, None).expect("svg");
    let r = wb
        .insert_shape(0, 10, 2, "rightArrow", false, Some((120.0, 60.0)))
        .expect("arrow");
    wb.set_drawing_fill(0, r, "accent2").expect("fill");
    wb.set_drawing_line(0, r, Some("000000"), Some(2.0), Some("dash"))
        .expect("line");
    wb.set_drawing_rotation(0, r, 30.0).expect("rotate");
    let t = wb
        .insert_shape(0, 10, 8, "rect", true, Some((140.0, 50.0)))
        .expect("text box");
    wb.set_drawing_text(0, t, "Total & notes\nsecond line")
        .expect("text");
    wb.insert_shape(0, 16, 2, "line", false, Some((100.0, 40.0)))
        .expect("line");
    let o = wb
        .insert_shape(0, 16, 8, "ellipse", false, None)
        .expect("oval");
    wb.set_drawing_text(0, o, "Oval").expect("text");
    wb.arrange_drawing(0, r, "front").expect("front");
    std::fs::write(path, wb.save().expect("saves")).expect("writes");
}
