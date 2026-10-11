//! Read tools added with the full command-layer coverage: the command catalogue, formulas,
//! styles and recalculation. Read-only; hidden content follows `--include-hidden`.

use open365_commands::{CommandError, Target};
use serde_json::{Value, json};

use crate::bounded::{self, BoundedError};
use crate::registry;
use crate::server::Server;
use crate::tools::{Detail, NOTICE, ToolError};
use crate::tools_ext::snapshot;
use crate::{views, views_ext};

fn opt_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(ToolError::invalid(format!(
            "argument {key} must be a string"
        ))),
    }
}

impl Server {
    pub(crate) fn list_commands(&mut self, detail: &mut Detail) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("read"));
        Ok(registry::catalogue_json())
    }

    pub(crate) fn read_formulas(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("read"));
        let hidden = self.config.include_hidden;
        let range = opt_str(args, "range")?.map(str::to_owned);
        let sheet = args.get("sheet").cloned().unwrap_or(Value::Null);
        let s = self.session(args, detail)?;
        let wb = s
            .target
            .xlsx()
            .ok_or_else(|| ToolError::invalid("read_formulas needs an XLSX document"))?;
        let index = views::resolve_sheet(wb, &sheet, hidden).map_err(ToolError::invalid)?;
        let mut out = views_ext::xlsx_formulas(wb, index, range.as_deref(), hidden)
            .map_err(ToolError::invalid)?;
        out["sheet"] = json!(index);
        out["notice"] = json!(NOTICE);
        Ok(out)
    }

    pub(crate) fn get_styles(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("read"));
        let hidden = self.config.include_hidden;
        let range = opt_str(args, "range")?.map(str::to_owned);
        let sheet = args.get("sheet").cloned().unwrap_or(Value::Null);
        let s = self.session(args, detail)?;
        let mut out = match (s.target.docx(), s.target.xlsx()) {
            (Some(d), _) => views_ext::docx_styles(d),
            (_, Some(wb)) => {
                let index = if range.is_some() {
                    views::resolve_sheet(wb, &sheet, hidden).map_err(ToolError::invalid)?
                } else {
                    0
                };
                let mut v = views_ext::xlsx_styles(wb, index, range.as_deref(), hidden)
                    .map_err(ToolError::invalid)?;
                if range.is_some() {
                    v["sheet"] = json!(index);
                }
                v
            }
            _ => json!({}),
        };
        out["notice"] = json!(NOTICE);
        Ok(out)
    }

    pub(crate) fn recalc(&mut self, args: &Value, detail: &mut Detail) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("read"));
        let hidden = self.config.include_hidden;
        let range = opt_str(args, "range")?.map(str::to_owned);
        let sheet = args.get("sheet").cloned().unwrap_or(Value::Null);
        let deadline = self.config.deadline;
        let s = self.session_mut(args, detail)?;
        let wb = s
            .target
            .xlsx()
            .ok_or_else(|| ToolError::invalid("recalc needs an XLSX document"))?;
        let index = views::resolve_sheet(wb, &sheet, hidden).map_err(ToolError::invalid)?;
        let bytes = snapshot(s)?;
        // The engine is not Send, so it is built and dropped on the worker thread.
        let done = bounded::run(deadline, move |cancel| {
            let target = Target::open_xlsx(&bytes)?;
            let wb = target
                .xlsx()
                .ok_or_else(|| CommandError::Formula("not a workbook".to_owned()))?;
            views_ext::xlsx_recalc(wb, index, range.as_deref(), hidden, &cancel)
        });
        let mut out = match done {
            Ok(Ok(v)) => v,
            Ok(Err(CommandError::Cancelled)) | Err(BoundedError::TimedOut) => {
                return Err(ToolError::timeout("recalc", deadline));
            }
            Ok(Err(e)) => return Err(ToolError::invalid(e.to_string())),
            Err(BoundedError::Failed) => {
                return Err(ToolError::failed(
                    "the recalculation worker stopped unexpectedly",
                ));
            }
        };
        out["notice"] = json!(NOTICE);
        Ok(out)
    }
}
