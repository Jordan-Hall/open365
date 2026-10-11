//! Tools added after the first slice: PDF export, new document from a template, layout
//! summary and formula evaluation. Same rules as `tools.rs`: every path is fenced, every call
//! is audited by the caller, effects go through `policy`, and output has a size cap.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use open365_commands::{
    Command, EffectClass, Target, evaluate_formula as eval_formula, template_output_extension,
};
use open365_layout::{LayoutOptions, ParaRole, layout_document};
use open365_pdf::{PdfMetadata, export_pdf};
use open365_text::{FontLibrary, FontMatch};
use open365_xlsx::parse_cell_ref;
use serde_json::{Value, json};

use crate::server::{MAX_DOCUMENT_BYTES, Server};
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
        let s = self.session(args, detail)?;
        let doc = s
            .target
            .docx()
            .ok_or_else(|| ToolError::invalid("export_pdf needs a DOCX document"))?;
        let fonts = FontLibrary::bundled();
        let layout = layout_document(doc, &fonts, LayoutOptions::default());
        let out = export_pdf(doc, &layout, &fonts, &PdfMetadata::default())
            .map_err(|e| ToolError::failed(e.to_string()))?;
        if out.bytes.len() > MAX_EXPORT_BYTES {
            return Err(ToolError::refused(format!(
                "the PDF is larger than {MAX_EXPORT_BYTES} bytes"
            )));
        }
        write_new(&path, &out.bytes, overwrite)?;
        detail.insert("bytes_written".to_owned(), json!(out.bytes.len()));
        let warnings: Vec<&String> = out.warnings.iter().take(MAX_NOTES).collect();
        Ok(json!({
            "exported_to": display(&path),
            "bytes_written": out.bytes.len(),
            "pages": layout.pages.len(),
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
        let s = self.session(args, detail)?;
        let doc = s
            .target
            .docx()
            .ok_or_else(|| ToolError::invalid("layout_summary needs a DOCX document"))?;
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
        Ok(json!({
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
        let s = self.session(args, detail)?;
        let wb = s
            .target
            .xlsx()
            .ok_or_else(|| ToolError::invalid("evaluate_formula needs an XLSX document"))?;
        let index = views::resolve_sheet(wb, &sheet, hidden).map_err(ToolError::invalid)?;
        let keep = views::visibility_filter(wb, hidden);
        let out = eval_formula(wb, index, row, col, formula, &keep)
            .map_err(|e| ToolError::invalid(e.to_string()))?;
        let mut v = serde_json::to_value(&out).map_err(|e| ToolError::failed(e.to_string()))?;
        v["sheet"] = json!(index);
        v["cell"] = json!(cell);
        v["hidden_content_excluded"] = json!(!hidden);
        v["saved"] = json!(false);
        v["notice"] = json!(NOTICE);
        Ok(v)
    }
}

/// Writes a new file. Without `overwrite` the file must not exist (`create_new`); with it the
/// bytes go to a sibling temporary file that replaces the target.
fn write_new(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), ToolError> {
    if overwrite {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".open365-tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, bytes).map_err(|e| ToolError::failed(e.to_string()))?;
        return std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            ToolError::failed(e.to_string())
        });
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                ToolError::refused("the target exists; pass overwrite: true to replace it")
            } else {
                ToolError::failed(e.to_string())
            }
        })?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.flush()) {
        let _ = std::fs::remove_file(path);
        return Err(ToolError::failed(e.to_string()));
    }
    Ok(())
}
