//! Cell-formatting commands: apply, undo, redo and the save round trip.

use open365_commands::{
    CellFormatChange, Command, FillChange, FontChange, RangeRef, Target, apply,
};

fn blank() -> Target {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/xlsx/format-blank.xlsx");
    let bytes = std::fs::read(path).expect("fixture");
    Target::open_xlsx(&bytes).expect("opens")
}

fn bold_a1() -> Command {
    Command::FormatCells {
        sheet: 0,
        range: RangeRef::cell(1, 1),
        change: Box::new(CellFormatChange {
            font: FontChange {
                bold: Some(true),
                ..FontChange::default()
            },
            ..CellFormatChange::default()
        }),
    }
}

fn is_bold(t: &Target, row: u32, col: u32) -> bool {
    t.xlsx()
        .is_some_and(|w| w.cell_format(0, row, col).font.bold)
}

#[test]
fn format_undo_redo_and_save() {
    let mut t = blank();
    apply(&mut t, bold_a1()).expect("bold");
    assert!(is_bold(&t, 1, 1));
    assert!(!is_bold(&t, 2, 1));
    t.undo().expect("undo");
    assert!(!is_bold(&t, 1, 1));
    t.redo().expect("redo");
    assert!(is_bold(&t, 1, 1));

    let bytes = t.snapshot_bytes().expect("saves");
    let reopened = Target::open_xlsx(&bytes).expect("reopens");
    assert!(is_bold(&reopened, 1, 1));
}

#[test]
fn whole_column_style_and_clear() {
    let mut t = blank();
    let whole_column = RangeRef {
        first_row: 1,
        first_col: 3,
        last_row: open365_xlsx::MAX_ROW,
        last_col: 3,
    };
    apply(
        &mut t,
        Command::FormatCells {
            sheet: 0,
            range: whole_column,
            change: Box::new(CellFormatChange {
                fill: Some(FillChange::Solid {
                    color: open365_commands::ColorSpec::rgb("FFFF00"),
                }),
                ..CellFormatChange::default()
            }),
        },
    )
    .expect("fill column");
    let wb = t.xlsx().expect("xlsx");
    assert_ne!(
        wb.cell_format(0, 700, 3).fill.pattern,
        open365_xlsx::FillPattern::None
    );
    assert_eq!(
        wb.cell_format(0, 700, 4).fill.pattern,
        open365_xlsx::FillPattern::None
    );
    apply(
        &mut t,
        Command::ClearFormats {
            sheet: 0,
            range: whole_column,
        },
    )
    .expect("clear");
    let wb = t.xlsx().expect("xlsx");
    assert_eq!(
        wb.cell_format(0, 700, 3).fill.pattern,
        open365_xlsx::FillPattern::None
    );
}

#[test]
fn commands_round_trip_through_json() {
    let json = serde_json::to_string(&bold_a1()).expect("serialises");
    let back: Command = serde_json::from_str(&json).expect("parses");
    assert_eq!(back, bold_a1());
}
