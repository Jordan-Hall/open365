//! `open365` CLI over the command layer: open, inspect, edit, recalc, save and export for
//! documents and workbooks, `commands`/`run` for every command, `apply` for command files,
//! `mcp` for the agent server. `--json` gives the `open365.cli/1` envelope (`output.rs`).

mod apply_cmd;
mod commands_cmd;
mod eval_cmd;
mod export_pdf_cmd;
mod inspect_cmd;
mod layout_cmd;
mod mcp_cmd;
mod output;
mod read_cmd;
mod share_cmd;
mod template_cmd;

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use open365_mcp::Scope;
use open365_opc::OpcError;
use open365_testkit::{inspect, roundtrip};
use serde_json::json;

use inspect_cmd::Kind;
use output::{Report, finish};

#[derive(Debug, Parser)]
#[command(name = "open365", version)]
struct Cli {
    /// Print one JSON object (schema open365.cli/1) instead of text; see docs/reference/cli.md.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Args)]
struct InspectArgs {
    /// Also list what a user may not see (hidden text, comments, tracked changes, hidden
    /// sheets/rows/columns, defined names, external links, macros, custom XML, thumbnails,
    /// unreachable parts, author metadata).
    #[arg(long)]
    hidden: bool,
    file: PathBuf,
}

#[derive(Debug, Subcommand)]
enum XlsxCommand {
    /// Print parts, relationships and content types.
    Inspect(InspectArgs),
    /// List the visible sheets with their used ranges.
    Outline(read_cmd::FileArgs),
    /// Print the values and formulas of a range.
    Cells(read_cmd::CellsArgs),
    /// List the formula cells of a sheet with their stored values.
    Formulas(read_cmd::SheetArgs),
    /// Print cell styles and number formats; with --range also the formatting of those cells.
    Styles(read_cmd::StylesArgs),
    /// Recalculate a sheet's formulas from scratch and compare with the stored values.
    /// Read-only: nothing is saved.
    Recalc(read_cmd::SheetArgs),
    /// Evaluate a formula against the workbook and print the result as JSON. Read-only:
    /// nothing is saved.
    Eval(eval_cmd::Args),
    /// Make a new .xlsx or .xlsm from a .xltx or .xltm template. A VBA project is kept and
    /// never run. Replacing a file needs --overwrite and --allow-destructive.
    NewFromTemplate(template_cmd::Args),
}

#[derive(Debug, Subcommand)]
enum DocxCommand {
    /// Print parts, relationships and content types.
    Inspect(InspectArgs),
    /// List the headings with their paragraph index.
    Outline(read_cmd::FileArgs),
    /// Print paragraph text.
    Text(read_cmd::TextArgs),
    /// List the styles of the style sheet.
    Styles(read_cmd::FileArgs),
    /// Lay the document out and write it as a tagged PDF 1.7 file.
    ///
    /// Fonts are the bundled ones (Calibri -> Carlito, Cambria -> Caladea) unless
    /// --system-fonts is given. No author or date is written unless asked for.
    ExportPdf(export_pdf_cmd::Args),
    /// Print pages, lines and table cell boxes from the layout engine (read-only).
    Layout(layout_cmd::Args),
    /// Make a new .docx or .docm from a .dotx or .dotm template. A VBA project is kept and
    /// never run. Replacing a file needs --overwrite and --allow-destructive.
    NewFromTemplate(template_cmd::Args),
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the parts, content types and integrity issues of any OPC package.
    Inspect { file: PathBuf },
    /// DOCX commands.
    Docx {
        #[command(subcommand)]
        command: DocxCommand,
    },
    /// XLSX commands.
    Xlsx {
        #[command(subcommand)]
        command: XlsxCommand,
    },
    /// List every command of the command layer (the catalogue `run` and MCP use).
    Commands(commands_cmd::ListArgs),
    /// Share a document: send a copy or PDF, list hidden content, write a sanitised copy.
    Share {
        #[command(subcommand)]
        command: share_cmd::ShareCommand,
    },
    /// Run one command by name and save the result (`save` and `export_pdf` write to their
    /// own `path`). Effect classes and refusals work as for `apply`.
    Run(commands_cmd::RunArgs),
    /// Apply a JSON command file through the command layer and save the result.
    ///
    /// Each command's effect class is printed to stdout and one JSON audit line per command
    /// goes to stderr. Destructive and egress commands are refused (and nothing runs) unless
    /// --allow-destructive is given.
    Apply {
        /// JSON array of commands (the serialised command log).
        commands: PathBuf,
        /// Input .docx or .xlsx file.
        input: PathBuf,
        /// Output file.
        #[arg(short, long, required_unless_present = "dry_run")]
        output: Option<PathBuf>,
        /// Run every command in memory, report, and write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Permit destructive and egress commands, and replacing an existing output file.
        #[arg(long)]
        allow_destructive: bool,
        /// Fence the output file to these folders (repeatable): the path is resolved with
        /// symlinks followed, must lie inside one, and relative paths resolve against the first.
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
    },
    /// Serve document tools to an agent over stdio (MCP). Read-only unless --allow-edit or
    /// --grant.
    ///
    /// Every tool call appends one JSON line to --audit-log. Destructive actions need the
    /// operator's confirmation: --allow-destructive approves them in advance, --confirm
    /// terminal asks at this terminal, otherwise they are refused.
    Mcp {
        /// Folder agents may open and save in (repeatable). Relative paths use the first.
        #[arg(long = "root", required = true)]
        roots: Vec<PathBuf>,
        /// Audit log file (appended; one JSON line per call).
        #[arg(long)]
        audit_log: PathBuf,
        /// Grant the scopes edit, save and export.
        #[arg(long)]
        allow_edit: bool,
        /// Grant one scope for the session (repeatable): edit, save, export or
        /// execute-external. read is always granted.
        #[arg(long = "grant", value_parser = ["edit", "save", "export", "execute-external"])]
        grants: Vec<String>,
        /// Approve destructive actions and file replacement in advance for the session.
        #[arg(long)]
        allow_destructive: bool,
        /// Who confirms destructive actions and scope requests: deny (nobody, the default)
        /// or terminal (ask on this machine's console, apart from the agent's pipe).
        #[arg(long, value_parser = ["deny", "terminal"], default_value = "deny")]
        confirm: String,
        /// Include hidden text, hidden sheets, rows and columns, and hidden-content findings.
        #[arg(long)]
        include_hidden: bool,
        /// Longest a layout_summary, export_pdf, recalc or evaluate_formula call may run, in
        /// seconds. A call that times out is cancelled: layout, the PDF writer and
        /// recalculation stop at their next check, and an export that timed out leaves no file.
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout_secs: u64,
    },
    /// Verify that files survive read -> write -> read unchanged.
    Roundtrip {
        /// Required: run the check without writing anything.
        #[arg(long, required = true)]
        check: bool,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

#[derive(Debug)]
pub(crate) enum CliError {
    Io(PathBuf, std::io::Error),
    Opc(PathBuf, OpcError),
    /// The policy refused the request (exit code 3).
    Refused(String),
    Other(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Self::Opc(p, e) => write!(f, "{}: {e}", p.display()),
            Self::Refused(m) | Self::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CliError {}

pub(crate) fn read(path: &Path) -> Result<Vec<u8>, CliError> {
    std::fs::read(path).map_err(|e| CliError::Io(path.to_owned(), e))
}

fn run_inspect(file: &Path) -> Result<Report, CliError> {
    let info = inspect(&read(file)?).map_err(|e| CliError::Opc(file.to_owned(), e))?;
    Ok(Report::new(
        inspect_cmd::to_json(&info, None),
        inspect_cmd::render_text(&info, None),
    ))
}

/// Checks one file; returns its JSON entry, its text line and whether it passed.
fn check_file(file: &Path) -> (serde_json::Value, String, bool) {
    let name = file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let result =
        read(file).and_then(|b| roundtrip(&b).map_err(|e| CliError::Opc(file.to_owned(), e)));
    match result {
        Ok(r) if r.is_lossless() => (
            json!({"file": name, "passed": true, "parts": r.part_count}),
            format!("PASS {name} parts={}", r.part_count),
            true,
        ),
        Ok(r) => (
            json!({"file": name, "passed": false, "detail": r.describe()}),
            format!("FAIL {name}: {}", r.describe()),
            false,
        ),
        Err(e) => (
            json!({"file": name, "passed": false, "detail": e.to_string()}),
            format!("FAIL {name}: {e}"),
            false,
        ),
    }
}

fn run_roundtrip(files: &[PathBuf]) -> Result<Report, CliError> {
    let checks: Vec<_> = files.iter().map(|f| check_file(f)).collect();
    let failed = checks.iter().filter(|c| !c.2).count();
    let data = json!({
        "files": checks.iter().map(|c| c.0.clone()).collect::<Vec<_>>(),
        "passed": checks.len() - failed,
        "failed": failed,
    });
    let lines: Vec<&str> = checks.iter().map(|c| c.1.as_str()).collect();
    let report = Report::new(data, lines.join("\n"));
    Ok(if failed == 0 {
        report
    } else {
        report.failed(format!("{failed} file(s) did not round-trip"))
    })
}

type Outcome = (&'static str, Result<Report, CliError>);

fn docx_command(command: &DocxCommand) -> Outcome {
    match command {
        DocxCommand::Inspect(a) => (
            "docx inspect",
            inspect_cmd::run(Kind::Docx, &a.file, a.hidden),
        ),
        DocxCommand::Outline(a) => ("docx outline", read_cmd::docx_outline(a)),
        DocxCommand::Text(a) => ("docx text", read_cmd::docx_text(a)),
        DocxCommand::Styles(a) => ("docx styles", read_cmd::docx_styles(a)),
        DocxCommand::ExportPdf(a) => ("docx export-pdf", export_pdf_cmd::run(a)),
        DocxCommand::Layout(a) => ("docx layout", layout_cmd::run(a)),
        DocxCommand::NewFromTemplate(a) => ("docx new-from-template", template_cmd::run(a, true)),
    }
}

fn xlsx_command(command: &XlsxCommand) -> Outcome {
    match command {
        XlsxCommand::Inspect(a) => (
            "xlsx inspect",
            inspect_cmd::run(Kind::Xlsx, &a.file, a.hidden),
        ),
        XlsxCommand::Outline(a) => ("xlsx outline", read_cmd::xlsx_outline(a)),
        XlsxCommand::Cells(a) => ("xlsx cells", read_cmd::xlsx_cells(a)),
        XlsxCommand::Formulas(a) => ("xlsx formulas", read_cmd::xlsx_formulas(a)),
        XlsxCommand::Styles(a) => ("xlsx styles", read_cmd::xlsx_styles(a)),
        XlsxCommand::Recalc(a) => ("xlsx recalc", read_cmd::xlsx_recalc(a)),
        XlsxCommand::Eval(a) => ("xlsx eval", eval_cmd::run(a)),
        XlsxCommand::NewFromTemplate(a) => ("xlsx new-from-template", template_cmd::run(a, false)),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    let (name, result) = match &cli.command {
        Command::Inspect { file } => ("inspect", run_inspect(file)),
        Command::Docx { command } => docx_command(command),
        Command::Xlsx { command } => xlsx_command(command),
        Command::Commands(a) => ("commands", commands_cmd::list(a)),
        Command::Run(a) => ("run", commands_cmd::run(a)),
        Command::Share { command } => ("share", share_cmd::run(command)),
        Command::Apply {
            commands,
            input,
            output,
            dry_run,
            allow_destructive,
            roots,
        } => {
            let opts = apply_cmd::Options {
                input: input.clone(),
                output: output.clone(),
                dry_run: *dry_run,
                allow_destructive: *allow_destructive,
                roots: roots.clone(),
            };
            ("apply", apply_cmd::run_file(commands, &opts))
        }
        Command::Mcp {
            roots,
            audit_log,
            allow_edit,
            grants,
            allow_destructive,
            confirm,
            include_hidden,
            timeout_secs,
        } => {
            if json {
                eprintln!("error: mcp speaks JSON-RPC on stdout; --json does not apply");
                return ExitCode::from(2);
            }
            let scopes: BTreeSet<Scope> = grants.iter().filter_map(|g| Scope::parse(g)).collect();
            if *allow_destructive && !*allow_edit && !scopes.contains(&Scope::Edit) {
                eprintln!("error: --allow-destructive needs --allow-edit or --grant edit");
                return ExitCode::from(2);
            }
            let run = mcp_cmd::run(mcp_cmd::Options {
                roots: roots.clone(),
                audit_log: audit_log.clone(),
                allow_edit: *allow_edit,
                allow_destructive: *allow_destructive,
                scopes,
                confirm_terminal: confirm == "terminal",
                include_hidden: *include_hidden,
                deadline: std::time::Duration::from_secs(*timeout_secs),
            });
            return match run {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        Command::Roundtrip { files, .. } => ("roundtrip", run_roundtrip(files)),
    };
    finish(json, name, result)
}
