#![no_main]
//! Formula text: parse -> print -> load into an engine -> recalculate.

use libfuzzer_sys::fuzz_target;
use open365_calc::{Engine, Value, parse_formula, to_formula_string};

fuzz_target!(|text: &str| {
    if let Ok(ast) = parse_formula(text) {
        let _ = to_formula_string(&ast);
        let mut engine = Engine::new();
        engine.add_sheet("Sheet1");
        engine.set_value("Sheet1", 1, 1, Value::Number(2.0));
        if engine.set_formula("Sheet1", 1, 2, text).is_ok() {
            let _ = engine.recalc_dirty();
            let _ = engine.get("Sheet1", 1, 2);
        }
    }
});
