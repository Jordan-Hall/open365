//! `open365 docx|xlsx new-from-template <template> -o <out>`: a new document from a template.
//! The change goes through the command layer (`Save` is an export). A VBA project is kept
//! and never run.

use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use open365_commands::{Command, Target, template_output_extension};

use crate::{CliError, read};

#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// The template: .dotx or .dotm (docx), .xltx or .xltm (xlsx).
    template: PathBuf,
    /// The new file: .docx, .docm, .xlsx or .xlsm to match the template.
    #[arg(short, long)]
    output: PathBuf,
    /// Replace the output file if it exists.
    #[arg(long)]
    overwrite: bool,
}

fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// `want_docx` is the subcommand the user chose.
pub(crate) fn run(args: &Args, want_docx: bool) -> Result<(), CliError> {
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
    let out = target
        .apply(Command::Save {
            path: args.output.clone(),
        })
        .map_err(|e| CliError::Other(e.to_string()))?;
    println!(
        "wrote {} ({} bytes)",
        args.output.display(),
        out.bytes_written
    );
    Ok(())
}
