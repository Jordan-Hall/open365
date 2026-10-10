#![no_main]
//! DOCX: open -> save -> reopen, plus a few edits through the command layer.

use libfuzzer_sys::fuzz_target;
use open365_commands::{Command, DocPos, Target};
use open365_docx::Document;
use open365_opc::Limits;

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_parts: 2_000,
        max_part_bytes: 8 * 1024 * 1024,
        max_total_bytes: 32 * 1024 * 1024,
        ..Limits::default()
    };
    if let Ok(mut doc) = Document::open(data, &limits) {
        for i in 0..doc.paragraph_count().min(20) {
            let _ = doc.paragraph_text(i);
        }
        if let Ok(saved) = doc.save() {
            let _ = Document::open(&saved, &limits);
        }
    }
    if let Ok(mut target) = Target::open_docx(data) {
        let _ = target.apply(Command::DocxInsertText {
            para: 0,
            offset: 1,
            text: "x".to_owned(),
        });
        let _ = target.apply(Command::DocxDeleteRange {
            start: DocPos { para: 0, offset: 0 },
            end: DocPos { para: 1, offset: 2 },
        });
        let _ = target.undo();
        let _ = target.redo();
    }
});
