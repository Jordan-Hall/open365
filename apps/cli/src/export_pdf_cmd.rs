//! `open365 docx export-pdf <in> -o <out>`: layout and PDF export (ADR-004).

use std::path::PathBuf;

use clap::Args as ClapArgs;
use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_pdf::{PdfMetadata, export_pdf};
use open365_text::{FontLibrary, FontMatch};

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
}

pub(crate) fn run(args: &Args) -> Result<(), CliError> {
    let bytes = read(&args.file)?;
    let doc = Document::open(&bytes, &Limits::default())
        .map_err(|e| CliError::Other(format!("{}: {e}", args.file.display())))?;
    let mut fonts = FontLibrary::bundled();
    let mut layout = layout_document(&doc, &fonts, LayoutOptions::default());
    if args.system_fonts {
        let missing: Vec<String> = layout
            .fonts
            .iter()
            .filter(|f| f.matched == FontMatch::Fallback)
            .map(|f| f.requested.clone())
            .collect();
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(String::as_str).collect();
            fonts.add_system_families(&names, &[]);
            layout = layout_document(&doc, &fonts, LayoutOptions::default());
        }
    }
    for f in layout
        .fonts
        .iter()
        .filter(|f| f.matched == FontMatch::Fallback)
    {
        eprintln!(
            "warning: font \"{}\" is not available; the default font is used",
            f.requested
        );
    }
    let meta = PdfMetadata {
        title: args.title.clone(),
        author: args.author.clone(),
        subject: args.subject.clone(),
        keywords: args.keywords.clone(),
        language: args.language.clone(),
        ..PdfMetadata::default()
    };
    let out = export_pdf(&doc, &layout, &fonts, &meta)
        .map_err(|e| CliError::Other(format!("{}: {e}", args.file.display())))?;
    for w in &out.warnings {
        eprintln!("warning: {w}");
    }
    std::fs::write(&args.output, &out.bytes).map_err(|e| CliError::Io(args.output.clone(), e))?;
    println!(
        "wrote {} ({} pages, {} bytes)",
        args.output.display(),
        layout.pages.len(),
        out.bytes.len()
    );
    Ok(())
}
