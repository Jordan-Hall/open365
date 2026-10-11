//! The CLI as an agent surface: every command of the command layer runs through `open365 run`,
//! every `--json` output matches the versioned schema, and `save` / `export_pdf` obey the same
//! policy gate as every other command.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("open365-surf-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("root")).expect("scratch");
        std::fs::create_dir_all(dir.join("outside")).expect("outside");
        Self(dir)
    }
    fn root(&self, name: &str) -> PathBuf {
        self.0.join("root").join(name)
    }
    fn outside(&self, name: &str) -> PathBuf {
        self.0.join("outside").join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open365(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_open365"))
        .args(args)
        .output()
        .expect("run open365")
}

fn p(path: &Path) -> &str {
    path.to_str().expect("utf8 path")
}

fn envelope(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| {
        panic!(
            "not JSON ({e}): {} / {}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    })
}

fn audit(o: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

// ---- a small JSON Schema subset: type, required, properties, items, enum, const ----

fn type_ok(v: &Value, t: &str) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "null" => v.is_null(),
        _ => false,
    }
}

fn validate(v: &Value, schema: &Value, at: &str, errors: &mut Vec<String>) {
    if let Some(c) = schema.get("const")
        && v != c
    {
        errors.push(format!("{at}: expected {c}, got {v}"));
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array)
        && !e.contains(v)
    {
        errors.push(format!("{at}: {v} is not one of {e:?}"));
    }
    match schema.get("type") {
        Some(Value::String(t)) if !type_ok(v, t) => errors.push(format!("{at}: not {t}: {v}")),
        Some(Value::Array(ts)) if !ts.iter().filter_map(Value::as_str).any(|t| type_ok(v, t)) => {
            errors.push(format!("{at}: not one of {ts:?}: {v}"));
        }
        _ => {}
    }
    if let Some(obj) = v.as_object() {
        for key in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !obj.contains_key(key) {
                errors.push(format!("{at}: missing {key}"));
            }
        }
        for (key, sub) in schema
            .get("properties")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if let Some(child) = obj.get(key) {
                validate(child, sub, &format!("{at}.{key}"), errors);
            }
        }
    }
    if let (Some(items), Some(sub)) = (v.as_array(), schema.get("items")) {
        for (i, item) in items.iter().enumerate() {
            validate(item, sub, &format!("{at}[{i}]"), errors);
        }
    }
}

fn schemas() -> Value {
    let text = std::fs::read_to_string(fixture("mcp/schemas/cli-v1.json")).expect("schema file");
    serde_json::from_str(&text).expect("schema json")
}

/// Checks an envelope and, when it succeeded, its result against the schema for `key`.
fn check(o: &Output, key: &str) -> Value {
    let all = schemas();
    let env = envelope(o);
    let mut errors = Vec::new();
    validate(&env, &all["envelope"], "envelope", &mut errors);
    assert_eq!(env["ok"], true, "{key}: {env}");
    let schema = all["results"]
        .get(key)
        .unwrap_or_else(|| panic!("no schema for {key}"));
    validate(&env["result"], schema, key, &mut errors);
    assert!(errors.is_empty(), "{key}: {errors:#?}\n{env}");
    env["result"].clone()
}

// ---- coverage ----

const DOCX: &str = "docx/plain-paragraphs.docx";
const XLSX: &str = "xlsx/formulas-basic.xlsx";

