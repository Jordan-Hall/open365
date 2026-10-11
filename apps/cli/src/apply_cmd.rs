//! `open365 apply` and `open365 run`: run commands through the command layer (ADR-002) and
//! nothing else. `apply` takes a command file; `run` takes one command by name.

use std::io::Read;
use std::path::{Path, PathBuf};

use open365_commands::{Command, CommandLog, EffectClass, Target};
use open365_mcp::Fence;
use open365_mcp::policy::{assess_in, assess_sequence, cli_decision};
use serde_json::{Value, json};

use crate::output::Report;
use crate::{CliError, read};

/// Command files above this size are refused; they are untrusted input.
const MAX_COMMAND_FILE: u64 = 16 * 1024 * 1024;

pub struct Options {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub dry_run: bool,
    pub allow_destructive: bool,
    /// Folders the output (`-o`) must lie in; empty means unrestricted.
    pub roots: Vec<PathBuf>,
}

pub(crate) fn effect_name(effect: EffectClass) -> String {
    serde_json::to_value(effect)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{effect:?}").to_lowercase())
}

/// One JSON audit line per command on stderr.
pub(crate) fn audit(line: &Value) {
    eprintln!("{line}");
}

/// A write the shared gate allowed: the command to run, with its path resolved, and the
/// canonical roots the save must stay inside (empty when no `--root` was given).
pub(crate) struct Gated {
    pub command: Command,
    pub roots: Vec<PathBuf>,
}

/// The one gate every CLI write passes (`save`, `export_pdf`, `-o`, `new-from-template`):
/// 1. with `--root` folders, the path must resolve inside one of them (symlinks followed,
///    `..` and device names refused) and the command carries the *resolved* path, so what is
///    written is what was checked, not the argument as typed;
/// 2. the shared policy classifies it against `target` and, if it replaces an existing file or
///    destroys content, refuses unless `allow_destructive`.
///
/// A refusal writes one audit line to stderr and returns `CliError::Refused`; nothing was
/// written. `seq` numbers the audit line.
pub(crate) fn gate_write(
    command: &Command,
    target: &Target,
    allow_destructive: bool,
    roots: &[PathBuf],
    seq: usize,
) -> Result<Gated, CliError> {
    let refuse = |reason: String| {
        audit(&json!({
            "seq": seq,
            "command": command.name(),
            "effect": effect_name(command.effect_in(target)),
            "status": "refused",
            "reason": reason,
        }));
        Err(CliError::Refused(format!(
            "refused; nothing was written: {reason}"
        )))
    };
    let mut command = command.clone();
    let mut canonical = Vec::new();
    if !roots.is_empty() {
        let (Command::Save { path } | Command::ExportPdf { path, .. }) = &mut command else {
            return Ok(Gated {
                command,
                roots: canonical,
            });
        };
        let fence = Fence::new(roots).map_err(CliError::Other)?;
        match fence.resolve_new(&path.to_string_lossy()) {
            Ok(resolved) => *path = resolved,
            Err(denied) => return refuse(format!("{}: {}", path.display(), denied.0)),
        }
        canonical = fence.roots().to_vec();
    }
    match cli_decision(&assess_in(&command, target), allow_destructive) {
        Ok(()) => Ok(Gated {
            command,
            roots: canonical,
        }),
        Err(reason) => refuse(reason),
    }
}

/// Reads a text file of at most `MAX_COMMAND_FILE` bytes.
pub(crate) fn read_small(path: &Path) -> Result<String, CliError> {
    let io = |e| CliError::Io(path.to_owned(), e);
    let file = std::fs::File::open(path).map_err(io)?;
    let mut text = String::new();
    file.take(MAX_COMMAND_FILE + 1)
        .read_to_string(&mut text)
        .map_err(io)?;
    if text.len() as u64 > MAX_COMMAND_FILE {
        return Err(CliError::Other(format!(
            "{}: command file larger than {MAX_COMMAND_FILE} bytes",
            path.display()
        )));
    }
    Ok(text)
}

fn load_log(path: &Path) -> Result<CommandLog, CliError> {
    CommandLog::from_json(&read_small(path)?)
        .map_err(|e| CliError::Other(format!("{}: {e}", path.display())))
}

fn is_supported(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("docx" | "docm" | "xlsx" | "xlsm")
    )
}

/// `open365 apply <commands.json> <input> [-o out]`.
pub fn run_file(commands: &Path, opts: &Options) -> Result<Report, CliError> {
    execute(&load_log(commands)?, opts)
}

