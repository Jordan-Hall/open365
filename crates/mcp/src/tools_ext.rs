//! Tools added after the first slice: PDF export, new document from a template, layout
//! summary and formula evaluation. Same rules as `tools.rs`: every path is fenced, every call
//! is audited by the caller, effects go through `policy`, and output has a size cap.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use open365_commands::{
    Command, CommandError, EffectClass, PdfOptions, Target, template_output_extension,
};
use open365_docx::Document;
use open365_layout::{LayoutOptions, ParaRole, layout_document};
use open365_text::{FontLibrary, FontMatch};
use open365_xlsx::parse_cell_ref;
use serde_json::{Value, json};

use crate::bounded::{self, BoundedError, EvalWorker, Stamp};
use crate::server::{MAX_DOCUMENT_BYTES, Server, Session};
use crate::tools::{Detail, NOTICE, ToolError, display, policy, str_arg};
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

/// The write policy shared by the two tools that create files: `--allow-edit` for a new
/// file; replacing one needs `overwrite: true` and `--allow-destructive`.
fn check_create(server: &Server, path: &Path, overwrite: bool) -> Result<(), ToolError> {
    let c = &server.config;
    if let Some(why) = policy(c.allow_edit, c.allow_destructive, EffectClass::Export) {
        return Err(ToolError::refused(why));
    }
    if path.exists() {
        if !overwrite {
            return Err(ToolError::refused(
                "the target exists; pass overwrite: true to replace it",
            ));
        }
        if let Some(why) = policy(c.allow_edit, c.allow_destructive, EffectClass::Destructive) {
            return Err(ToolError::refused(format!("replacing a file: {why}")));
        }
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
        detail.insert("requested_path".to_owned(), json!(raw));
        let overwrite = bool_arg(args, "overwrite");
        detail.insert("overwrite".to_owned(), json!(overwrite));
        let path = self
            .fence
            .resolve_new(raw)
            .map_err(|d| ToolError::refused(d.0))?;
        detail.insert("path".to_owned(), json!(display(&path)));
        check_create(self, &path, overwrite)?;
        if extension(&path) != "pdf" {
            return Err(ToolError::invalid("the file extension must be .pdf"));
        }
        let deadline = self.config.deadline;
        let s = self.session_mut(args, detail)?;
        if s.target.docx().is_none() {
            return Err(ToolError::invalid("export_pdf needs a DOCX document"));
        }
        let bytes = snapshot(s)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let options = PdfOptions {
            max_bytes: Some(MAX_EXPORT_BYTES),
            cancel: Some(Arc::clone(&cancel)),
            ..PdfOptions::default()
        };
        let target_path = path.clone();
        // The export is the `export_pdf` command on a private copy of the document, so the
        // open document is not borrowed while the layout runs.
        let done = bounded::run(deadline, move || {
            let mut copy = Target::open_docx(&bytes)?;
            copy.set_pdf_options(options);
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
                ));
            }
            Ok(Err(e @ CommandError::TooLarge { .. })) => {
                return Err(ToolError::refused(e.to_string()));
            }
            Ok(Err(e)) => return Err(ToolError::failed(e.to_string())),
            Err(BoundedError::TimedOut) => {
                // The worker checks this before it creates the file.
                cancel.store(true, Ordering::Relaxed);
                return Err(ToolError::timeout("export_pdf", deadline));
            }
            Err(BoundedError::Failed) => {
                return Err(ToolError::failed("the export worker stopped unexpectedly"));
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
        detail.insert("requested_path".to_owned(), json!(raw));
        detail.insert("requested_output".to_owned(), json!(raw_out));
        let overwrite = bool_arg(args, "overwrite");
        detail.insert("overwrite".to_owned(), json!(overwrite));
        let template = self
            .fence
            .resolve_existing(raw)
            .map_err(|d| ToolError::refused(d.0))?;
        let output = self
            .fence
            .resolve_new(raw_out)
            .map_err(|d| ToolError::refused(d.0))?;
        detail.insert("path".to_owned(), json!(display(&template)));
        detail.insert("output".to_owned(), json!(display(&output)));
        check_create(self, &output, overwrite)?;
        let ext = extension(&template);
        let want = template_output_extension(&ext).ok_or_else(|| {
            ToolError::invalid("the template must be a .dotx, .dotm, .xltx or .xltm file")
        })?;
        if extension(&output) != want {
            return Err(ToolError::invalid(format!(
                "a .{ext} template makes a .{want} file; the output extension must be .{want}"
            )));
        }
        let len = std::fs::metadata(&template)
            .map_err(|e| ToolError::failed(e.to_string()))?
            .len();
        if len > MAX_DOCUMENT_BYTES {
            return Err(ToolError::refused(format!(
                "file is larger than {MAX_DOCUMENT_BYTES} bytes"
            )));
        }
        let bytes = std::fs::read(&template).map_err(|e| ToolError::failed(e.to_string()))?;
        let mut target =
            Target::open_template(&bytes, &ext).map_err(|e| ToolError::invalid(e.to_string()))?;
        let macros = target.docx().is_some_and(|d| d.has_vba_project())
            || target.xlsx().is_some_and(|w| w.has_vba_project());
        let out = target
            .apply(Command::Save {
                path: output.clone(),
            })
            .map_err(|e| ToolError::failed(e.to_string()))?;
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
        let summary = bounded::run(deadline, move || {
            let copy = Target::open_docx(&bytes).map_err(|e| e.to_string())?;
            let doc = copy.docx().ok_or("not a DOCX document")?;
            Ok::<Value, String>(summarize(doc, hidden))
        });
        match summary {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(ToolError::failed(e)),
            Err(BoundedError::TimedOut) => Err(ToolError::timeout("layout_summary", deadline)),
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
        detail.insert("cell".to_owned(), json!(cell));
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
            Ok(Err(e)) => return Err(ToolError::invalid(e)),
            Err(BoundedError::TimedOut) => {
                // Drop the worker: the next call builds a fresh engine instead of queueing
                // behind a job that may never end.
                s.eval = None;
                return Err(ToolError::timeout("evaluate_formula", deadline));
            }
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
fn snapshot(s: &mut Session) -> Result<Vec<u8>, ToolError> {
    if s.target.revision() == 0 {
        return Ok(s.original.clone());
    }
    s.target
        .snapshot_bytes()
        .map_err(|e| ToolError::failed(format!("the edited document cannot be copied: {e}")))
}

/// The `layout_summary` result for a document. Runs on a worker thread.
fn summarize(doc: &Document, hidden: bool) -> Value {
    let fonts = FontLibrary::bundled();
    let layout = layout_document(doc, &fonts, LayoutOptions::default());
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
    json!({
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
    })
}
