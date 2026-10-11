//! `open365 docx export-pdf <in> -o <out>`: layout and PDF export (ADR-004).
//!
//! The export is the `export_pdf` command (effect export) and goes through the shared policy
//! gate (`open365_mcp::policy`) exactly as `open365 run export_pdf` does: replacing a file needs
//! `--allow-destructive`, `--root` fences the output to folders (the file is written at the
//! resolved path, never at the argument as typed), and one audit line goes to stderr.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::Args as ClapArgs;
use open365_commands::{Command, PdfMetadata};
use open365_docx::Document;
use open365_layout::{Layout, LayoutOptions, layout_document_cancellable};
use open365_mcp::Fence;
use open365_mcp::policy::{assess, cli_decision};
use open365_opc::Limits;
use open365_pdf::{Conformance, PdfOptions, export_pdf_with};
use open365_text::{FontLibrary, FontMatch};
use serde_json::json;

use crate::apply_cmd::{audit, effect_name};
use crate::output::Report;
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
    /// BCP 47 language of the document, for screen readers (for example en-GB). Without it the
    /// document's own default language is written, as Word does.
    #[arg(long, visible_alias = "lang")]
    language: Option<String>,
    /// Take title, author, subject, keywords and dates from the document's own properties (Word's
    /// "Document properties" option). Flags such as --author override single fields. Off by
    /// default so that an export does not leak the author.
    #[arg(long)]
    document_properties: bool,
    /// Write an archival PDF/A file: 1b (PDF 1.4) or 2b (PDF 1.7).
    #[arg(long, value_parser = ["1b", "2b"])]
    pdfa: Option<String>,
    /// Declare PDF/UA-1 (`pdfuaid` in the XMP) and write what the profile needs: a title is
    /// always set (the first heading when --title is not given).
    #[arg(long)]
    pdfua: bool,
    /// Do not write bookmarks from the headings (Word's "Create bookmarks using Headings").
    #[arg(long)]
    no_bookmarks: bool,
    /// Use fonts installed on this machine for families the bundled fonts cannot stand in for.
    /// The result then depends on the machine.
    #[arg(long)]
    system_fonts: bool,
    /// Replace the output file if it exists (also needs --allow-destructive).
    #[arg(long)]
    overwrite: bool,
    /// Permit replacing an existing output file (the policy treats that as destructive).
    #[arg(long)]
    allow_destructive: bool,
    /// Fence the output to these folders (repeatable): the path is resolved with symlinks
    /// followed and must lie inside one; the file is written at the resolved path.
    #[arg(long = "root")]
    roots: Vec<PathBuf>,
}

fn pdf_options(args: &Args) -> PdfOptions {
    PdfOptions {
        conformance: match args.pdfa.as_deref() {
            Some("1b") => Conformance::PdfA1b,
            Some("2b") => Conformance::PdfA2b,
            _ => Conformance::None,
        },
        bookmarks: !args.no_bookmarks,
        ua: args.pdfua,
        pages: None,
    }
}

fn refuse(command: &Command, reason: String) -> CliError {
    audit(&json!({
        "seq": 1,
        "command": command.name(),
        "effect": effect_name(command.effect(None)),
        "status": "refused",
        "reason": reason,
    }));
    CliError::Refused(format!("refused; nothing was written: {reason}"))
}

fn metadata(args: &Args, doc: &Document) -> PdfMetadata {
    let mut metadata = if args.document_properties {
        PdfMetadata::from_document(doc)
    } else {
        PdfMetadata::default()
    };
    for (slot, value) in [
        (&mut metadata.title, &args.title),
        (&mut metadata.author, &args.author),
        (&mut metadata.subject, &args.subject),
        (&mut metadata.keywords, &args.keywords),
        (&mut metadata.language, &args.language),
    ] {
        if value.is_some() {
            slot.clone_from(value);
        }
    }
    metadata
}

fn fallback_fonts(layout: &Layout) -> Vec<String> {
    layout
        .fonts
        .iter()
        .filter(|f| f.matched == FontMatch::Fallback)
        .map(|f| f.requested.clone())
        .collect()
}

