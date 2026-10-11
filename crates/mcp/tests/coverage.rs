//! Every command of the command layer must be reachable through MCP. The list comes from
//! `registry::COMMANDS`, which `registry.rs` ties to the `Command` enum's source, so a new
//! command fails these tests until it has an entry that really runs.

mod common;

use common::{Rig, Scratch, s};
use open365_mcp::Scope;
use open365_mcp::registry::{COMMANDS, Domain};
use serde_json::{Value, json};

const DOCX: &str = "docx/plain-paragraphs.docx";
const XLSX: &str = "xlsx/formulas-basic.xlsx";

/// Runs one command example through its MCP route; returns the tool result.
fn run_through_mcp(name: &str, example: &Value, fixture: &str) -> Value {
    let scratch = Scratch::new(&format!("cov-{name}-{}", fixture.replace('/', "_")));
    let file = scratch.put(fixture);
    let mut r = Rig::new(
        &scratch,
        &[
            Scope::Edit,
            Scope::Save,
            Scope::Export,
            Scope::ExecuteExternal,
        ],
        true,
    );
    let id = r.open(&file);
    if matches!(name, "docx_move_tab" | "docx_clear_tab") {
        // These act on a tab stop that has to exist first.
        r.ok(
            "apply_commands",
            json!({"document": id, "commands": [
                {"command": "docx_add_tab", "para": 0, "pos_pt": 36.0}]}),
        );
    }
    let ext = if fixture.starts_with("docx") {
        "docx"
    } else {
        "xlsx"
    };
    match name {
        "save" => r.ok(
            "save_as",
            json!({"document": id, "path": s(&scratch.root().join(format!("saved.{ext}")))}),
        ),
        "export_pdf" => r.ok(
            "export_pdf",
            json!({"document": id, "path": s(&scratch.root().join("out.pdf"))}),
        ),
        "share_send_copy" => r.ok(
            "share_send_copy",
            json!({"document": id, "path": s(&scratch.root().join(format!("copy.{ext}")))}),
        ),
        "share_send_pdf" => r.ok(
            "share_send_pdf",
            json!({"document": id, "path": s(&scratch.root().join("copy.pdf"))}),
        ),
        "share_inspect" => r.ok("share_inspect", json!({"document": id})),
        "share_sanitize_copy" => r.ok(
            "share_sanitize_copy",
            json!({"document": id, "remove": ["comments", "personal_info"],
                "path": s(&scratch.root().join(format!("clean.{ext}")))}),
        ),
        "share_os_sheet" => {
            let made = scratch.root().join(format!("made.{ext}"));
            r.ok("share_send_copy", json!({"document": id, "path": s(&made)}));
            r.ok("share_os_sheet", json!({"document": id, "path": s(&made)}))
        }
        _ => r.ok(
            "apply_commands",
            json!({"document": id, "commands": [example]}),
        ),
    }
}

#[test]
fn every_command_runs_through_mcp() {
    let mut covered = 0;
    for spec in COMMANDS {
        let example = (spec.example)();
        let fixtures: &[&str] = match spec.domain {
            Domain::Docx => &[DOCX],
            Domain::Xlsx => &[XLSX],
            Domain::Any => &[DOCX, XLSX],
        };
        for fixture in fixtures {
            let out = run_through_mcp(spec.name, &example, fixture);
            match spec.name {
                "save" => assert!(out["bytes_written"].as_u64().unwrap_or(0) > 0, "{out}"),
                "export_pdf" => assert!(out["pages"].as_u64().unwrap_or(0) > 0, "{out}"),
                "share_send_copy" | "share_sanitize_copy" => {
                    assert!(out["bytes_written"].as_u64().unwrap_or(0) > 0, "{out}");
                }
                "share_send_pdf" => assert!(out["pages"].as_u64().unwrap_or(0) > 0, "{out}"),
                "share_inspect" => assert!(out["items"].is_array(), "{out}"),
                "share_os_sheet" => assert_eq!(out["performed"], false, "{out}"),
                _ => assert_eq!(out["applied"], 1, "{}: {out}", spec.name),
            }
        }
        covered += 1;
    }
    assert_eq!(covered, COMMANDS.len());
    assert!(covered >= 31, "the command layer has at least 31 commands");
}

#[test]
fn the_catalogue_is_served_by_the_tool_and_the_resource() {
    let scratch = Scratch::new("cov-list");
    let mut r = Rig::new(&scratch, &[], false);
    let tool = r.ok("list_commands", json!({}));
    assert_eq!(tool["count"], COMMANDS.len());
    let names: Vec<&str> = tool["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, COMMANDS.iter().map(|c| c.name).collect::<Vec<_>>());
    let res = r.rpc("resources/read", json!({"uri": "open365://commands"}));
    let text = res["result"]["contents"][0]["text"].as_str().expect("text");
    let parsed: Value = serde_json::from_str(text).expect("json");
    assert_eq!(parsed, tool);
}
