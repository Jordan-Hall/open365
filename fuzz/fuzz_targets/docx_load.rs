#![no_main]
//! DOCX document load and save. Seed the corpus with `fixtures/docx/*.docx`.

use libfuzzer_sys::fuzz_target;
use open365_docx::Document;
use open365_opc::Limits;

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_parts: 2_000,
        max_part_bytes: 8 * 1024 * 1024,
        max_total_bytes: 32 * 1024 * 1024,
        max_ratio: 200,
        ratio_floor_bytes: 1024 * 1024,
    };
    if let Ok(mut doc) = Document::open(data, &limits) {
        let _ = doc.save();
    }
});