#[test]
fn every_command_runs_through_the_cli() {
    let list = check(&open365(&["commands", "--json"]), "commands");
    let commands = list["commands"].as_array().expect("commands");
    assert_eq!(list["count"], commands.len());
    assert!(commands.len() >= 31);
    let scratch = Scratch::new("cover");
    let mut covered = 0;
    for c in commands {
        let name = c["name"].as_str().expect("name");
        let domain = c["domain"].as_str().expect("domain");
        let fixtures: &[&str] = match domain {
            "docx" => &[DOCX],
            "xlsx" => &[XLSX],
            _ => &[DOCX, XLSX],
        };
        for fx in fixtures {
            let ext = if fx.starts_with("docx") {
                "docx"
            } else {
                "xlsx"
            };
            let out = scratch.root(&format!("{name}-{ext}.{ext}"));
            if let Some(sub) = name.strip_prefix("share_") {
                // The share commands have their own subcommands.
                let input = fixture(fx);
                let pdf = scratch.root(&format!("{name}.pdf"));
                let extra: Vec<&str> = match sub {
                    "send_copy" => vec!["send-copy", p(&input), "--to", p(&out)],
                    "send_pdf" => vec!["send-pdf", p(&input), "--to", p(&pdf)],
                    "inspect" => vec!["inspect", p(&input)],
                    "sanitize_copy" => vec![
                        "sanitize-copy",
                        p(&input),
                        "--to",
                        p(&out),
                        "--remove",
                        "comments",
                    ],
                    "os_sheet" => vec!["os-sheet", p(&input), p(&input), "--allow-destructive"],
                    other => panic!("unknown share command {other}"),
                };
                let mut full = vec!["share"];
                full.extend(extra);
                full.push("--json");
                check(&open365(&full), "share");
                continue;
            }
            let mut args = c["example"].clone();
            let obj = args.as_object_mut().expect("example object");
            obj.remove("command");
            match name {
                "save" => {
                    obj.insert("path".into(), json!(p(&out)));
                }
                "export_pdf" => {
                    obj.insert(
                        "path".into(),
                        json!(p(&scratch.root(&format!("{name}.pdf")))),
                    );
                }
                _ => {}
            }
            let mut input = fixture(fx);
            if matches!(name, "docx_move_tab" | "docx_clear_tab") {
                // These act on a tab stop that has to exist first.
                let prepared = scratch.root(&format!("{name}-prepared.docx"));
                let o = open365(&[
                    "run",
                    "docx_add_tab",
                    p(&input),
                    "--args",
                    r#"{"para":0,"pos_pt":36.0}"#,
                    "-o",
                    p(&prepared),
                    "--json",
                ]);
                check(&o, "run");
                input = prepared;
            }
            let o = open365(&[
                "run",
                name,
                p(&input),
                "--args",
                &args.to_string(),
                "-o",
                p(&out),
                "--allow-destructive",
                "--json",
            ]);
            let result = check(&o, "run");
            assert_eq!(result["commands"][0]["command"], name);
            assert_eq!(result["commands"][0]["status"], "applied");
        }
        covered += 1;
    }
    assert_eq!(covered, commands.len());
}

#[test]
fn json_output_of_every_read_command_matches_the_schema() {
    let d = fixture(DOCX);
    let x = fixture(XLSX);
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("inspect", vec!["inspect", p(&d)]),
        ("inspect", vec!["xlsx", "inspect", p(&x)]),
        ("docx outline", vec!["docx", "outline", p(&d)]),
        ("docx text", vec!["docx", "text", p(&d), "--count", "2"]),
        ("docx styles", vec!["docx", "styles", p(&d)]),
        ("layout", vec!["docx", "layout", p(&d)]),
        ("xlsx outline", vec!["xlsx", "outline", p(&x)]),
        (
            "xlsx cells",
            vec!["xlsx", "cells", p(&x), "--sheet", "0", "--range", "A1:C3"],
        ),
        (
            "xlsx formulas",
            vec!["xlsx", "formulas", p(&x), "--sheet", "0"],
        ),
        (
            "xlsx styles",
            vec!["xlsx", "styles", p(&x), "--sheet", "0", "--range", "A1:B2"],
        ),
        ("xlsx recalc", vec!["xlsx", "recalc", p(&x), "--sheet", "0"]),
        ("roundtrip", vec!["roundtrip", "--check", p(&d), p(&x)]),
    ];
    for (key, args) in cases {
        let mut full = args.clone();
        full.push("--json");
        let o = open365(&full);
        check(&o, key);
    }
    // eval
    let o = open365(&[
        "xlsx",
        "eval",
        p(&x),
        "--sheet",
        "0",
        "--cell",
        "A1",
        "--formula",
        "1+2",
        "--json",
    ]);
    check(&o, "xlsx eval");
}

