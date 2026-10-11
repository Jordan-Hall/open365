//! `open365 apply`: runs a command file through the command layer (ADR-002) and nothing else.

use std::io::Read;
use std::path::{Path, PathBuf};

use open365_commands::{Command, CommandLog, EffectClass, Target};
use serde_json::{Value, json};

use crate::CliError;

/// Command files above this size are refused; they are untrusted input.
const MAX_COMMAND_FILE: u64 = 16 * 1024 * 1024;

pub struct Options {
    pub commands: PathBuf,
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub dry_run: bool,
    pub allow_destructive: bool,
}

/// How a run ended, for the process exit code.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// At least one command was refused by the effect policy; nothing ran.
    Refused,
}

fn effect_name(effect: EffectClass) -> String {
    serde_json::to_value(effect)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{effect:?}").to_lowercase())
}

fn audit(line: &Value) {
    eprintln!("{line}");
}

fn load_log(path: &Path) -> Result<CommandLog, CliError> {
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
    CommandLog::from_json(&text).map_err(|e| CliError::Other(format!("{}: {e}", path.display())))
}

fn is_supported(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("docx" | "xlsx" | "xlsm")
    )
}

pub fn run(opts: &Options) -> Result<Outcome, CliError> {
    let log = load_log(&opts.commands)?;
    if !is_supported(&opts.input) {
        return Err(CliError::Other(format!(
            "{}: input must be a .docx, .xlsx or .xlsm file",
            opts.input.display()
        )));
    }
    let mut target = Target::open_path(&opts.input)
        .map_err(|e| CliError::Other(format!("{}: {e}", opts.input.display())))?;

    // Classify everything first so a refusal leaves the document and the output untouched.
    let mut refused = false;
    for (i, command) in log.commands().iter().enumerate() {
        let effect = command.effect(target.path());
        println!(
            "{:>3} {:<24} {}",
            i + 1,
            command.name(),
            effect_name(effect)
        );
        let save = matches!(command, Command::Save { .. } | Command::ExportPdf { .. });
        let blocked = save
            || (!opts.allow_destructive
                && matches!(effect, EffectClass::Destructive | EffectClass::Egress));
        if blocked {
            refused = true;
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
        }
    }
    if refused {
        return Ok(Outcome::Refused);
    }

    for (i, command) in log.commands().iter().enumerate() {
        let name = command.name();
        match target.apply(command.clone()) {
            Ok(out) => audit(&json!({
                "seq": i + 1,
                "command": name,
                "effect": effect_name(out.effect),
                "status": if opts.dry_run { "dry_run_ok" } else { "applied" },
                "changed_cells": out.changed_cells.len(),
                "notes": out.notes,
            })),
            Err(e) => {
                audit(&json!({
                    "seq": i + 1,
                    "command": name,
                    "effect": effect_name(command.effect(target.path())),
                    "status": "error",
                    "error": e.to_string(),
                }));
                return Err(CliError::Other(format!("command {} ({name}): {e}", i + 1)));
            }
        }
    }

    if opts.dry_run {
        return Ok(Outcome::Done);
    }
    let Some(output) = &opts.output else {
        return Err(CliError::Other("-o <out> is required".to_owned()));
    };
    let save = Command::Save {
        path: output.clone(),
    };
    let effect = save.effect(target.path());
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
            Ok(Outcome::Done)
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