/// Runs the commands in memory, then saves to the output unless `dry_run`.
pub fn execute(log: &CommandLog, opts: &Options) -> Result<Report, CliError> {
    if !is_supported(&opts.input) {
        return Err(CliError::Other(format!(
            "{}: input must be a .docx, .docm, .xlsx or .xlsm file",
            opts.input.display()
        )));
    }
    let bytes = read(&opts.input)?;
    let open = || {
        Target::open_path_bytes(&opts.input, &bytes)
            .map_err(|e| CliError::Other(format!("{}: {e}", opts.input.display())))
    };
    let mut target = open()?;

    // Classify everything first so a refusal leaves the document and the output untouched.
    // Each command is classified against the state the earlier ones leave (a scratch copy), so
    // overwriting a non-empty cell is destructive even when a batch reaches it indirectly.
    let mut human = String::new();
    let mut refused = Vec::new();
    let mut scratch = open()?;
    let steps = assess_sequence(&mut scratch, log.commands());
    for (i, (command, step)) in log.commands().iter().zip(&steps).enumerate() {
        let effect = step.risk.effect;
        human.push_str(&format!(
            "{:>3} {:<24} {}\n",
            i + 1,
            command.name(),
            effect_name(effect)
        ));
        let save = matches!(command, Command::Save { .. } | Command::ExportPdf { .. });
        let blocked = save || cli_decision(&step.risk, opts.allow_destructive).is_err();
        if blocked {
            let reason = if save {
                "save and export_pdf are not allowed in a command file; the output path is set with -o (use docx export-pdf for a PDF)"
            } else {
                "refused: destructive/egress command needs --allow-destructive"
            };
            audit(&json!({
                "seq": i + 1,
                "command": command.name(),
                "effect": effect_name(effect),
                "status": "refused",
                "reason": reason,
            }));
            refused.push(format!("{} ({})", i + 1, command.name()));
        }
    }
    // The output is gated up front too: fenced to --root and, if it replaces a file, refused
    // without --allow-destructive. Nothing has run yet, so a refusal changes nothing.
    let save = if opts.dry_run {
        None
    } else if let Some(output) = &opts.output {
        let command = Command::Save {
            path: output.clone(),
        };
        match gate_write(
            &command,
            &target,
            opts.allow_destructive,
            &opts.roots,
            log.commands().len() + 1,
        ) {
            Ok(gated) => Some(gated),
            Err(_) => {
                refused.push("-o".to_owned());
                None
            }
        }
    } else {
        return Err(CliError::Other("-o <out> is required".to_owned()));
    };
    if !refused.is_empty() {
        return Err(CliError::Refused(format!(
            "refused; nothing was applied or written: {} needs --allow-destructive, lies outside --root, or is not allowed in a command file",
            refused.join(", ")
        )));
    }

    let mut results = Vec::new();
    for (i, command) in log.commands().iter().enumerate() {
        let name = command.name();
        match target.apply(command.clone()) {
            Ok(out) => {
                let line = json!({
                    "seq": i + 1,
                    "command": name,
                    "effect": effect_name(out.effect),
                    "status": if opts.dry_run { "dry_run_ok" } else { "applied" },
                    "changed_cells": out.changed_cells.len(),
                    "notes": out.notes,
                });
                audit(&line);
                results.push(line);
            }
            Err(e) => {
                audit(&json!({
                    "seq": i + 1,
                    "command": name,
                    "effect": effect_name(steps[i].risk.effect),
                    "status": "error",
                    "error": e.to_string(),
                }));
                return Err(CliError::Other(format!("command {} ({name}): {e}", i + 1)));
            }
        }
    }

    let mut data = json!({
        "input": opts.input.display().to_string(),
        "dry_run": opts.dry_run,
        "commands": results,
        "output": Value::Null,
        "bytes_written": Value::Null,
    });
    if opts.dry_run {
        return Ok(Report::new(data, human));
    }
    let Some(Gated {
        command: save,
        roots,
    }) = save
    else {
        return Err(CliError::Other("-o <out> is required".to_owned()));
    };
    let Command::Save { path: output } = &save else {
        return Err(CliError::Other("-o <out> is required".to_owned()));
    };
    let output = output.clone();
    target.restrict_writes_to(&roots);
    let effect = save.effect_in(&target);
    match target.apply(save) {
        Ok(out) => {
            audit(&json!({
                "seq": log.commands().len() + 1,
                "command": "save",
                "effect": effect_name(out.effect),
                "status": "applied",
                "path": output.display().to_string(),
                "bytes_written": out.bytes_written,
            }));
            data["output"] = json!(output.display().to_string());
            data["bytes_written"] = json!(out.bytes_written);
            Ok(Report::new(data, human))
        }
        Err(e) => {
            audit(&json!({
                "seq": log.commands().len() + 1,
                "command": "save",
                "effect": effect_name(effect),
                "status": "error",
                "error": e.to_string(),
            }));
            Err(CliError::Other(format!("save {}: {e}", output.display())))
        }
    }
}
