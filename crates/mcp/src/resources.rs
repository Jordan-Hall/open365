//! MCP resources: read-only views addressed by URI. The static list names the catalogue and the
//! session; the templates address views of an open document. Reading goes through the same
//! tools as `tools/call`, so hidden-content and size rules cannot differ between the two.

use serde_json::{Value, json};

use crate::registry;
use crate::server::{
    MAX_COMMANDS, MAX_DOCUMENT_BYTES, MAX_OPEN_DOCUMENTS, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
    Server,
};
use crate::tools::{Detail, ToolError};

const SCHEME: &str = "open365://";

/// `resources/list`: the same for every client, whatever documents are open.
pub fn list() -> Value {
    json!({"resources": [
        {
            "uri": "open365://commands",
            "name": "commands",
            "title": "Command catalogue",
            "description": "Every document command with its domain, effect class and a runnable example.",
            "mimeType": "application/json",
        },
        {
            "uri": "open365://session",
            "name": "session",
            "title": "Session policy",
            "description": "The scopes this session holds, its limits and its open documents.",
            "mimeType": "application/json",
        },
    ]})
}

/// `resources/templates/list`: views of an open document (the id comes from `open_document`).
pub fn templates() -> Value {
    let t = |uri: &str, name: &str, description: &str| json!({"uriTemplate": uri, "name": name, "description": description, "mimeType": "application/json"});
    json!({"resourceTemplates": [
        t("open365://document/{document}/outline", "outline", "DOCX headings or XLSX sheets."),
        t("open365://document/{document}/structure", "structure", "Package parts, relationships and integrity issues."),
        t("open365://document/{document}/styles", "styles", "DOCX styles or XLSX cell styles and number formats."),
        t("open365://document/{document}/text/{start}", "text", "DOCX paragraphs from a start index."),
        t("open365://document/{document}/cells/{sheet}/{range}", "cells", "XLSX values and formulas in a range such as A1:D20."),
        t("open365://document/{document}/formulas/{sheet}", "formulas", "XLSX formula cells of a sheet."),
    ]})
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn sheet_arg(text: &str) -> Value {
    text.parse::<u64>()
        .map_or_else(|_| json!(text), |n| json!(n))
}

fn not_found() -> ToolError {
    ToolError::invalid("resource not found")
}

/// Reads a resource and returns its JSON text.
pub(crate) fn read(
    server: &mut Server,
    uri: &str,
    detail: &mut Detail,
) -> Result<String, ToolError> {
    let rest = uri.strip_prefix(SCHEME).ok_or_else(not_found)?;
    let parts: Vec<String> = rest
        .split('/')
        .map(percent_decode)
        .collect::<Option<_>>()
        .ok_or_else(not_found)?;
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    detail.insert("effect".to_owned(), json!("read"));
    let value = match parts.as_slice() {
        ["commands"] => registry::catalogue_json(),
        ["session"] => session(server),
        ["document", id, view @ ..] => {
            let (tool, mut args) = match view {
                ["outline"] => ("get_outline", json!({})),
                ["structure"] => ("inspect", json!({})),
                ["styles"] => ("get_styles", json!({})),
                ["text", start] => (
                    "read_text",
                    json!({"start": start.parse::<u64>().map_err(|_| not_found())?}),
                ),
                ["cells", sheet, range] => (
                    "read_cells",
                    json!({"sheet": sheet_arg(sheet), "range": range}),
                ),
                ["formulas", sheet] => ("read_formulas", json!({"sheet": sheet_arg(sheet)})),
                _ => return Err(not_found()),
            };
            args["document"] = json!(id);
            server.dispatch(tool, &args, detail)?
        }
        _ => return Err(not_found()),
    };
    Ok(value.to_string())
}

fn session(server: &Server) -> Value {
    json!({
        "scopes": server.granted().iter().map(|s| s.name()).collect::<Vec<_>>(),
        "include_hidden": server.config.include_hidden,
        "documents": server.docs.keys().collect::<Vec<_>>(),
        "limits": {
            "request_bytes": MAX_REQUEST_BYTES,
            "response_bytes": MAX_RESPONSE_BYTES,
            "document_bytes": MAX_DOCUMENT_BYTES,
            "open_documents": MAX_OPEN_DOCUMENTS,
            "commands_per_call": MAX_COMMANDS,
            "deadline_secs": server.config.deadline.as_secs_f64(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("My%20Sheet").as_deref(), Some("My Sheet"));
        assert_eq!(percent_decode("a%2"), None);
        assert_eq!(percent_decode("%ff"), None);
    }
}
