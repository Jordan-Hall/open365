//! Tools added after the first slice: PDF export, new document from a template, layout
//! summary and formula evaluation. Same rules as `tools.rs`: every path is fenced, every call
//! is audited by the caller, effects go through `policy`, and output has a size cap.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use open365_commands::{Command, CommandError, PdfOptions, Target, template_output_extension};
use open365_docx::Document;
use open365_layout::{LayoutOptions, ParaRole, layout_document_cancellable};
use open365_text::{FontLibrary, FontMatch};
use open365_xlsx::parse_cell_ref;
use serde_json::{Value, json};

use crate::audit::put_text;
use crate::bounded::{self, BoundedError, EvalWorker, Stamp};
use crate::policy::assess_new_file;
use crate::server::{MAX_DOCUMENT_BYTES, Server, Session};
use crate::tools::{Detail, NOTICE, ToolError, display, read_limited, str_arg};
use crate::views;

/// Largest PDF `export_pdf` writes, in bytes.
pub const MAX_EXPORT_BYTES: usize = 64 * 1024 * 1024;
/// Most pages listed by `layout_summary`.
pub const MAX_SUMMARY_PAGES: usize = 1000;
/// Most headings listed by `layout_summary`.
pub const MAX_SUMMARY_HEADINGS: usize = 200;
/// Most tables listed by `layout_summary`.
pub const MAX_SUMMARY_TABLES: usize = 100;
/// Longest heading text returned, in characters.
pub const MAX_HEADING_CHARS: usize = 200;
/// Most layout notes and PDF warnings returned.
const MAX_NOTES: usize = 20;

