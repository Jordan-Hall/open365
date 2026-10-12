//! Sheets ribbon: the Formulas tab (`formulas.*`).

use crate::dialogs::formulas_function::Category;
use crate::ribbon::{Handled, RibbonCtx};
use crate::sheet::sheet_ext::formulas_more::LabelEdges;

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
    "formulas.defined-names.define-name",
    "formulas.defined-names.use-in-formula",
    "formulas.defined-names.create-from-selection",
    "formulas.formula-auditing.trace-precedents",
    "formulas.formula-auditing.trace-dependents",
    "formulas.formula-auditing.remove-arrows",
    "formulas.formula-auditing.show-formulas",
    "formulas.formula-auditing.error-checking",
    "formulas.formula-auditing.evaluate-formula",
    "formulas.formula-auditing.watch-window",
    "formulas.calculation.calculation-options",
    "formulas.calculation.calculate-now",
    "formulas.calculation.calculate-sheet",
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
    // The value a widget sent after `=` (a typed name, the label edges).
    let value = id.split_once('=').map(|(_, v)| v);
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
        "define-name" => match value.map(str::trim).filter(|v| !v.is_empty()) {
            Some(name) => {
                if let Err(e) = sheet.define_name_for_selection(name) {
                    sheet.status = e;
                }
            }
            // Excel opens New Name here; until that dialog exists the Name Manager does.
            None => sheet.request_name_manager(),
        },
        "use-in-formula" => {
            if !item.is_empty()
                && let Err(e) = sheet.use_name_in_formula(item)
            {
                sheet.status = e;
            }
        }
        "create-from-selection" => {
            let edges = value.map_or(
                LabelEdges {
                    top: true,
                    ..LabelEdges::default()
                },
                LabelEdges::parse,
            );
            sheet.create_names_from_selection(edges);
        }
        "watch-window" => match item {
            "add" => sheet.watch_add_selection(),
            "delete" => {
                let rows: Vec<usize> = value
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|v| v.trim().parse().ok())
                    .collect();
                sheet.watch_delete(&rows);
            }
            _ => {
                sheet.toggle_watch_window();
            }
        },
        "calculation-options" => {
            if !item.is_empty()
                && let Err(e) = sheet.set_calculation_mode(item)
            {
                sheet.status = e;
            }
        }
        "calculate-now" => {
            if let Err(e) = sheet.calculate_now() {
                sheet.status = e;
            }
        }
        "calculate-sheet" => {
            if let Err(e) = sheet.calculate_sheet() {
                sheet.status = e;
            }
        }
        _ => return Handled::No,
    }
    Handled::Yes
}
