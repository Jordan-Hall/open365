//! Workspace automation. `cargo xtask ci` runs every check CI runs.

use std::collections::BTreeMap;
use std::env;
use std::process::{Command, ExitCode};

use cargo_metadata::{DependencyKind, MetadataCommand};

/// Architectural layer of each workspace crate. A crate may depend (normal or
/// build dependency) only on crates in the same or a lower layer. Dev
/// dependencies are exempt so tests can use `open365-testkit`.
const LAYERS: &[(&str, u8)] = &[
    ("open365-locale", 0),
    ("open365-opc", 0),
    ("open365-docx", 1),
    ("open365-xlsx", 1),
    ("open365-calc", 2),
    ("open365-commands", 2),
    ("open365-text", 3),
    ("open365-layout", 3),
    ("open365-render", 4),
    ("open365-pdf", 4),
    ("open365-mcp", 5),
    ("open365-testkit", 5),
    ("open365-ui-egui", 5),
    ("open365-desktop", 6),
    ("open365-cli", 6),
    ("xtask", 6),
];

fn main() -> ExitCode {
    let task = env::args().nth(1);
    let result = match task.as_deref() {
        Some("ci") => ci(),
        Some("layers") => check_layers(),
        _ => Err("usage: cargo xtask <ci|layers>".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn ci() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--workspace"])?;
    check_layers()?;
    deny()
}

fn cargo(args: &[&str]) -> Result<(), String> {
    println!("==> cargo {}", args.join(" "));
    let status = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()))
        .args(args)
        .status()
        .map_err(|e| format!("failed to run cargo {}: {e}", args.join(" ")))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed", args.join(" ")))
    }
}

/// Runs `cargo deny check`. Missing cargo-deny is fatal in CI and a warning locally.
fn deny() -> Result<(), String> {
    let installed = Command::new("cargo")
        .args(["deny", "--version"])
        .output()
        .is_ok_and(|o| o.status.success());
    if installed {
        return cargo(&["deny", "check"]);
    }
    if env::var_os("CI").is_some() {
        return Err("cargo-deny is required in CI".to_owned());
    }
    println!("==> SKIPPED cargo deny check (not installed: `cargo install --locked cargo-deny`)");
    Ok(())
}

fn check_layers() -> Result<(), String> {
    println!("==> layering check");
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .map_err(|e| format!("cargo metadata failed: {e}"))?;
    let members: Vec<&str> = metadata
        .workspace_packages()
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    let edges: Vec<(&str, &str)> = metadata
        .workspace_packages()
        .iter()
        .flat_map(|package| {
            package
                .dependencies
                .iter()
                .filter(|d| d.kind != DependencyKind::Development)
                .filter(|d| members.contains(&d.name.as_str()))
                .map(|d| (package.name.as_str(), d.name.as_str()))
        })
        .collect();
    let violations = layer_violations(&members, &edges);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "layering violations:\n  {}",
            violations.join("\n  ")
        ))
    }
}

/// Returns one message per crate without a layer and per dependency that points upward.
fn layer_violations(members: &[&str], edges: &[(&str, &str)]) -> Vec<String> {
    let layers: BTreeMap<&str, u8> = LAYERS.iter().copied().collect();
    let mut violations: Vec<String> = members
        .iter()
        .filter(|m| !layers.contains_key(*m))
        .map(|m| format!("{m} has no layer; add it to LAYERS in xtask/src/main.rs"))
        .collect();
    for (from, to) in edges {
        if let (Some(f), Some(t)) = (layers.get(from), layers.get(to))
            && t > f
        {
            violations.push(format!(
                "{from} (layer {f}) must not depend on {to} (layer {t})"
            ));
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::layer_violations;

    #[test]
    fn rejects_format_crate_depending_on_render() {
        let violations = layer_violations(
            &["open365-docx", "open365-render"],
            &[("open365-docx", "open365-render")],
        );
        assert_eq!(
            violations,
            ["open365-docx (layer 1) must not depend on open365-render (layer 4)"]
        );
    }

    #[test]
    fn allows_downward_and_same_layer_dependencies() {
        let violations = layer_violations(
            &["open365-commands", "open365-docx", "open365-calc"],
            &[
                ("open365-commands", "open365-docx"),
                ("open365-commands", "open365-calc"),
            ],
        );
        assert!(violations.is_empty());
    }

    #[test]
    fn rejects_crate_without_layer() {
        assert_eq!(
            layer_violations(&["open365-mystery"], &[]),
            ["open365-mystery has no layer; add it to LAYERS in xtask/src/main.rs"]
        );
    }
}
