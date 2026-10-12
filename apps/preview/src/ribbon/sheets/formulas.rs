//! Sheets ribbon: the Formulas tab (`formulas.*`).

use crate::dialogs::formulas_function::Category;
use crate::ribbon::{Handled, RibbonCtx};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "formulas.function-library.insert-function",
    "formulas.function-library.auto-sum",
    "formulas.function-library.recently-used",
    "formulas.function-library.financial",
    "formulas.function-library.logical",
    "formulas.function-library.text",
    "formulas.function-library.date-time",
    "formulas.function-library.lookup-reference",
    "formulas.function-library.maths-trig",
    "formulas.function-library.more-functions",
    "formulas.defined-names.name-manager",
    "formulas.formula-auditing.trace-precedents",
    "formulas.formula-auditing.trace-dependents",
    "formulas.formula-auditing.remove-arrows",
    "formulas.formula-auditing.show-formulas",
    "formulas.formula-auditing.error-checking",
    "formulas.formula-auditing.evaluate-formula",
];

/// The listed id `id` is, or is a menu item of (`<id>.<item>`), and the item.
fn split(id: &str) -> Option<(&'static str, &str)> {
    let id = crate::ribbon::base(id);
    IDS.iter().find_map(|k| {
        let rest = id.strip_prefix(k)?;
        match rest.strip_prefix('.') {
            Some(item) => Some((*k, item)),
            None if rest.is_empty() => Some((*k, "")),
            None => None,
        }
    })
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let Some((key, item)) = split(id) else {
        return Handled::No;
    };
    let tail = key.rsplit('.').next().unwrap_or(key);
    let item = if item == "run" { "" } else { item };
    let sheet = &mut *ctx.sheet;
    match tail {
        "insert-function" => sheet.request_insert_function(None),
        "auto-sum" => {
            let f = match item {
                "average" => "AVERAGE",
                "count-numbers" | "count" => "COUNT",
                "max" => "MAX",
                "min" => "MIN",
                _ => "SUM",
            };
            let _ = sheet.auto_sum(f);
        }
        "recently-used" | "financial" | "logical" | "text" | "date-time" | "lookup-reference"
        | "maths-trig" | "more-functions" => {
            if item.is_empty() {
                sheet.request_insert_function(Category::from_control(tail));
            } else {
                sheet.request_function(item);
            }
        }
        "name-manager" => sheet.request_name_manager(),
        "trace-precedents" => sheet.trace_precedents(),
        "trace-dependents" => sheet.trace_dependents(),
        "remove-arrows" => sheet.remove_arrows(),
        "show-formulas" => {
            sheet.toggle_show_formulas();
        }
        "error-checking" => {
            sheet.check_errors();
        }
        "evaluate-formula" => {
            sheet.request_evaluate();
        }
        _ => return Handled::No,
    }
    Handled::Yes
}
