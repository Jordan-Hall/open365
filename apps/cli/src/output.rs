//! The `--json` output contract, schema `open365.cli/1` (see `docs/reference/cli.md`).
//!
//! With `--json` every command prints exactly one JSON object on stdout:
//! `{"schema":"open365.cli/1","command":"xlsx cells","ok":true,"result":{...}}` on success, or
//! `{"schema":"open365.cli/1","command":"xlsx cells","ok":false,"error":{"code":"...","message":"..."}}`
//! on failure. The process exit code is 0 on success, 1 on an error, 2 for a usage error
//! (printed by the argument parser) and 3 when the policy refused the request.

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::CliError;

/// The schema identifier in every `--json` document. The number changes only for a breaking
/// change to an existing result shape; added fields do not change it.
pub const SCHEMA: &str = "open365.cli/1";

/// What a command produced: the data for `--json` and the text for people.
#[derive(Debug)]
pub struct Report {
    pub data: Value,
    pub human: String,
    /// Set when the command ran but its check failed (`roundtrip`): exit code 1, and the
    /// envelope has `ok: false` with both `error` and `result`.
    pub failure: Option<String>,
}

impl Report {
    pub fn new(data: Value, human: impl Into<String>) -> Self {
        Self {
            data,
            human: human.into(),
            failure: None,
        }
    }

    pub fn failed(mut self, message: impl Into<String>) -> Self {
        self.failure = Some(message.into());
        self
    }

    /// A report whose human form is the pretty-printed data.
    pub fn json_only(data: Value) -> Self {
        let human = serde_json::to_string_pretty(&data).unwrap_or_default();
        Self {
            data,
            human,
            failure: None,
        }
    }
}

impl CliError {
    /// The stable error code in the `--json` envelope.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(..) => "io",
            Self::Opc(..) => "format",
            Self::Refused(_) => "refused",
            Self::Other(_) => "error",
        }
    }
}

/// The success envelope.
pub fn envelope_ok(command: &str, result: &Value) -> Value {
    json!({"schema": SCHEMA, "command": command, "ok": true, "result": result})
}

/// The failure envelope.
pub fn envelope_err(command: &str, error: &CliError) -> Value {
    json!({
        "schema": SCHEMA,
        "command": command,
        "ok": false,
        "error": {"code": error.code(), "message": error.to_string()},
    })
}

/// The human text without trailing newlines.
fn trimmed(text: &str) -> &str {
    text.trim_end_matches(['\n', '\r'])
}

/// Prints the result and returns the exit code.
pub fn finish(json: bool, command: &str, result: Result<Report, CliError>) -> ExitCode {
    match result {
        Ok(report) => {
            if let Some(message) = report.failure {
                if json {
                    println!(
                        "{}",
                        json!({
                            "schema": SCHEMA,
                            "command": command,
                            "ok": false,
                            "error": {"code": "check_failed", "message": message},
                            "result": report.data,
                        })
                    );
                } else {
                    println!("{}", trimmed(&report.human));
                    eprintln!("error: {message}");
                }
                return ExitCode::FAILURE;
            }
            if json {
                println!("{}", envelope_ok(command, &report.data));
            } else if !report.human.is_empty() {
                println!("{}", trimmed(&report.human));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json {
                println!("{}", envelope_err(command, &e));
            } else {
                eprintln!("error: {e}");
            }
            match e {
                CliError::Refused(_) => ExitCode::from(3),
                _ => ExitCode::FAILURE,
            }
        }
    }
}
