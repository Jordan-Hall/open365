#![no_main]
//! XLSX: open -> save -> reopen, plus edits and recalculation through the command layer.

use libfuzzer_sys::fuzz_target;
use open365_commands::{CellEntry, Command, Target};
use open365_opc::Limits;
use open365_xlsx::Workbook;

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_parts: 2_000,
        max_part_bytes: 8 * 1024 * 1024,
        max_total_bytes: 32 * 1024 * 1024,
        ..Limits::default()
    };
    if let Ok(mut wb) = Workbook::open(data, &limits) {
        let _ = wb.cell(0, 1, 1);
        if let Ok(saved) = wb.save() {
            let _ = Workbook::open(&saved, &limits);
        }
    }
    if let Ok(mut target) = Target::open_xlsx(data) {
        let _ = target.apply(Command::XlsxSetCellValue {
            sheet: 0,
            row: 1,
            col: 1,
            value: CellEntry::Number(1.0),
        });
        let _ = target.apply(Command::XlsxSetCellFormula {
            sheet: 0,
            row: 2,
            col: 1,
            formula: "=A1+1".to_owned(),
        });
        let _ = target.undo();
        let _ = target.redo();
    }
});