fn bool_arg(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// The write policy shared by the two tools that create files: the `export` scope for a new
/// file; replacing one needs `overwrite: true` and the operator's confirmation.
pub(crate) fn check_create(
    server: &mut Server,
    tool: &str,
    path: &Path,
    overwrite: bool,
    detail: &mut Detail,
) -> Result<(), ToolError> {
    let risk = assess_new_file(path, overwrite);
    server.require(risk.scope)?;
    if path.exists() {
        if !overwrite {
            return Err(ToolError::refused(
                "the target exists; pass overwrite: true to replace it",
            )
            .coded("target_exists"));
        }
        let summary = format!("replace the existing file {}", display(path));
        server
            .confirm_destructive(tool, &summary, detail)
            .map_err(|e| {
                ToolError::refused(format!("replacing a file: {}", e.message))
                    .coded("confirmation_denied")
            })?;
    }
    Ok(())
}

fn clip_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

impl Server {
    pub(crate) fn export_pdf(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("export"));
        let raw = str_arg(args, "path")?;
        put_text(detail, "requested_path", raw);
        let overwrite = bool_arg(args, "overwrite");
        detail.insert("overwrite".to_owned(), json!(overwrite));
        let path = self
            .fence
            .resolve_new(raw)
            .map_err(|d| ToolError::refused(d.0).coded("path_denied"))?;
        put_text(detail, "path", &display(&path));
        check_create(self, "export_pdf", &path, overwrite, detail)?;
        if extension(&path) != "pdf" {
            return Err(ToolError::invalid("the file extension must be .pdf"));
        }
        let deadline = self.config.deadline;
        let s = self.session_mut(args, detail)?;
        if s.target.docx().is_none() {
            return Err(ToolError::invalid("export_pdf needs a DOCX document"));
        }
        let bytes = snapshot(s)?;
        let target_path = path.clone();
        let roots = self.fence.roots().to_vec();
        // The export is the `export_pdf` command on a private copy of the document, so the
        // open document is not borrowed while the layout runs.
        let done = bounded::run(deadline, move |cancel| {
            let mut copy = Target::open_docx(&bytes)?;
            copy.restrict_writes_to(&roots);
            copy.set_pdf_options(PdfOptions {
                max_bytes: Some(MAX_EXPORT_BYTES),
                cancel: Some(cancel),
                ..PdfOptions::default()
            });
            copy.apply(Command::ExportPdf {
                path: target_path,
                overwrite,
            })
        });
        let out = match done {
            Ok(Ok(out)) => out,
            Ok(Err(CommandError::Exists { .. })) => {
                return Err(ToolError::refused(
                    "the target exists; pass overwrite: true to replace it",
                )
                .coded("target_exists"));
            }
            Ok(Err(e @ CommandError::TooLarge { .. })) => {
                return Err(ToolError::refused(e.to_string()).coded("limit_exceeded"));
            }
            Ok(Err(CommandError::Cancelled)) | Err(BoundedError::TimedOut) => {
                // The timeout set the request's flag; the writer removes a file it created.
                return Err(ToolError::timeout("export_pdf", deadline));
            }
            Ok(Err(e)) => return Err(ToolError::failed(e.to_string()).coded("export_failed")),
            Err(BoundedError::Failed) => {
                return Err(ToolError::failed("the export worker stopped unexpectedly")
                    .coded("worker_failed"));
            }
        };
        detail.insert("bytes_written".to_owned(), json!(out.bytes_written));
        let warnings: Vec<&String> = out.notes.iter().take(MAX_NOTES).collect();
        Ok(json!({
            "exported_to": display(&path),
            "bytes_written": out.bytes_written,
            "pages": out.pages,
            "warnings": warnings,
            "effect": "export",
        }))
    }

    pub(crate) fn new_from_template(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("export"));
        let raw = str_arg(args, "path")?;
        let raw_out = str_arg(args, "output")?;
        put_text(detail, "requested_path", raw);
        put_text(detail, "requested_output", raw_out);
        let overwrite = bool_arg(args, "overwrite");
        detail.insert("overwrite".to_owned(), json!(overwrite));
        let template = self
            .fence
            .resolve_existing(raw)
            .map_err(|d| ToolError::refused(d.0).coded("path_denied"))?;
        let output = self
            .fence
            .resolve_new(raw_out)
            .map_err(|d| ToolError::refused(d.0).coded("path_denied"))?;
        put_text(detail, "path", &display(&template));
        put_text(detail, "output", &display(&output));
        check_create(self, "new_from_template", &output, overwrite, detail)?;
        let ext = extension(&template);
        let want = template_output_extension(&ext).ok_or_else(|| {
            ToolError::invalid("the template must be a .dotx, .dotm, .xltx or .xltm file")
                .coded("bad_extension")
        })?;
        if extension(&output) != want {
            return Err(ToolError::invalid(format!(
                "a .{ext} template makes a .{want} file; the output extension must be .{want}"
            ))
            .coded("bad_extension"));
        }
        let bytes = read_limited(&template, MAX_DOCUMENT_BYTES)?;
        let mut target = Target::open_template(&bytes, &ext)
            .map_err(|e| ToolError::invalid(e.to_string()).coded("open_failed"))?;
        target.restrict_writes_to(self.fence.roots());
        let macros = target.docx().is_some_and(|d| d.has_vba_project())
            || target.xlsx().is_some_and(|w| w.has_vba_project());
        let out = target
            .apply(Command::Save {
                path: output.clone(),
            })
            .map_err(|e| ToolError::failed(e.to_string()).coded("save_failed"))?;
        detail.insert("bytes_written".to_owned(), json!(out.bytes_written));
        Ok(json!({
            "created": display(&output),
            "bytes_written": out.bytes_written,
            "effect": "export",
            "vba_project": macros,
            "note": "A VBA project is kept as is and never run.",
        }))
    }

    pub(crate) fn layout_summary(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("read"));
        let hidden = self.config.include_hidden;
        let deadline = self.config.deadline;
        let s = self.session_mut(args, detail)?;
        if s.target.docx().is_none() {
            return Err(ToolError::invalid("layout_summary needs a DOCX document"));
        }
        let bytes = snapshot(s)?;
        let summary = bounded::run(deadline, move |cancel| {
            let copy =
                Target::open_docx(&bytes).map_err(|e| SummaryError::Failed(e.to_string()))?;
            let doc = copy
                .docx()
                .ok_or_else(|| SummaryError::Failed("not a DOCX document".to_owned()))?;
            summarize(doc, hidden, &cancel).ok_or(SummaryError::Cancelled)
        });
        match summary {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(SummaryError::Cancelled)) | Err(BoundedError::TimedOut) => {
                Err(ToolError::timeout("layout_summary", deadline))
            }
            Ok(Err(SummaryError::Failed(e))) => Err(ToolError::failed(e)),
            Err(BoundedError::Failed) => {
                Err(ToolError::failed("the layout worker stopped unexpectedly"))
            }
        }
    }

    pub(crate) fn evaluate_formula(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("read"));
        let hidden = self.config.include_hidden;
        let cell = str_arg(args, "cell")?.to_owned();
        put_text(detail, "cell", &cell);
        let formula = match args.get("formula") {
            None | Some(Value::Null) => None,
            Some(Value::String(f)) => Some(f.as_str()),
            Some(_) => return Err(ToolError::invalid("argument formula must be a string")),
        };
        // The formula is the agent's own input, not document content; only its size is logged.
        detail.insert(
            "formula_chars".to_owned(),
            json!(formula.map_or(0, |f| f.chars().count())),
        );
        let (row, col) =
            parse_cell_ref(&cell).ok_or_else(|| ToolError::invalid("cell must look like B2"))?;
        let sheet = args.get("sheet").cloned().unwrap_or(Value::Null);
        let deadline = self.config.deadline;
        let s = self.session_mut(args, detail)?;
        let wb = s
            .target
            .xlsx()
            .ok_or_else(|| ToolError::invalid("evaluate_formula needs an XLSX document"))?;
        let index = views::resolve_sheet(wb, &sheet, hidden).map_err(ToolError::invalid)?;
        // One engine per open workbook, rebuilt when the document or its file changes.
        let stamp = Stamp::of(&s.target);
        let reuse = s.eval.as_ref().is_some_and(|w| w.stamp == stamp);
        detail.insert(
            "engine".to_owned(),
            json!(if reuse { "reused" } else { "built" }),
        );
        if !reuse {
            s.eval = None;
            let bytes = snapshot(s)?;
            let worker = EvalWorker::spawn(bytes, hidden, stamp)
                .map_err(|_| ToolError::failed("the evaluation worker could not start"))?;
            s.eval = Some(worker);
        }
        let worker = s
            .eval
            .as_ref()
            .ok_or_else(|| ToolError::failed("no evaluation worker"))?;
        let answer = worker.evaluate(deadline, index, row, col, formula.map(str::to_owned));
        let out = match answer {
            Ok(Ok(out)) => out,
            Ok(Err(CommandError::Cancelled)) | Err(BoundedError::TimedOut) => {
                // The job was cancelled. The worker keeps its engine: the probe cell is put
                // back and unfinished cells stay dirty, so the next call can reuse it.
                return Err(ToolError::timeout("evaluate_formula", deadline));
            }
            Ok(Err(e)) => return Err(ToolError::invalid(e.to_string())),
            Err(BoundedError::Failed) => {
                s.eval = None;
                return Err(ToolError::failed(
                    "the evaluation worker stopped unexpectedly",
                ));
            }
        };
        let mut v = serde_json::to_value(&out).map_err(|e| ToolError::failed(e.to_string()))?;
        v["sheet"] = json!(index);
        v["cell"] = json!(cell);
        v["hidden_content_excluded"] = json!(!hidden);
        v["saved"] = json!(false);
        v["notice"] = json!(NOTICE);
        Ok(v)
    }
}