#[test]
fn write_commands_match_the_schema_and_errors_use_the_error_envelope() {
    let scratch = Scratch::new("schema-write");
    let d = fixture(DOCX);
    let pdf = scratch.root("o.pdf");
    let o = open365(&["docx", "export-pdf", p(&d), "-o", p(&pdf), "--json"]);
    check(&o, "export-pdf");
    let made = scratch.root("n.docx");
    let o = open365(&[
        "docx",
        "new-from-template",
        p(&fixture("docx/template-basic.dotx")),
        "-o",
        p(&made),
        "--json",
    ]);
    check(&o, "new-from-template");

    let o = open365(&[
        "xlsx",
        "cells",
        p(&fixture(XLSX)),
        "--sheet",
        "nope",
        "--range",
        "A1",
        "--json",
    ]);
    assert_eq!(o.status.code(), Some(1));
    let env = envelope(&o);
    let mut errors = Vec::new();
    validate(&env, &schemas()["envelope"], "envelope", &mut errors);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"]["code"], "error");
}

#[test]
fn read_commands_leave_hidden_content_out_unless_asked() {
    let hidden = fixture("xlsx/hidden-sheet.xlsx");
    let o = open365(&["xlsx", "outline", p(&hidden), "--json"]);
    let plain = check(&o, "xlsx outline");
    let o = open365(&["xlsx", "outline", p(&hidden), "--hidden", "--json"]);
    let all = check(&o, "xlsx outline");
    assert!(
        all["sheets"].as_array().map(Vec::len) > plain["sheets"].as_array().map(Vec::len),
        "{plain} vs {all}"
    );
}

// ---- the policy gate on save and export_pdf ----

