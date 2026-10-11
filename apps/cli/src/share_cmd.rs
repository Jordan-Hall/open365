//! `open365 share ...`: the Stage 0 sharing commands (ADR-013) on a document file.
//!
//! Each subcommand builds one `share_*` command and runs it through the shared write gate
//! (`apply_cmd::gate_write`): paths are fenced to `--root`, and a command that replaces a file or
//! hands data to the operating system (`os-sheet`) is refused without `--allow-destructive`. The
//! input file is never written.

use std::path::{Path, PathBuf};

use clap::{Args as ClapArgs, Subcommand};
use open365_commands::{Command, HiddenKind, Target};
use serde_json::json;

use crate::apply_cmd::{audit, effect_name, gate_write};
use crate::output::Report;
use crate::{CliError, read};

#[derive(Debug, ClapArgs)]
pub(crate) struct CopyArgs {
    /// The .docx or .xlsx file to share (not modified).
    input: PathBuf,
    /// Where to write the copy. Without it the copy goes to a new temporary folder.
    #[arg(long)]
    to: Option<PathBuf>,
    /// Replace the target if it exists (also needs --allow-destructive).
    #[arg(long)]
    overwrite: bool,
    /// Permit replacing a file.
    #[arg(long)]
    allow_destructive: bool,
    /// Fence the copy to these folders (repeatable).
    #[arg(long = "root")]
    roots: Vec<PathBuf>,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct InspectArgs {
    /// The .docx or .xlsx file to inspect.
    input: PathBuf,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct SanitizeArgs {
    #[command(flatten)]
    copy: CopyArgs,
    /// What to remove (repeatable): comments, tracked-changes, hidden-text, properties,
    /// personal-info. Tracked changes are accepted. Hidden sheets, rows and columns of a
    /// workbook cannot be removed.
    #[arg(long = "remove", required = true,
        value_parser = ["comments", "tracked-changes", "hidden-text", "properties", "personal-info"])]
    remove: Vec<String>,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct OsSheetArgs {
    /// The document the file belongs to.
    input: PathBuf,
    /// The file to hand to the operating system share sheet.
    file: PathBuf,
    /// Permit the egress request.
    #[arg(long)]
    allow_destructive: bool,
    /// Fence the file to these folders (repeatable).
    #[arg(long = "root")]
    roots: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ShareCommand {
    /// Write a copy of the document for an attachment (effect export).
    SendCopy(CopyArgs),
    /// Write the document as a PDF to send (effect export; DOCX only).
    SendPdf(CopyArgs),
    /// List comments, tracked changes, hidden text, properties and author names (effect read).
    Inspect(InspectArgs),
    /// Write a copy without the chosen content; the original is unchanged (effect export).
    SanitizeCopy(SanitizeArgs),
    /// Build the request for the operating system share sheet (effect egress; sends nothing).
    OsSheet(OsSheetArgs),
}

fn kind(name: &str) -> HiddenKind {
    match name {
        "comments" => HiddenKind::Comments,
        "tracked-changes" => HiddenKind::TrackedChanges,
        "hidden-text" => HiddenKind::HiddenText,
        "properties" => HiddenKind::Properties,
        _ => HiddenKind::PersonalInfo,
    }
}

fn run_command(
    input: &Path,
    command: Command,
    allow_destructive: bool,
    roots: &[PathBuf],
) -> Result<Report, CliError> {
    let bytes = read(input)?;
    let mut target = Target::open_path_bytes(input, &bytes)
        .map_err(|e| CliError::Other(format!("{}: {e}", input.display())))?;
    let gated = gate_write(&command, &target, allow_destructive, roots, 1)?;
    target.restrict_writes_to(&gated.roots);
    let name = gated.command.name();
    let out = target
        .apply(gated.command.clone())
        .map_err(|e| CliError::Other(format!("{name}: {e}")))?;
    audit(&json!({
        "seq": 1,
        "command": name,
        "effect": effect_name(out.effect),
        "status": "applied",
    }));
    let data = json!({
        "input": input.display().to_string(),
        "command": name,
        "effect": effect_name(out.effect),
        "written_to": out.saved_to.as_ref().map(|p| p.display().to_string()),
        "bytes_written": out.bytes_written,
        "pages": out.pages,
        "notes": out.notes,
        "report": out.report,
    });
    let mut human = format!("{name} {}", effect_name(out.effect));
    if let Some(p) = &out.saved_to {
        human.push_str(&format!(
            ": wrote {} ({} bytes)",
            p.display(),
            out.bytes_written
        ));
    }
    human.push('\n');
    for note in &out.notes {
        human.push_str(&format!("  {note}\n"));
    }
    if name == "share_os_sheet"
        && let Some(r) = &out.report
    {
        human.push_str(&format!("  {r}\n"));
    }
    Ok(Report::new(data, human))
}

pub(crate) fn run(command: &ShareCommand) -> Result<Report, CliError> {
    match command {
        ShareCommand::SendCopy(a) => run_command(
            &a.input,
            Command::ShareSendCopy {
                path: a.to.clone(),
                overwrite: a.overwrite,
            },
            a.allow_destructive,
            &a.roots,
        ),
        ShareCommand::SendPdf(a) => run_command(
            &a.input,
            Command::ShareSendPdf {
                path: a.to.clone(),
                overwrite: a.overwrite,
            },
            a.allow_destructive,
            &a.roots,
        ),
        ShareCommand::Inspect(a) => run_command(&a.input, Command::ShareInspect, false, &[]),
        ShareCommand::SanitizeCopy(a) => run_command(
            &a.copy.input,
            Command::ShareSanitizeCopy {
                path: a.copy.to.clone(),
                overwrite: a.copy.overwrite,
                remove: a.remove.iter().map(|k| kind(k)).collect(),
            },
            a.copy.allow_destructive,
            &a.copy.roots,
        ),
        ShareCommand::OsSheet(a) => run_command(
            &a.input,
            Command::ShareOsSheet {
                path: a.file.clone(),
            },
            a.allow_destructive,
            &a.roots,
        ),
    }
}
