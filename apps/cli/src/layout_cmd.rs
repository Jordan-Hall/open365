//! `open365 docx layout <file>`: pages, lines and table cell boxes from the layout
//! engine (read-only).

use std::path::PathBuf;

use clap::Args as ClapArgs;
use open365_docx::Document;
use open365_layout::{CellBox, LayoutOptions, Line, Page, ParaRole, layout_document};
use open365_opc::Limits;
use open365_text::FontLibrary;
use serde_json::{Value, json};

use crate::output::Report;
use crate::{CliError, read};

/// Pages printed unless `--max-pages` says otherwise.
const DEFAULT_MAX_PAGES: usize = 200;

#[derive(Debug, ClapArgs)]
pub(crate) struct Args {
    /// Input .docx file.
    file: PathBuf,
    /// Most pages to print; the output says when it was cut.
    #[arg(long, default_value_t = DEFAULT_MAX_PAGES)]
    max_pages: usize,
}

fn line_json(l: &Line) -> Value {
    let mut v = json!({
        "paragraph": l.paragraph,
        "start": l.start,
        "end": l.end,
        "top": l.top,
        "baseline": l.baseline,
        "height": l.height,
        "left": l.left,
    });
    match l.role {
        ParaRole::Plain => v["role"] = json!("plain"),
        ParaRole::Heading(n) => {
            v["role"] = json!("heading");
            v["level"] = json!(n);
        }
        ParaRole::ListItem => v["role"] = json!("list_item"),
    }
    v
}

fn cell_json(c: &CellBox) -> Value {
    json!({
        "table": c.table,
        "row": c.row,
        "col": c.col,
        "row_span": c.row_span,
        "col_span": c.col_span,
        "header_row": c.header_row,
        "x": c.x,
        "y": c.y,
        "width": c.width,
        "height": c.height,
        "fill": c.fill,
    })
}

fn page_json(p: &Page) -> Value {
    json!({
        "number": p.number,
        "width": p.section.page_width,
        "height": p.section.page_height,
        "lines": p.lines.iter().map(line_json).collect::<Vec<_>>(),
        "cells": p.cells.iter().map(cell_json).collect::<Vec<_>>(),
    })
}

pub(crate) fn run(args: &Args) -> Result<Report, CliError> {
    let bytes = read(&args.file)?;
    let doc = Document::open(&bytes, &Limits::default())
        .map_err(|e| CliError::Other(format!("{}: {e}", args.file.display())))?;
    let fonts = FontLibrary::bundled();
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());
    let shown = layout.pages.len().min(args.max_pages);
    let data = json!({
        "page_count": layout.pages.len(),
        "pages": layout.pages.iter().take(shown).map(page_json).collect::<Vec<_>>(),
        "truncated": shown < layout.pages.len(),
        "notes": layout.notes,
    });
    let mut human = format!(
        "{} pages
",
        layout.pages.len()
    );
    for p in layout.pages.iter().take(shown) {
        human.push_str(&format!(
            "page {:>3}: {:>4} lines, {:>3} cells ({} x {} pt)
",
            p.number,
            p.lines.len(),
            p.cells.len(),
            p.section.page_width,
            p.section.page_height
        ));
    }
    if shown < layout.pages.len() {
        human.push_str(&format!(
            "... {} more pages (--max-pages)
",
            layout.pages.len() - shown
        ));
    }
    for n in &layout.notes {
        human.push_str(&format!(
            "note: {n}
"
        ));
    }
    Ok(Report::new(data, human))
}
