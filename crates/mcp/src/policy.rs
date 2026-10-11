//! The one place that decides what a command needs: its effect class, the scope that covers
//! it, and whether it needs a confirmation. The MCP tools and the CLI both call [`assess`], so
//! the two surfaces cannot disagree about what is destructive.
//!
//! The rules (ADR-002):
//! - `Read` needs the `read` scope.
//! - `Edit` and `Destructive` commands need `edit`; `Egress` needs `execute-external`.
//! - `save` needs `save`; `export_pdf` and new files from a template need `export`.
//! - A command needs confirmation when it is destructive or egress, or when it replaces a file
//!   that already exists (`save` to any existing path, the opened file included, and
//!   `export_pdf` with `overwrite`).
//! - Destructive means destructive by actual effect: [`assess_in`] classifies a command against
//!   the state it will run on, so overwriting a non-empty cell, pasting over data and clearing
//!   existing formats are destructive, not edits. [`assess_sequence`] does so for a batch, each
//!   command against the state left by the ones before it.

use std::path::Path;

use open365_commands::{Command, CommandError, EffectClass, Outcome, Target};

use crate::scope::Scope;

/// What a command needs before it may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Risk {
    pub effect: EffectClass,
    pub scope: Scope,
    /// The operator must confirm (CLI: pass `--allow-destructive`; MCP: the confirmation hook).
    pub confirm: bool,
}

/// Classifies `command` from the command alone, for a document opened from `current_path`.
/// Looks at the file system only to see whether a target file already exists. It cannot see
/// what a cell holds: any edit that depends on document state must use [`assess_in`].
pub fn assess(command: &Command, current_path: Option<&Path>) -> Risk {
    risk_of(command, command.effect(current_path))
}

/// Classifies `command` by its actual effect on `target` as it is now.
pub fn assess_in(command: &Command, target: &Target) -> Risk {
    risk_of(command, command.effect_in(target))
}

fn risk_of(command: &Command, effect: EffectClass) -> Risk {
    match command {
        Command::Save { path } => Risk {
            effect,
            scope: Scope::Save,
            confirm: path.exists(),
        },
        Command::ExportPdf { path, overwrite } => Risk {
            effect,
            scope: Scope::Export,
            confirm: *overwrite && path.exists(),
        },
        _ => Risk {
            effect,
            scope: match effect {
                EffectClass::Read => Scope::Read,
                EffectClass::Egress => Scope::ExecuteExternal,
                EffectClass::Edit | EffectClass::Destructive | EffectClass::Export => Scope::Edit,
            },
            confirm: matches!(effect, EffectClass::Destructive | EffectClass::Egress),
        },
    }
}

/// One command of a batch: its risk and, when it was run on the scratch copy, the result.
#[derive(Debug)]
pub struct Step {
    pub risk: Risk,
    /// `None` for `save` and `export_pdf` (never run on the scratch copy, they write files) and
    /// for commands after the first one that failed.
    pub outcome: Option<Result<Outcome, CommandError>>,
}

/// Classifies a batch as it will run: each command against the state the earlier ones leave.
/// `scratch` must be a throwaway copy of the real target, in the same state; the commands are
/// applied to it. Run the batch for real only if every step is permitted. The first command that
/// fails ends the batch, so the later steps are classified against the state before it.
pub fn assess_sequence(scratch: &mut Target, commands: &[Command]) -> Vec<Step> {
    let mut failed = false;
    commands
        .iter()
        .map(|command| {
            let risk = assess_in(command, scratch);
            let runnable =
                !failed && !matches!(command, Command::Save { .. } | Command::ExportPdf { .. });
            let outcome = runnable.then(|| scratch.apply(command.clone()));
            failed |= matches!(outcome, Some(Err(_)));
            Step { risk, outcome }
        })
        .collect()
}

/// The risk of creating a file from a template at `path`.
pub fn assess_new_file(path: &Path, overwrite: bool) -> Risk {
    Risk {
        effect: EffectClass::Export,
        scope: Scope::Export,
        confirm: overwrite && path.exists(),
    }
}

/// The CLI's decision: the user is the operator, so scopes do not apply, but anything that
/// needs confirmation is refused unless `--allow-destructive` was given.
///
/// # Errors
/// The refusal reason.
pub fn cli_decision(risk: &Risk, allow_destructive: bool) -> Result<(), String> {
    if risk.confirm && !allow_destructive {
        return Err(format!(
            "a {} command that destroys or replaces content needs --allow-destructive",
            crate::tools::effect_name(risk.effect)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn save(path: &Path) -> Command {
        Command::Save {
            path: path.to_path_buf(),
        }
    }

    #[test]
    fn saving_over_any_existing_file_is_confirmed_and_over_a_new_one_is_not() {
        let dir = std::env::temp_dir().join(format!("open365-policy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let own = dir.join("own.docx");
        let other = dir.join("other.docx");
        std::fs::write(&own, b"x").expect("write");
        std::fs::write(&other, b"x").expect("write");
        let r = assess(&save(&own), Some(&own));
        assert_eq!((r.effect, r.confirm), (EffectClass::Destructive, true));
        let r = assess(&save(&other), Some(&own));
        assert_eq!((r.effect, r.confirm), (EffectClass::Export, true));
        let r = assess(&save(&dir.join("new.docx")), Some(&own));
        assert!(!r.confirm);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_over_an_existing_file_is_confirmed_only_with_overwrite() {
        let file: PathBuf =
            std::env::temp_dir().join(format!("open365-pol2-{}.pdf", std::process::id()));
        std::fs::write(&file, b"x").expect("write");
        let cmd = |overwrite| Command::ExportPdf {
            path: file.clone(),
            overwrite,
        };
        assert!(!assess(&cmd(false), None).confirm);
        assert!(assess(&cmd(true), None).confirm);
        assert!(cli_decision(&assess(&cmd(true), None), false).is_err());
        assert!(cli_decision(&assess(&cmd(true), None), true).is_ok());
        let _ = std::fs::remove_file(&file);
    }
}
