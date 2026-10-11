//! `open365 commands` lists the command catalogue and `open365 run <name>` runs one command.
//! Both come from `open365_mcp::registry`, the table the MCP server lists too, so a command
//! added to the command layer shows up here and in MCP together (a test enforces it).

use std::path::PathBuf;

use clap::Args as ClapArgs;
use open365_commands::{Command, CommandLog, PdfOptions, Target};
use open365_mcp::registry;
use serde_json::{Map, Value, json};

use crate::apply_cmd::{self, Options, effect_name, gate_write, read_small};
use crate::output::Report;
use crate::{CliError, read};

#[derive(Debug, ClapArgs)]
pub(crate) struct ListArgs {
    /// Only commands for this kind of document.
    #[arg(long, value_parser = ["docx", "xlsx"])]
    domain: Option<String>,
}

#[derive(Debug, ClapArgs)]
pub(crate) struct RunArgs {
    /// Command name, as listed by `open365 commands` (for example `xlsx_set_cell_value`).
    name: String,
    /// Input .docx, .docm, .xlsx or .xlsm file.
    input: PathBuf,
    /// The command's arguments as a JSON object (the `command` field is added for you).
    #[arg(long, conflicts_with = "args_file")]
    args: Option<String>,
    /// Read the arguments from a JSON file.
    #[arg(long)]
    args_file: Option<PathBuf>,
    /// Output file. Not used by `save` and `export_pdf`, which take `path` in their arguments.
    /// Under `--root` it must lie inside a root. Replacing an existing file needs
    /// `--allow-destructive`.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Run in memory, report, and write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Permit destructive and egress commands, and replacing an existing file.
    #[arg(long)]
    allow_destructive: bool,
    /// Fence `save`, `export_pdf` and `-o` to these folders (repeatable): paths are resolved
    /// with symlinks followed and must lie inside one; relative paths resolve against the first
    /// root, and the file is written at the resolved path. Without it any path is allowed, as
    /// the user owns the process.
    #[arg(long = "root")]
    roots: Vec<PathBuf>,
}

pub(crate) fn list(args: &ListArgs) -> Result<Report, CliError> {
    let mut data = registry::catalogue_json();
    if let Some(domain) = &args.domain
        && let Some(list) = data["commands"].as_array_mut()
    {
        list.retain(|c| c["domain"] == *domain || c["domain"] == "any");
        data["count"] = json!(list.len());
    }
    let mut human = String::new();
    for c in data["commands"].as_array().into_iter().flatten() {
        human.push_str(&format!(
            "{:<26} {:<5} {:<12} {}\n",
            c["name"].as_str().unwrap_or(""),
            c["domain"].as_str().unwrap_or(""),
            c["example_effect"].as_str().unwrap_or("-"),
            c["summary"].as_str().unwrap_or("")
        ));
    }
    human.push_str(&format!("{} commands\n", data["count"]));
    Ok(Report::new(data, human))
}

fn command_from(args: &RunArgs) -> Result<Command, CliError> {
    let spec = registry::find(&args.name).ok_or_else(|| {
        CliError::Other(format!(
            "unknown command {}; `open365 commands` lists them",
            args.name
        ))
    })?;
    let text = match (&args.args, &args.args_file) {
        (Some(t), _) => t.clone(),
        (None, Some(f)) => read_small(f)?,
        (None, None) => "{}".to_owned(),
    };
    let mut object: Map<String, Value> = match serde_json::from_str(&text) {
        Ok(Value::Object(o)) => o,
        Ok(_) => return Err(CliError::Other("--args must be a JSON object".to_owned())),
        Err(e) => return Err(CliError::Other(format!("--args: {e}"))),
    };
    if object
        .get("command")
        .is_some_and(|c| c.as_str() != Some(spec.name))
    {
        return Err(CliError::Other(format!(
            "--args names a different command than {}",
            spec.name
        )));
    }
    object.insert("command".to_owned(), json!(spec.name));
    serde_json::from_value(Value::Object(object))
        .map_err(|e| CliError::Other(format!("{}: invalid arguments: {e}", spec.name)))
}

pub(crate) fn run(args: &RunArgs) -> Result<Report, CliError> {
    let command = command_from(args)?;
    let options = Options {
        input: args.input.clone(),
        output: args.output.clone(),
        dry_run: args.dry_run,
        allow_destructive: args.allow_destructive,
        roots: args.roots.clone(),
    };
    match &command {
        Command::Save { .. } | Command::ExportPdf { .. } => {
            run_write(&command, &options, &args.roots)
        }
        _ => apply_cmd::execute(&CommandLog(vec![command]), &options),
    }
}

/// `save` and `export_pdf` write to the path in their arguments, under the same policy as
/// every other command.
fn run_write(command: &Command, options: &Options, roots: &[PathBuf]) -> Result<Report, CliError> {
    let bytes = read(&options.input)?;
    let mut target = if matches!(command, Command::ExportPdf { .. }) {
        Target::open_docx(&bytes)
    } else {
        Target::open_path(&options.input)
    }
    .map_err(|e| CliError::Other(format!("{}: {e}", options.input.display())))?;
    // The file is written at the path the gate resolved, not at the argument as typed.
    let gated = gate_write(command, &target, options.allow_destructive, roots, 1)?;
    let command = &gated.command;
    target.restrict_writes_to(&gated.roots);
    target.set_pdf_options(PdfOptions::default());
    let effect = command.effect_in(&target);
    if options.dry_run {
        let data = json!({
            "input": options.input.display().to_string(),
            "dry_run": true,
            "commands": [{"seq": 1, "command": command.name(), "effect": effect_name(effect), "status": "dry_run_ok"}],
        });
        return Ok(Report::new(
            data,
            format!("{} {}\n", command.name(), effect_name(effect)),
        ));
    }
    let out = target
        .apply(command.clone())
        .map_err(|e| CliError::Other(format!("{}: {e}", command.name())))?;
    let data = json!({
        "input": options.input.display().to_string(),
        "dry_run": false,
        "commands": [{
            "seq": 1,
            "command": command.name(),
            "effect": effect_name(out.effect),
            "status": "applied",
            "bytes_written": out.bytes_written,
            "pages": out.pages,
            "notes": out.notes,
        }],
    });
    Ok(Report::new(
        data,
        format!(
            "{} {} ({} bytes)\n",
            command.name(),
            effect_name(out.effect),
            out.bytes_written
        ),
    ))
}
