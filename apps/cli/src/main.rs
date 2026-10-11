//! `open365` CLI: `docx|xlsx inspect`, `apply`, `roundtrip --check`, `mcp` and `inspect`.

mod apply_cmd;
mod eval_cmd;
mod export_pdf_cmd;
mod inspect_cmd;
mod layout_cmd;
mod mcp_cmd;
mod template_cmd;

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use open365_opc::OpcError;
use open365_testkit::{inspect, roundtrip};

use inspect_cmd::Kind;

#[derive(Debug, Parser)]
#[command(name = "open365", version)]
struct Cli {
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
    /// Machine-readable output.
    #[arg(long)]
    json: bool,
    file: PathBuf,
}

#[derive(Debug, Subcommand)]
enum XlsxCommand {
    /// Print parts, relationships and content types.
    Inspect(InspectArgs),
    /// Evaluate a formula against the workbook and print the result as JSON. Read-only:
    /// nothing is saved.
    Eval(eval_cmd::Args),
    /// Make a new .xlsx or .xlsm from a .xltx or .xltm template. A VBA project is kept and
    /// never run. Refuses to replace a file unless --overwrite is given.
    NewFromTemplate(template_cmd::Args),
}

#[derive(Debug, Subcommand)]
enum DocxCommand {
    /// Print parts, relationships and content types.
    Inspect(InspectArgs),
    /// Lay the document out and write it as a tagged PDF 1.7 file.
    ///
    /// Fonts are the bundled ones (Calibri -> Carlito, Cambria -> Caladea) unless
    /// --system-fonts is given. No author or date is written unless asked for.
    ExportPdf(export_pdf_cmd::Args),
    /// Print pages, lines and table cell boxes from the layout engine (read-only).
    Layout(layout_cmd::Args),
    /// Make a new .docx or .docm from a .dotx or .dotm template. A VBA project is kept and
    /// never run. Refuses to replace a file unless --overwrite is given.
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
        /// Permit destructive and egress commands.
        #[arg(long)]
        allow_destructive: bool,
    },
    /// Serve document tools to an agent over stdio (MCP). Read-only unless --allow-edit.
    ///
    /// Every tool call appends one JSON line to --audit-log. There is no native confirmation
    /// prompt yet, so destructive and egress effects are refused unless --allow-destructive.
    Mcp {
        /// Folder agents may open and save in (repeatable). Relative paths use the first.
        #[arg(long = "root", required = true)]
        roots: Vec<PathBuf>,
        /// Audit log file (appended; one JSON line per call).
        #[arg(long)]
        audit_log: PathBuf,
        /// Permit apply_commands and save_as.
        #[arg(long)]
        allow_edit: bool,
        /// Permit destructive and egress commands and overwriting other files.
        #[arg(long, requires = "allow_edit")]
        allow_destructive: bool,
        /// Include hidden text, hidden sheets, rows and columns, and hidden-content findings.
        #[arg(long)]
        include_hidden: bool,
        /// Longest a layout_summary, export_pdf or evaluate_formula call may run, in seconds.
        /// A call that times out is cancelled: layout, the PDF writer and recalculation stop at
        /// their next check, and an export that timed out leaves no file.
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
    Other(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Self::Opc(p, e) => write!(f, "{}: {e}", p.display()),
            Self::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CliError {}

pub(crate) fn read(path: &Path) -> Result<Vec<u8>, CliError> {
    std::fs::read(path).map_err(|e| CliError::Io(path.to_owned(), e))
}

fn run_inspect(file: &Path) -> Result<(), CliError> {
    let info = inspect(&read(file)?).map_err(|e| CliError::Opc(file.to_owned(), e))?;
    println!(
        "{:>10}  {:<8}  {:<60}  PART",
        "SIZE", "METHOD", "CONTENT-TYPE"
    );
    for p in &info.parts {
        println!(
            "{:>10}  {:<8}  {:<60}  {}",
            p.size,
            format!("{:?}", p.compression),
            p.content_type.as_deref().unwrap_or("-"),
            p.name
        );
    }
    println!(
        "{} parts, {} relationships, {} integrity issues",
        info.parts.len(),
        info.relationship_count,
        info.integrity_issues.len()
    );
    for issue in &info.integrity_issues {
        println!("  issue: {issue:?}");
    }
    Ok(())
}

/// Prints one line for `file`; returns whether it passed.
fn check_file(file: &Path) -> bool {
    let name = file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let result =
        read(file).and_then(|b| roundtrip(&b).map_err(|e| CliError::Opc(file.to_owned(), e)));
    match result {
        Ok(r) if r.is_lossless() => {
            println!("PASS {name} parts={}", r.part_count);
            true
        }
        Ok(r) => {
            println!("FAIL {name}: {}", r.describe());
            false
        }
        Err(e) => {
            println!("FAIL {name}: {e}");
            false
        }
    }
}

fn finish(result: Result<(), CliError>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn xlsx_command(command: &XlsxCommand) -> ExitCode {
    match command {
        XlsxCommand::Inspect(a) => finish(inspect_cmd::run(Kind::Xlsx, &a.file, a.hidden, a.json)),
        XlsxCommand::Eval(a) => finish(eval_cmd::run(a)),
        XlsxCommand::NewFromTemplate(a) => finish(template_cmd::run(a, false)),
    }
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Inspect { file } => finish(run_inspect(&file)),
        Command::Docx { command } => match command {
            DocxCommand::Inspect(a) => {
                finish(inspect_cmd::run(Kind::Docx, &a.file, a.hidden, a.json))
            }
            DocxCommand::ExportPdf(args) => finish(export_pdf_cmd::run(&args)),
            DocxCommand::Layout(args) => finish(layout_cmd::run(&args)),
            DocxCommand::NewFromTemplate(args) => finish(template_cmd::run(&args, true)),
        },
        Command::Xlsx { command } => xlsx_command(&command),
        Command::Apply {
            commands,
            input,
            output,
            dry_run,
            allow_destructive,
        } => {
            let opts = apply_cmd::Options {
                commands,
                input,
                output,
                dry_run,
                allow_destructive,
            };
            match apply_cmd::run(&opts) {
                Ok(apply_cmd::Outcome::Done) => ExitCode::SUCCESS,
                Ok(apply_cmd::Outcome::Refused) => {
                    eprintln!("error: refused; nothing was applied or written");
                    ExitCode::from(3)
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Mcp {
            roots,
            audit_log,
            allow_edit,
            allow_destructive,
            include_hidden,
            timeout_secs,
        } => finish(mcp_cmd::run(mcp_cmd::Options {
            roots,
            audit_log,
            allow_edit,
            allow_destructive,
            include_hidden,
            deadline: std::time::Duration::from_secs(timeout_secs),
        })),
        Command::Roundtrip { files, .. } => {
            let results: Vec<bool> = files.iter().map(|f| check_file(f)).collect();
            if results.iter().all(|&ok| ok) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
    }
}
