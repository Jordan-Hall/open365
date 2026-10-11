//! Read-only views over a document: outline, text, cells, formulas, styles, recalc. They call
//! the same view functions as the MCP tools (`open365_mcp::views`), so both surfaces return the
//! same data. Hidden content is left out unless `--hidden` is given.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::Args as ClapArgs;
use open365_docx::Document;
use open365_mcp::{views, views_ext};
use open365_opc::Limits;
use open365_xlsx::Workbook;
use serde_json::{Value, json};

use crate::output::Report;
use crate::{CliError, read};

#[derive(Debug, ClapArgs)]
pub(crate) struct FileArgs {
    /// Input file.
    file: PathBuf,
    /// Include hidden text, sheets, rows and columns.
    #[arg(long)]
    hidden: bool,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct TextArgs {
    file: PathBuf,
    #[arg(long)]
    hidden: bool,
    /// First paragraph index.
    #[arg(long, default_value_t = 0)]
    start: usize,
    /// Most paragraphs to return (1 to 1000).
    #[arg(long, default_value_t = 200)]
    count: usize,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct CellsArgs {
    file: PathBuf,
    #[arg(long)]
    hidden: bool,
    /// Sheet name or 0-based index.
    #[arg(long)]
    sheet: String,
    /// Range such as A1:D20 (at most 10000 cells).
    #[arg(long)]
    range: String,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct SheetArgs {
    file: PathBuf,
    #[arg(long)]
    hidden: bool,
    /// Sheet name or 0-based index.
    #[arg(long)]
    sheet: String,
    /// Range such as A1:D20. Default: the used range.
    #[arg(long)]
    range: Option<String>,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct StylesArgs {
    file: PathBuf,
    #[arg(long)]
    hidden: bool,
    /// Sheet name or 0-based index; needed with --range.
    #[arg(long, requires = "range")]
    sheet: Option<String>,
    /// Describe the formatting of these cells (at most 1000), such as A1:C5.
    #[arg(long, requires = "sheet")]
    range: Option<String>,
}

fn open_docx(path: &Path) -> Result<Document, CliError> {
    Document::open(&read(path)?, &Limits::default())
        .map_err(|e| CliError::Other(format!("{}: {e}", path.display())))
}

fn open_xlsx(path: &Path) -> Result<Workbook, CliError> {
    Workbook::open(&read(path)?, &Limits::default())
        .map_err(|e| CliError::Other(format!("{}: {e}", path.display())))
}

fn sheet_value(text: &str) -> Value {
    text.parse::<u64>()
        .map_or_else(|_| json!(text), |n| json!(n))
}

fn sheet_index(wb: &Workbook, sheet: &str, hidden: bool) -> Result<usize, CliError> {
    views::resolve_sheet(wb, &sheet_value(sheet), hidden).map_err(CliError::Other)
}

pub(crate) fn docx_outline(args: &FileArgs) -> Result<Report, CliError> {
    let doc = open_docx(&args.file)?;
    let data = views::docx_outline(&doc, args.hidden);
    let mut human = format!("{} paragraphs\n", doc.paragraph_count());
    if let Some(items) = data["headings"].as_array() {
        for h in items {
            let level = h["level"].as_u64().unwrap_or(1) as usize;
            human.push_str(&format!(
                "{}{} (paragraph {})\n",
                "  ".repeat(level.saturating_sub(1)),
                h["text"].as_str().unwrap_or(""),
                h["paragraph"]
            ));
        }
    }
    Ok(Report::new(data, human))
}

pub(crate) fn docx_text(args: &TextArgs) -> Result<Report, CliError> {
    let doc = open_docx(&args.file)?;
    let data = views::docx_text(&doc, args.hidden, args.start, args.count);
    let mut human = String::new();
    if let Some(items) = data["paragraphs"].as_array() {
        for p in items {
            human.push_str(&format!(
                "[{}] {}\n",
                p["paragraph"],
                p["text"].as_str().unwrap_or("")
            ));
        }
    }
    if let Some(next) = data["next_paragraph"].as_u64() {
        human.push_str(&format!("... more: --start {next}\n"));
    }
    Ok(Report::new(data, human))
}

pub(crate) fn docx_styles(args: &FileArgs) -> Result<Report, CliError> {
    let doc = open_docx(&args.file)?;
    Ok(Report::json_only(views_ext::docx_styles(&doc)))
}

pub(crate) fn xlsx_outline(args: &FileArgs) -> Result<Report, CliError> {
    let wb = open_xlsx(&args.file)?;
    let data = views::xlsx_outline(&wb, args.hidden);
    let mut human = String::new();
    if let Some(sheets) = data["sheets"].as_array() {
        for s in sheets {
            human.push_str(&format!(
                "{}: {} ({})\n",
                s["sheet"],
                s["name"].as_str().unwrap_or(""),
                s["used_range"].as_str().unwrap_or("empty")
            ));
        }
    }
    Ok(Report::new(data, human))
}

pub(crate) fn xlsx_cells(args: &CellsArgs) -> Result<Report, CliError> {
    let wb = open_xlsx(&args.file)?;
    let index = sheet_index(&wb, &args.sheet, args.hidden)?;
    let mut data =
        views::xlsx_cells(&wb, index, &args.range, args.hidden).map_err(CliError::Other)?;
    data["sheet"] = json!(index);
    let mut human = String::new();
    if let Some(cells) = data["cells"].as_array() {
        for c in cells {
            let formula = c["formula"]
                .as_str()
                .map(|f| format!("  (={f})"))
                .unwrap_or_default();
            human.push_str(&format!(
                "{}\t{}{}\n",
                c["ref"].as_str().unwrap_or(""),
                c["value"],
                formula
            ));
        }
    }
    Ok(Report::new(data, human))
}

pub(crate) fn xlsx_formulas(args: &SheetArgs) -> Result<Report, CliError> {
    let wb = open_xlsx(&args.file)?;
    let index = sheet_index(&wb, &args.sheet, args.hidden)?;
    let mut data = views_ext::xlsx_formulas(&wb, index, args.range.as_deref(), args.hidden)
        .map_err(CliError::Other)?;
    data["sheet"] = json!(index);
    let mut human = String::new();
    if let Some(items) = data["formulas"].as_array() {
        for f in items {
            human.push_str(&format!(
                "{}\t={}\t{}\n",
                f["ref"].as_str().unwrap_or(""),
                f["formula"].as_str().unwrap_or(""),
                f["value"]
            ));
        }
    }
    Ok(Report::new(data, human))
}

pub(crate) fn xlsx_styles(args: &StylesArgs) -> Result<Report, CliError> {
    let wb = open_xlsx(&args.file)?;
    let index = match &args.sheet {
        Some(s) => sheet_index(&wb, s, args.hidden)?,
        None => 0,
    };
    let mut data = views_ext::xlsx_styles(&wb, index, args.range.as_deref(), args.hidden)
        .map_err(CliError::Other)?;
    if args.range.is_some() {
        data["sheet"] = json!(index);
    }
    Ok(Report::json_only(data))
}

pub(crate) fn xlsx_recalc(args: &SheetArgs) -> Result<Report, CliError> {
    let wb = open_xlsx(&args.file)?;
    let index = sheet_index(&wb, &args.sheet, args.hidden)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let data = views_ext::xlsx_recalc(&wb, index, args.range.as_deref(), args.hidden, &cancel)
        .map_err(|e| CliError::Other(e.to_string()))?;
    let mut human = format!(
        "{} formulas checked, {} stale; nothing was saved\n",
        data["formulas_checked"], data["stale"]
    );
    for x in data["differences"].as_array().into_iter().flatten() {
        human.push_str(&format!(
            "{}\tstored {}\trecalculated {}\n",
            x["ref"].as_str().unwrap_or(""),
            x["stored"],
            x["recalculated"]
        ));
    }
    Ok(Report::new(data, human))
}