/// The bytes of the session's document as it is now: the file as opened while nothing was
/// edited, otherwise a fresh serialisation. Worker threads reopen their own copy from them.
pub(crate) fn snapshot(s: &mut Session) -> Result<Vec<u8>, ToolError> {
    if s.target.revision() == 0 {
        return Ok(s.original.clone());
    }
    s.target
        .snapshot_bytes()
        .map_err(|e| ToolError::failed(format!("the edited document cannot be copied: {e}")))
}

/// Why a layout summary produced nothing.
enum SummaryError {
    Cancelled,
    Failed(String),
}

/// The `layout_summary` result for a document. Runs on a worker thread.
fn summarize(doc: &Document, hidden: bool, cancel: &Arc<AtomicBool>) -> Option<Value> {
    let fonts = FontLibrary::bundled();
    let layout = layout_document_cancellable(doc, &fonts, LayoutOptions::default(), cancel)?;
    let page_lines: Vec<usize> = layout
        .pages
        .iter()
        .take(MAX_SUMMARY_PAGES)
        .map(|p| p.lines.len())
        .collect();
    let mut headings = Vec::new();
    let mut headings_cut = false;
    let mut seen = HashSet::new();
    // table id -> (first page, last page, rows, columns)
    let mut tables: BTreeMap<usize, (usize, usize, usize, usize)> = BTreeMap::new();
    for page in &layout.pages {
        for line in &page.lines {
            let ParaRole::Heading(level) = line.role else {
                continue;
            };
            if !seen.insert(line.paragraph) {
                continue;
            }
            let text = doc
                .paragraph(line.paragraph)
                .map(|p| views::visible_text(p, hidden).0)
                .unwrap_or_default();
            if text.trim().is_empty() {
                continue;
            }
            if headings.len() >= MAX_SUMMARY_HEADINGS {
                headings_cut = true;
                continue;
            }
            headings.push(json!({
                "page": page.number,
                "paragraph": line.paragraph,
                "level": level,
                "text": clip_chars(text.trim(), MAX_HEADING_CHARS),
            }));
        }
        for c in &page.cells {
            let e = tables
                .entry(c.table)
                .or_insert((page.number, page.number, 0, 0));
            e.1 = page.number;
            e.2 = e.2.max(c.row + c.row_span.max(1));
            e.3 = e.3.max(c.col + c.col_span.max(1));
        }
    }
    let tables_cut = tables.len() > MAX_SUMMARY_TABLES;
    let table_list: Vec<Value> = tables
            .iter()
            .take(MAX_SUMMARY_TABLES)
            .map(|(id, (first, last, rows, cols))| {
                json!({"table": id, "first_page": first, "last_page": last, "rows": rows, "columns": cols})
            })
            .collect();
    let notes: Vec<&String> = layout.notes.iter().take(MAX_NOTES).collect();
    let fallback_fonts = layout
        .fonts
        .iter()
        .filter(|f| f.matched == FontMatch::Fallback)
        .count();
    Some(json!({
        "page_count": layout.pages.len(),
        "page_lines": page_lines,
        "pages_truncated": layout.pages.len() > MAX_SUMMARY_PAGES,
        "headings": headings,
        "headings_truncated": headings_cut,
        "tables": table_list,
        "tables_truncated": tables_cut,
        "layout_notes": notes,
        "fallback_fonts": fallback_fonts,
        "notice": NOTICE,
    }))
}
