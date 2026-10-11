//! `open365 xlsx eval <file> --sheet <s> --cell <ref> [--formula <f>]`: evaluate a formula
//! against the workbook without saving anything.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use open365_commands::evaluate_formula;
use open365_opc::Limits;
use open365_xlsx::{Workbook, parse_cell_ref};

use crate::output::Report;
use crate::{CliError, read};

#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Input .xlsx or .xlsm file. It is never written.
    file: PathBuf,
    /// Sheet name or 0-based index.
    #[arg(long)]
    sheet: String,
    /// Cell the formula is evaluated in, such as B2 (relative references resolve from it).
    #[arg(long)]
    cell: String,
    /// Formula to evaluate. Without it the cell's own formula is recalculated.
    #[arg(long)]
    formula: Option<String>,
}

pub(crate) fn run(args: &Args) -> Result<Report, CliError> {
    let bytes = read(&args.file)?;
    let wb = Workbook::open(&bytes, &Limits::default())
        .map_err(|e| CliError::Other(format!("{}: {e}", args.file.display())))?;
    let sheet = wb
        .sheet_index(&args.sheet)
        .or_else(|| {
            args.sheet
                .parse::<usize>()
                .ok()
                .filter(|i| *i < wb.sheets().len())
        })
        .ok_or_else(|| CliError::Other(format!("no such sheet: {}", args.sheet)))?;
    let (row, col) = parse_cell_ref(&args.cell)
        .ok_or_else(|| CliError::Other("--cell must look like B2".to_owned()))?;
    let out = evaluate_formula(&wb, sheet, row, col, args.formula.as_deref(), &|_, _, _| {
        true
    })
    .map_err(|e| CliError::Other(e.to_string()))?;
    let mut v = serde_json::to_value(&out).map_err(|e| CliError::Other(e.to_string()))?;
    v["sheet"] = serde_json::json!(sheet);
    v["cell"] = serde_json::json!(args.cell);
    v["saved"] = serde_json::json!(false);
    // Without --json the result is printed as one compact JSON line, as it always was.
    let human = v.to_string();
    Ok(Report::new(v, human))
}
