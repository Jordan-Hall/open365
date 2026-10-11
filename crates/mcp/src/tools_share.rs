//! The Stage 0 sharing tools (ADR-013): `share_send_copy`, `share_send_pdf`, `share_inspect`,
//! `share_sanitize_copy` and `share_os_sheet`. Each runs the matching `share_*` command on a
//! private copy of the open document, so the open document is never touched. Every path is
//! fenced; copies need the `export` scope and, to replace a file, `overwrite: true` plus the
//! operator's confirmation; `share_os_sheet` is egress and needs `execute-external`.

use std::path::{Path, PathBuf};

use open365_commands::{Command, CommandError, HiddenKind, Outcome, PdfOptions, Target};
use serde_json::{Value, json};

use crate::audit::put_text;
use crate::bounded::{self, BoundedError};
use crate::policy::assess;
use crate::server::{Kind, Server};
use crate::tools::{Detail, NOTICE, ToolError, display, effect_name, str_arg};
use crate::tools_ext::{MAX_EXPORT_BYTES, check_create, snapshot};

fn bool_arg(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

fn map_error(e: CommandError) -> ToolError {
    match e {
        CommandError::Exists { .. } => {
            ToolError::refused("the target exists; pass overwrite: true to replace it")
                .coded("target_exists")
        }
        CommandError::Share(m) => ToolError::refused(m).coded("share_refused"),
        CommandError::TooLarge { .. } => ToolError::refused(e.to_string()).coded("limit_exceeded"),
        e => ToolError::failed(e.to_string()).coded("share_failed"),
    }
}

/// A private copy of the session's document as it is now, at the same path so the copy knows
/// which file is the original.
fn private_copy(
    kind: Kind,
    path: Option<&Path>,
    bytes: &[u8],
    roots: &[PathBuf],
) -> Result<Target, ToolError> {
    let opened = match path {
        Some(p) => Target::open_path_bytes(p, bytes),
        None => match kind {
            Kind::Docx => Target::open_docx(bytes),
            Kind::Xlsx => Target::open_xlsx(bytes),
        },
    };
    let mut target =
        opened.map_err(|e| ToolError::failed(e.to_string()).coded("scratch_failed"))?;
    target.restrict_writes_to(roots);
    Ok(target)
}

impl Server {
    /// Shared body of the three tools that write a copy.
    fn share_write(
        &mut self,
        tool: &'static str,
        args: &Value,
        detail: &mut Detail,
        pdf: bool,
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
        let remove = if tool == "share_sanitize_copy" {
            let list = args
                .get("remove")
                .and_then(Value::as_array)
                .ok_or_else(|| ToolError::invalid("argument remove (array) is required"))?;
            let kinds: Vec<HiddenKind> = serde_json::from_value(Value::Array(list.clone()))
                .map_err(|e| {
                    ToolError::invalid(format!("invalid remove list: {e}")).coded("invalid_remove")
                })?;
            detail.insert(
                "remove".to_owned(),
                json!(kinds.iter().map(|k| k.name()).collect::<Vec<_>>()),
            );
            kinds
        } else {
            Vec::new()
        };
        check_create(self, tool, &path, overwrite, detail)?;
        let s = self.session_mut(args, detail)?;
        let (kind, ext) = (s.kind, s.ext.clone());
        let want = if pdf { "pdf".to_owned() } else { ext };
        if extension(&path) != want {
            return Err(ToolError::invalid(format!(
                "the file extension must be .{want}"
            )));
        }
        if pdf && kind != Kind::Docx {
            return Err(ToolError::invalid("share_send_pdf needs a DOCX document"));
        }
        let bytes = snapshot(s)?;
        let source = s.target.path().map(Path::to_path_buf);
        let roots = self.fence.roots().to_vec();
        let command = match tool {
            "share_send_copy" => Command::ShareSendCopy {
                path: Some(path.clone()),
                overwrite,
            },
            "share_send_pdf" => Command::ShareSendPdf {
                path: Some(path.clone()),
                overwrite,
            },
            _ => Command::ShareSanitizeCopy {
                path: Some(path.clone()),
                overwrite,
                remove,
            },
        };
        let risk = assess(&command, source.as_deref());
        self.require(risk.scope)?;
        let deadline = self.config.deadline;
        let done = bounded::run(deadline, move |cancel| {
            let mut copy = private_copy(kind, source.as_deref(), &bytes, &roots)
                .map_err(|e| CommandError::Share(e.message))?;
            copy.set_pdf_options(PdfOptions {
                max_bytes: Some(MAX_EXPORT_BYTES),
                cancel: Some(cancel),
                ..PdfOptions::default()
            });
            copy.apply(command)
        });
        let out: Outcome = match done {
            Ok(Ok(out)) => out,
            Ok(Err(CommandError::Cancelled)) | Err(BoundedError::TimedOut) => {
                return Err(ToolError::timeout(tool, deadline));
            }
            Ok(Err(e)) => return Err(map_error(e)),
            Err(BoundedError::Failed) => {
                return Err(ToolError::failed("the share worker stopped unexpectedly")
                    .coded("worker_failed"));
            }
        };
        detail.insert("bytes_written".to_owned(), json!(out.bytes_written));
        Ok(json!({
            "written_to": display(&path),
            "bytes_written": out.bytes_written,
            "pages": out.pages,
            "notes": out.notes.iter().take(20).collect::<Vec<_>>(),
            "report": out.report,
            "effect": effect_name(out.effect),
        }))
    }

    pub(crate) fn share_send_copy(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        self.share_write("share_send_copy", args, detail, false)
    }

    pub(crate) fn share_send_pdf(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        self.share_write("share_send_pdf", args, detail, true)
    }

    pub(crate) fn share_sanitize_copy(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        self.share_write("share_sanitize_copy", args, detail, false)
    }

    pub(crate) fn share_inspect(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("read"));
        let roots = self.fence.roots().to_vec();
        let s = self.session_mut(args, detail)?;
        let kind = s.kind;
        let bytes = snapshot(s)?;
        let source = s.target.path().map(Path::to_path_buf);
        let mut copy = private_copy(kind, source.as_deref(), &bytes, &roots)?;
        let out = copy.apply(Command::ShareInspect).map_err(map_error)?;
        let mut report = out.report.unwrap_or(Value::Null);
        report["notice"] = json!(NOTICE);
        Ok(report)
    }

    pub(crate) fn share_os_sheet(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("egress"));
        let raw = str_arg(args, "path")?;
        put_text(detail, "requested_path", raw);
        let path = self
            .fence
            .resolve_existing(raw)
            .map_err(|d| ToolError::refused(d.0).coded("path_denied"))?;
        put_text(detail, "path", &display(&path));
        let command = Command::ShareOsSheet { path: path.clone() };
        let risk = assess(&command, None);
        self.require(risk.scope)?;
        let summary = format!(
            "hand {} to the operating system share sheet",
            display(&path)
        );
        self.confirm_destructive("share_os_sheet", &summary, detail)?;
        let roots = self.fence.roots().to_vec();
        let s = self.session_mut(args, detail)?;
        let kind = s.kind;
        let bytes = snapshot(s)?;
        let source = s.target.path().map(Path::to_path_buf);
        let mut copy = private_copy(kind, source.as_deref(), &bytes, &roots)?;
        let out = copy.apply(command).map_err(map_error)?;
        Ok(out.report.unwrap_or(Value::Null))
    }
}
