//! The grid draws what the Home tab's formatting commands write: fonts, fills, number formats,
//! and the formats blank cells inherit from a styled row or column.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use egui::{Color32, vec2};
use egui_kittest::Harness;
use open365_commands::{
    AlignmentChange, BorderChange, CellEntry, CellFormatChange, ColorSpec, Command, FillChange,
    FontChange, LineSpec, NumberFormatSpec, RangeRef, Target, UnderlineKind,
};
use open365_ui_egui::{App, PaintedCell};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The blank fixture workbook with `commands` applied, shown in a grid.
fn harness(commands: Vec<Command>) -> Harness<'static, App> {
    let mut target =
        Target::open_path(&root().join("fixtures/xlsx/format-blank.xlsx")).expect("fixture opens");
    for c in commands {
        target.apply(c).expect("command applies");
    }
    let bytes = target.snapshot_bytes().expect("snapshot");
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "open365-grid-format-{}-{}.xlsx",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, &bytes).expect("temp file");
    let app = App::new(Some(path.clone()));
    std::fs::remove_file(&path).ok();
    let mut h = Harness::builder()
        .with_size(vec2(900.0, 1200.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.run();
    h
}

fn painted(h: &Harness<'_, App>, row: u32, col: u32) -> Option<PaintedCell> {
    h.state()
        .grid
        .painted()
        .into_iter()
        .find(|p| p.row == row && p.col == col)
}

fn cells(range: (u32, u32, u32, u32), change: CellFormatChange) -> Command {
    Command::FormatCells {
        sheet: 0,
        range: RangeRef {
            first_row: range.0,
            first_col: range.1,
            last_row: range.2,
            last_col: range.3,
        },
        change: Box::new(change),
    }
}

fn yellow() -> CellFormatChange {
    CellFormatChange {
        fill: Some(FillChange::Solid {
            color: ColorSpec::rgb("FFFF00"),
        }),
        ..CellFormatChange::default()
    }
}

#[test]
fn stored_cells_show_their_font_fill_and_number_format() {
    let h = harness(vec![
        Command::XlsxSetCellValue {
            sheet: 0,
            row: 2,
            col: 2,
            value: CellEntry::Number(0.125),
        },
        cells(
            (2, 2, 2, 2),
            CellFormatChange {
                font: FontChange {
                    bold: Some(true),
                    italic: Some(true),
                    underline: Some(UnderlineKind::Single),
                    color: Some(ColorSpec::rgb("0070C0")),
                    ..FontChange::default()
                },
                fill: yellow().fill,
                number_format: Some(NumberFormatSpec::Builtin(10)),
                alignment: AlignmentChange {
                    horizontal: Some(open365_commands::HAlign::Center),
                    ..AlignmentChange::default()
                },
                ..CellFormatChange::default()
            },
        ),
    ]);
    let p = painted(&h, 2, 2).expect("B2 is drawn");
    assert_eq!(p.text, "12.50%");
    assert!(p.bold && p.italic && p.underline);
    assert_eq!(p.fill, Some(Color32::from_rgb(255, 255, 0)));
    assert_eq!(p.font_color, Color32::from_rgb(0x00, 0x70, 0xC0));
    assert_eq!(p.horizontal, open365_xlsx::HorizontalAlignment::Center);
}

#[test]
fn a_formatted_column_fills_its_blank_cells() {
    let h = harness(vec![cells((1, 2, 1_048_576, 2), yellow())]);
    for row in [1, 30, 45] {
        let p = painted(&h, row, 2).unwrap_or_else(|| panic!("B{row} is drawn"));
        assert_eq!(p.fill, Some(Color32::from_rgb(255, 255, 0)), "B{row}");
        assert!(p.text.is_empty());
    }
    assert!(painted(&h, 30, 1).is_none_or(|p| p.fill.is_none()));
    assert!(painted(&h, 30, 3).is_none_or(|p| p.fill.is_none()));
}

#[test]
fn a_formatted_row_fills_its_blank_cells() {
    let h = harness(vec![cells((4, 1, 4, 16_384), yellow())]);
    for col in [1, 3, 8] {
        let p = painted(&h, 4, col).unwrap_or_else(|| panic!("row 4, column {col} is drawn"));
        assert_eq!(p.fill, Some(Color32::from_rgb(255, 255, 0)), "column {col}");
    }
    assert!(painted(&h, 5, 3).is_none_or(|p| p.fill.is_none()));
}

#[test]
fn borders_on_a_whole_column_reach_blank_cells() {
    let borders = Command::SetBorders {
        sheet: 0,
        range: RangeRef {
            first_row: 1,
            first_col: 3,
            last_row: 1_048_576,
            last_col: 3,
        },
        borders: Box::new(BorderChange {
            left: Some(LineSpec {
                style: "thin".to_owned(),
                color: None,
            }),
            ..BorderChange::default()
        }),
    };
    let h = harness(vec![borders]);
    // The inherited cells are reported, so the border pass saw them.
    assert!(painted(&h, 40, 3).is_some());
    assert!(painted(&h, 40, 4).is_none());
}
