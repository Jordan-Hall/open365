#![no_main]
//! Formula parse, then evaluate on a small sheet. Seed with `fixtures/calc` formulas.

use libfuzzer_sys::fuzz_target;
use open365_calc::{Engine, Value};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if text.len() > 64 * 1024 {
        return;
    }
    let mut engine = Engine::new();
    engine.add_sheet("S");
    for row in 1..=10u32 {
        for col in 1..=3u32 {
            engine.set_value("S", row, col, Value::Number(f64::from(row * col)));
        }
    }
    if engine.set_formula("S", 1, 5, text).is_ok() {
        engine.recalc_all();
        let _ = engine.get("S", 1, 5);
    }
});
