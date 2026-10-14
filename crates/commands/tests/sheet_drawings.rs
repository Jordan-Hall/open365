//! Sheets pictures, shapes and text boxes through the typed command layer: each insert and edit is one
//! undo step, replays from JSON and reports the new object's id.

use std::path::PathBuf;

use open365_commands::objects_ops::{ObjectEdit, ObjectOrder, ObjectPlacement};
use open365_commands::{Command, EffectClass, ObjectsOp, Target};

fn fixture(name: &str) -> Target {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx")
        .join(format!("{name}.xlsx"));
    Target::open_path(&path).expect("open xlsx")
}

fn png() -> Vec<u8> {
    let t = fixture("drawing-basic");
    t.xlsx()
        .expect("xlsx")
        .part_bytes("xl/media/image1.png")
        .expect("image")
        .to_vec()
}

fn apply(t: &mut Target, o: ObjectsOp) -> open365_commands::Outcome {
    t.apply(Command::SheetObjects { sheet: 0, op: o })
        .expect("applies")
}

/// The id a command reported (`object N`).
fn new_id(out: &open365_commands::Outcome) -> u32 {
    out.notes
        .iter()
        .find_map(|n| n.strip_prefix("object ")?.parse().ok())
        .expect("the outcome names the object")
}

/// Applies `o`, then undoes and redoes it: one undo restores the saved bytes exactly.
fn one_undo_step(t: &mut Target, o: ObjectsOp) -> open365_commands::Outcome {
    let before = t.snapshot_bytes().expect("snapshot");
    let out = apply(t, o);
    let after = t.snapshot_bytes().expect("snapshot");
    assert_ne!(before, after, "the command changed the workbook");
    t.undo().expect("undo");
    assert_eq!(
        t.snapshot_bytes().expect("snapshot"),
        before,
        "one undo is enough"
    );
    t.redo().expect("redo");
    assert_eq!(
        t.snapshot_bytes().expect("snapshot"),
        after,
        "redo reapplies it"
    );
    out
}

fn objects(t: &Target) -> usize {
    t.xlsx().expect("xlsx").sheet_drawing_objects(0).len()
}

#[test]
fn a_picture_is_one_undo_step_and_reports_its_id() {
    let mut t = fixture("formulas-basic");
    let out = one_undo_step(
        &mut t,
        ObjectsOp::InsertPicture {
            data: png(),
            row: 4,
            col: 3,
            name: None,
        },
    );
    assert_eq!(new_id(&out), 2);
    assert_eq!(objects(&t), 1);
    t.undo().expect("undo");
    assert_eq!(objects(&t), 0);
}

#[test]
fn shapes_and_every_edit_are_one_undo_step() {
    let mut t = fixture("formulas-basic");
    let out = apply(
        &mut t,
        ObjectsOp::InsertShape {
            preset: "rect".to_owned(),
            text_box: false,
            row: 2,
            col: 2,
            size: Some((100.0, 50.0)),
        },
    );
    let id = new_id(&out);
    let tb = one_undo_step(
        &mut t,
        ObjectsOp::InsertShape {
            preset: "rect".to_owned(),
            text_box: true,
            row: 9,
            col: 2,
            size: None,
        },
    );
    let tb = new_id(&tb);
    assert_eq!(objects(&t), 2);

    let edits = [
        ObjectEdit::Frame {
            x: 30.0,
            y: 20.0,
            width: 90.0,
            height: 40.0,
        },
        ObjectEdit::Rotate { degrees: 30.0 },
        ObjectEdit::Flip {
            horizontal: true,
            vertical: true,
        },
        ObjectEdit::Fill {
            color: "accent2".to_owned(),
        },
        ObjectEdit::Line {
            color: Some("000000".to_owned()),
            width: Some(2.0),
            dash: Some("dash".to_owned()),
        },
        ObjectEdit::Text {
            text: "Hello".to_owned(),
        },
        ObjectEdit::Name {
            name: "Banner".to_owned(),
            alt: Some("A banner".to_owned()),
        },
        ObjectEdit::Placement {
            mode: ObjectPlacement::Free,
        },
    ];
    for edit in edits {
        one_undo_step(&mut t, ObjectsOp::EditObject { object: id, edit });
    }
    one_undo_step(
        &mut t,
        ObjectsOp::ArrangeObject {
            object: id,
            order: ObjectOrder::Front,
        },
    );
    // The text box takes text like any shape.
    one_undo_step(
        &mut t,
        ObjectsOp::EditObject {
            object: tb,
            edit: ObjectEdit::Text {
                text: "Note".to_owned(),
            },
        },
    );
    one_undo_step(&mut t, ObjectsOp::DeleteObject { object: id });
}

#[test]
fn deleting_is_destructive_and_bad_edits_leave_the_workbook_alone() {
    let mut t = fixture("formulas-basic");
    let id = new_id(&apply(
        &mut t,
        ObjectsOp::InsertShape {
            preset: "ellipse".to_owned(),
            text_box: false,
            row: 1,
            col: 1,
            size: None,
        },
    ));
    let delete = Command::SheetObjects {
        sheet: 0,
        op: ObjectsOp::DeleteObject { object: id },
    };
    assert_eq!(delete.effect(None), EffectClass::Destructive);
    let before = t.snapshot_bytes().expect("snapshot");
    for bad in [
        ObjectsOp::DeleteObject { object: 99 },
        ObjectsOp::EditObject {
            object: id,
            edit: ObjectEdit::Fill {
                color: "nope".to_owned(),
            },
        },
        ObjectsOp::InsertShape {
            preset: "noSuch".to_owned(),
            text_box: false,
            row: 1,
            col: 1,
            size: None,
        },
        ObjectsOp::InsertPicture {
            data: b"text".to_vec(),
            row: 1,
            col: 1,
            name: None,
        },
    ] {
        assert!(
            t.apply(Command::SheetObjects { sheet: 0, op: bad })
                .is_err()
        );
    }
    assert_eq!(t.snapshot_bytes().expect("snapshot"), before);
}

#[test]
fn the_commands_replay_from_json() {
    let json = r##"[
        {"op":"insert_shape","preset":"star5","row":3,"col":4,"size":[80.0,80.0]},
        {"op":"edit_object","object":2,"edit":{"kind":"fill","color":"FFC000"}},
        {"op":"edit_object","object":2,"edit":{"kind":"line","width":1.5}},
        {"op":"edit_object","object":2,"edit":{"kind":"rotate","degrees":15.0}},
        {"op":"arrange_object","object":2,"order":"back"},
        {"op":"delete_object","object":2}
    ]"##;
    let ops: Vec<ObjectsOp> = serde_json::from_str(json).expect("parses");
    let mut t = fixture("formulas-basic");
    for o in ops {
        apply(&mut t, o);
    }
    assert_eq!(objects(&t), 0);
    let text = serde_json::to_string(&ObjectsOp::EditObject {
        object: 7,
        edit: ObjectEdit::Placement {
            mode: ObjectPlacement::MoveAndSize,
        },
    })
    .expect("json");
    assert!(
        text.contains("\"placement\"") && text.contains("move_and_size"),
        "{text}"
    );
}