#[test]
fn save_over_an_existing_file_is_denied_without_the_flag_and_audited() {
    let scratch = Scratch::new("gate-save");
    let input = fixture(DOCX);
    let victim = scratch.root("victim.docx");
    std::fs::write(&victim, b"precious").expect("write");
    let args = json!({"path": p(&victim)}).to_string();
    let o = open365(&["run", "save", p(&input), "--args", &args]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(std::fs::read(&victim).expect("read"), b"precious");
    let lines = audit(&o);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["status"], "refused");
    assert_eq!(lines[0]["command"], "save");
    assert!(
        lines[0]["reason"]
            .as_str()
            .is_some_and(|r| r.contains("--allow-destructive"))
    );

    // The JSON form reports the same refusal.
    let o = open365(&["run", "save", p(&input), "--args", &args, "--json"]);
    assert_eq!(o.status.code(), Some(3));
    let env = envelope(&o);
    assert_eq!(env["error"]["code"], "refused");

    // With the flag the operator accepts the replacement.
    let o = open365(&[
        "run",
        "save",
        p(&input),
        "--args",
        &args,
        "--allow-destructive",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_ne!(std::fs::read(&victim).expect("read"), b"precious");

    // A new file needs no flag.
    let fresh = scratch.root("fresh.docx");
    let args = json!({"path": p(&fresh)}).to_string();
    let o = open365(&["run", "save", p(&input), "--args", &args]);
    assert!(o.status.success());
    assert!(fresh.exists());
}

#[test]
fn export_pdf_is_fenced_to_the_roots_and_replacing_needs_the_flag() {
    let scratch = Scratch::new("gate-pdf");
    let input = fixture(DOCX);
    let root = p(&scratch.0.join("root")).to_owned();
    let outside = scratch.outside("x.pdf");
    let args = json!({"path": p(&outside)}).to_string();
    // Outside the allowed roots: denied, even with --allow-destructive.
    for flags in [vec![], vec!["--allow-destructive"]] {
        let mut argv = vec![
            "run",
            "export_pdf",
            p(&input),
            "--args",
            &args,
            "--root",
            &root,
        ];
        argv.extend(flags);
        let o = open365(&argv);
        assert_eq!(
            o.status.code(),
            Some(3),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(!outside.exists());
        let lines = audit(&o);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["status"], "refused");
        assert!(
            lines[0]["reason"]
                .as_str()
                .is_some_and(|r| r.contains("outside")),
            "{lines:?}"
        );
    }
    // A traversal that starts inside the root is denied the same way.
    let sneaky = format!("{root}/../outside/y.pdf");
    let args = json!({"path": sneaky}).to_string();
    let o = open365(&[
        "run",
        "export_pdf",
        p(&input),
        "--args",
        &args,
        "--root",
        &root,
    ]);
    assert_eq!(o.status.code(), Some(3));
    assert!(!scratch.outside("y.pdf").exists());

    // Replacing an existing PDF inside the root needs the flag.
    let inside = scratch.root("in.pdf");
    std::fs::write(&inside, b"precious").expect("write");
    let args = json!({"path": p(&inside), "overwrite": true}).to_string();
    let o = open365(&[
        "run",
        "export_pdf",
        p(&input),
        "--args",
        &args,
        "--root",
        &root,
    ]);
    assert_eq!(o.status.code(), Some(3));
    assert_eq!(std::fs::read(&inside).expect("read"), b"precious");
    assert_eq!(audit(&o)[0]["status"], "refused");
    let o = open365(&[
        "run",
        "export_pdf",
        p(&input),
        "--args",
        &args,
        "--root",
        &root,
        "--allow-destructive",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_ne!(std::fs::read(&inside).expect("read"), b"precious");
}

#[test]
fn run_refuses_destructive_commands_without_the_flag() {
    let scratch = Scratch::new("gate-run");
    let input = fixture(DOCX);
    let out = scratch.root("o.docx");
    let args = r#"{"start":{"para":0,"offset":0},"end":{"para":0,"offset":2}}"#;
    let o = open365(&[
        "run",
        "docx_delete_range",
        p(&input),
        "--args",
        args,
        "-o",
        p(&out),
    ]);
    assert_eq!(o.status.code(), Some(3));
    assert!(!out.exists());
    assert_eq!(audit(&o)[0]["status"], "refused");
}

#[test]
fn unknown_commands_and_bad_arguments_are_errors() {
    let input = fixture(DOCX);
    let o = open365(&["run", "format_disk", p(&input), "--json"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(envelope(&o)["ok"], false);
    let o = open365(&[
        "run",
        "docx_insert_text",
        p(&input),
        "--args",
        "{\"para\": \"x\"}",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(o.status.code(), Some(1));
}

const INSERT_CMD: &str = r#"[{"command":"docx_insert_text","para":0,"offset":0,"text":"XY"}]"#;

#[test]
fn apply_and_run_output_files_obey_the_same_gate_as_save() {
    let scratch = Scratch::new("gate-output");
    let input = fixture(DOCX);
    let cmds = scratch.root("c.json");
    std::fs::write(&cmds, INSERT_CMD).expect("write");
    let root = p(&scratch.0.join("root")).to_owned();
    let existing = scratch.root("existing.docx");
    std::fs::write(&existing, b"precious").expect("write");

    // Replacing an existing output needs --allow-destructive, for `apply -o` and `run -o`.
    let o = open365(&["apply", p(&cmds), p(&input), "-o", p(&existing)]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(std::fs::read(&existing).expect("read"), b"precious");
    let lines = audit(&o);
    assert!(
        lines
            .iter()
            .any(|l| l["command"] == "save" && l["status"] == "refused"),
        "{lines:?}"
    );
    let o = open365(&[
        "run",
        "docx_insert_text",
        p(&input),
        "--args",
        r#"{"para":0,"offset":0,"text":"XY"}"#,
        "-o",
        p(&existing),
    ]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(std::fs::read(&existing).expect("read"), b"precious");

    // A new output is fine; the flag lets the replacement through.
    let fresh = scratch.root("fresh.docx");
    let o = open365(&["apply", p(&cmds), p(&input), "-o", p(&fresh)]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = open365(&[
        "apply",
        p(&cmds),
        p(&input),
        "-o",
        p(&existing),
        "--allow-destructive",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_ne!(std::fs::read(&existing).expect("read"), b"precious");

    // `-o` is fenced to --root, and --allow-destructive does not lift the fence.
    let outside = scratch.outside("o.docx");
    for flags in [vec![], vec!["--allow-destructive"]] {
        let mut argv = vec![
            "apply",
            p(&cmds),
            p(&input),
            "-o",
            p(&outside),
            "--root",
            &root,
        ];
        argv.extend(flags.clone());
        let o = open365(&argv);
        assert_eq!(
            o.status.code(),
            Some(3),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(!outside.exists());
        let mut argv = vec![
            "run",
            "docx_insert_text",
            p(&input),
            "--args",
            r#"{"para":0,"offset":0,"text":"XY"}"#,
            "-o",
            p(&outside),
            "--root",
            &root,
        ];
        argv.extend(flags);
        let o = open365(&argv);
        assert_eq!(
            o.status.code(),
            Some(3),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(!outside.exists());
    }
}

#[test]
fn a_relative_save_path_is_written_where_it_was_checked() {
    let scratch = Scratch::new("gate-relative");
    let input = fixture(DOCX);
    let cwd = scratch.0.join("cwd");
    std::fs::create_dir_all(&cwd).expect("cwd");
    let root = p(&scratch.0.join("root")).to_owned();
    let o = Command::new(env!("CARGO_BIN_EXE_open365"))
        .current_dir(&cwd)
        .args([
            "run",
            "save",
            p(&input),
            "--args",
            r#"{"path":"rel.docx"}"#,
            "--root",
            &root,
        ])
        .output()
        .expect("run");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    // The fence resolves a relative path against the first root: that is where the file goes.
    assert!(scratch.root("rel.docx").exists());
    assert!(
        !cwd.join("rel.docx").exists(),
        "written to the working directory instead"
    );
}

#[test]
fn new_from_template_is_fenced_audited_and_replaces_only_with_overwrite() {
    let scratch = Scratch::new("gate-template");
    let tpl = fixture("docx/template-basic.dotx");
    let root = p(&scratch.0.join("root")).to_owned();
    let outside = scratch.outside("n.docx");
    let o = open365(&[
        "docx",
        "new-from-template",
        p(&tpl),
        "-o",
        p(&outside),
        "--root",
        &root,
    ]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!outside.exists());
    let lines = audit(&o);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["status"], "refused");

    let inside = scratch.root("n.docx");
    let o = open365(&[
        "docx",
        "new-from-template",
        p(&tpl),
        "-o",
        p(&inside),
        "--root",
        &root,
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(inside.exists());
    assert_eq!(audit(&o)[0]["status"], "applied");
    // No overwrite flag: the existing file stays.
    std::fs::write(&inside, b"precious").expect("write");
    let o = open365(&[
        "docx",
        "new-from-template",
        p(&tpl),
        "-o",
        p(&inside),
        "--root",
        &root,
    ]);
    assert!(!o.status.success());
    assert_eq!(std::fs::read(&inside).expect("read"), b"precious");
    let o = open365(&[
        "docx",
        "new-from-template",
        p(&tpl),
        "-o",
        p(&inside),
        "--root",
        &root,
        "--overwrite",
    ]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(std::fs::read(&inside).expect("read"), b"precious");
    let o = open365(&[
        "docx",
        "new-from-template",
        p(&tpl),
        "-o",
        p(&inside),
        "--root",
        &root,
        "--overwrite",
        "--allow-destructive",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_ne!(std::fs::read(&inside).expect("read"), b"precious");
}

#[test]
fn overwriting_a_filled_cell_needs_the_flag() {
    let scratch = Scratch::new("gate-cell");
    let input = fixture(XLSX);
    let out = scratch.root("o.xlsx");
    // A1 of the fixture holds data: setting it is a destructive write.
    let args = r#"{"sheet":0,"row":1,"col":1,"value":{"type":"number","value":41}}"#;
    let o = open365(&[
        "run",
        "xlsx_set_cell_value",
        p(&input),
        "--args",
        args,
        "-o",
        p(&out),
    ]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!out.exists());
    let o = open365(&[
        "run",
        "xlsx_set_cell_value",
        p(&input),
        "--args",
        args,
        "-o",
        p(&out),
        "--allow-destructive",
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