pub(crate) fn run(args: &Args) -> Result<Report, CliError> {
    let bytes = read(&args.file)?;
    let doc = Document::open(&bytes, &Limits::default())
        .map_err(|e| CliError::Other(format!("{}: {e}", args.file.display())))?;
    let typed = Command::ExportPdf {
        path: args.output.clone(),
        overwrite: args.overwrite,
    };
    // The fence resolves the path once; that resolved path is what is checked and written.
    let target: PathBuf = if args.roots.is_empty() {
        args.output.clone()
    } else {
        let fence = Fence::new(&args.roots).map_err(CliError::Other)?;
        fence
            .resolve_new(&args.output.display().to_string())
            .map_err(|denied| refuse(&typed, format!("{}: {}", args.output.display(), denied.0)))?
    };
    let resolved = Command::ExportPdf {
        path: target.clone(),
        overwrite: args.overwrite,
    };
    cli_decision(&assess(&resolved, None), args.allow_destructive)
        .map_err(|reason| refuse(&resolved, reason))?;
    if target.exists() && !args.overwrite {
        return Err(CliError::Other(format!(
            "{} exists; pass --overwrite to replace it",
            args.output.display()
        )));
    }

    let never = Arc::new(AtomicBool::new(false));
    let mut fonts = FontLibrary::bundled();
    let lay = |fonts: &FontLibrary| {
        layout_document_cancellable(&doc, fonts, LayoutOptions::default(), &never)
            .ok_or_else(|| CliError::Other("layout was cancelled".to_owned()))
    };
    let mut layout = lay(&fonts)?;
    if args.system_fonts {
        let names = fallback_fonts(&layout);
        if !names.is_empty() {
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            fonts.add_system_families(&refs, &[]);
            layout = lay(&fonts)?;
        }
    }
    let mut warnings: Vec<String> = fallback_fonts(&layout)
        .into_iter()
        .map(|f| format!("font \"{f}\" is not available; the default font is used"))
        .collect();
    let out = export_pdf_with(
        &doc,
        &layout,
        &fonts,
        &metadata(args, &doc),
        &pdf_options(args),
        &never,
    )
    .map_err(|e| CliError::Other(format!("{}: {e}", args.file.display())))?;
    warnings.extend(out.warnings);
    write_pdf(&target, &out.bytes, args.overwrite).map_err(|e| match e {
        WriteError::Exists => CliError::Other(format!(
            "{} exists; pass --overwrite to replace it",
            args.output.display()
        )),
        WriteError::Io(e) => CliError::Io(target.clone(), e),
    })?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let pages = layout.pages.len();
    audit(&json!({
        "seq": 1,
        "command": resolved.name(),
        "effect": effect_name(resolved.effect(None)),
        "status": "applied",
        "path": target.display().to_string(),
        "bytes_written": out.bytes.len(),
        "pages": pages,
    }));
    let data = json!({
        "output": target.display().to_string(),
        "pages": pages,
        "bytes_written": out.bytes.len(),
        "pdfa": args.pdfa,
        "pdfua": args.pdfua,
        "bookmarks": !args.no_bookmarks,
        "warnings": warnings,
    });
    Ok(Report::new(
        data,
        format!(
            "wrote {} ({pages} pages, {} bytes)",
            target.display(),
            out.bytes.len()
        ),
    ))
}

enum WriteError {
    Exists,
    Io(std::io::Error),
}

/// Without `overwrite` the file is created exclusively, so an existing file or a planted link is
/// never followed or replaced; with it the bytes go to a sibling temporary file that replaces the
/// target.
fn write_pdf(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), WriteError> {
    if overwrite {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(WriteError::Io)?;
        let result = file
            .write_all(bytes)
            .and_then(|()| file.flush())
            .and_then(|()| {
                drop(file);
                std::fs::rename(&tmp, path)
            });
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(WriteError::Io(e));
        }
        return Ok(());
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                WriteError::Exists
            } else {
                WriteError::Io(e)
            }
        })?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.flush()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(WriteError::Io(e));
    }
    Ok(())
}
