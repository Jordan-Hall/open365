//! `open365 docx export-pdf <in> -o <out>`: layout and PDF export (ADR-004).

use std::path::PathBuf;

use clap::Args as ClapArgs;
use open365_commands::{Command, CommandError, PdfMetadata, PdfOptions, Target};

use crate::{CliError, read};

#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Input .docx file.
    file: PathBuf,
    /// Output .pdf file.
    #[arg(short, long)]
    output: PathBuf,
    /// Document title (also shown in the viewer's title bar).
    #[arg(long)]
    title: Option<String>,
    /// Document author. Empty unless given.
    #[arg(long)]
    author: Option<String>,
    #[arg(long)]
    subject: Option<String>,
    #[arg(long)]
    keywords: Option<String>,
    /// BCP 47 language of the document, for screen readers (for example en-GB).
    #[arg(long)]
    language: Option<String>,
    /// Use fonts installed on this machine for families the bundled fonts cannot stand in for.
    /// The result then depends on the machine.
    #[arg(long)]
    system_fonts: bool,
    /// Replace the output file if it exists. Without this an existing file is an error.
    #[arg(long)]
    overwrite: bool,
}

pub(crate) fn run(args: &Args) -> Result<(), CliError> {
    let bytes = read(&args.file)?;
    let mut target = Target::open_docx(&bytes)
        .map_err(|e| CliError::Other(format!("{}: {e}", args.file.display())))?;
    target.set_pdf_options(PdfOptions {
        metadata: PdfMetadata {
            title: args.title.clone(),
            author: args.author.clone(),
            subject: args.subject.clone(),
            keywords: args.keywords.clone(),
            language: args.language.clone(),
            ..PdfMetadata::default()
        },
        system_fonts: args.system_fonts,
        ..PdfOptions::default()
    });
    // The export is the `export_pdf` command (effect export): without --overwrite the output
    // is created exclusively and an existing file is an error.
    let out = target
        .apply(Command::ExportPdf {
            path: args.output.clone(),
            overwrite: args.overwrite,
        })
        .map_err(|e| match e {
            CommandError::Exists { .. } => CliError::Other(format!(
                "{} exists; pass --overwrite to replace it",
                args.output.display()
            )),
            e => CliError::Other(format!("{}: {e}", args.file.display())),
        })?;
    for w in &out.notes {
        eprintln!("warning: {w}");
    }
    println!(
        "wrote {} ({} pages, {} bytes)",
        args.output.display(),
        out.pages,
        out.bytes_written
    );
    Ok(())
}
