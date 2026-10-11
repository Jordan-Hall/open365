//! `open365 docx|xlsx new-from-template <template> -o <out>`: a new document from a template.
//! The change goes through the command layer (`Save` is an export) and the shared write gate
//! (`apply_cmd::gate_write`): `--root` fences the output, which is written at the resolved
//! path, replacing a file needs `--overwrite` and `--allow-destructive`, and one audit line goes to stderr. A VBA
//! project is kept and never run.

use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use open365_commands::{Command, Target, template_output_extension};

use crate::apply_cmd::{audit, effect_name, gate_write};
use crate::output::Report;
use crate::{CliError, read};
use serde_json::json;

#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// The template: .dotx or .dotm (docx), .xltx or .xltm (xlsx).
    template: PathBuf,
    /// The new file: .docx, .docm, .xlsx or .xlsm to match the template.
    #[arg(short, long)]
    output: PathBuf,
    /// Replace the output file if it exists. Also needs `--allow-destructive`.
    #[arg(long)]
    overwrite: bool,
    /// Permit replacing an existing output file (with `--overwrite`).
    #[arg(long)]
    allow_destructive: bool,
    /// Fence the output to these folders (repeatable): the path is resolved with symlinks
    /// followed, must lie inside one, and relative paths resolve against the first.
    #[arg(long = "root")]
    roots: Vec<PathBuf>,
}

fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// `want_docx` is the subcommand the user chose.
pub(crate) fn run(args: &Args, want_docx: bool) -> Result<Report, CliError> {
    let t_ext = ext(&args.template);
    let out_ext = template_output_extension(&t_ext)
        .filter(|e| e.starts_with("doc") == want_docx)
        .ok_or_else(|| {
            CliError::Other(if want_docx {
                "docx new-from-template needs a .dotx or .dotm template".to_owned()
            } else {
                "xlsx new-from-template needs a .xltx or .xltm template".to_owned()
            })
        })?;
    if ext(&args.output) != out_ext {
        return Err(CliError::Other(format!(
            "a .{t_ext} template makes a .{out_ext} file; give an output ending in .{out_ext}"
        )));
    }
    if args.output.exists() && !args.overwrite {
        return Err(CliError::Other(format!(
            "{} exists; pass --overwrite to replace it",
            args.output.display()
        )));
    }
    let bytes = read(&args.template)?;
    let mut target = Target::open_template(&bytes, &t_ext)
        .map_err(|e| CliError::Other(format!("{}: {e}", args.template.display())))?;
    // Replacing a file needs `--overwrite` (checked above) and `--allow-destructive`, as for
    // `export-pdf`; the fence, the classification and the audit line are the shared gate's.
    let gated = gate_write(
        &Command::Save {
            path: args.output.clone(),
        },
        &target,
        args.allow_destructive,
        &args.roots,
        1,
    )?;
    target.restrict_writes_to(&gated.roots);
    let Command::Save { path: output } = &gated.command else {
        return Err(CliError::Other("internal error: not a save".to_owned()));
    };
    let output = output.clone();
    let effect = gated.command.effect_in(&target);
    let out = target.apply(gated.command).map_err(|e| {
        audit(&json!({
            "seq": 1,
            "command": "save",
            "effect": effect_name(effect),
            "status": "error",
            "error": e.to_string(),
        }));
        CliError::Other(e.to_string())
    })?;
    audit(&json!({
        "seq": 1,
        "command": "save",
        "effect": effect_name(out.effect),
        "status": "applied",
        "path": output.display().to_string(),
        "bytes_written": out.bytes_written,
    }));
    let data = json!({
        "output": output.display().to_string(),
        "bytes_written": out.bytes_written,
    });
    Ok(Report::new(
        data,
        format!("wrote {} ({} bytes)", output.display(), out.bytes_written),
    ))
}
