//! Permission scopes, per-session grants and the confirmation hook (ADR-002).
//!
//! A scope names a class of action. A session holds a set of granted scopes; `read` is always
//! held. Scopes come from the operator (start-up flags or the embedding host) and can be widened
//! during the session only through the [`Confirm`] hook, which stands for a human at the
//! operator's own terminal or UI. An agent can ask (the `request_scope` tool) but cannot grant.
//! `execute-external` is reserved for commands that send data off the machine; it is never held
//! unless the operator names it at start-up, and an agent request for it is always refused.

use std::collections::BTreeSet;
use std::fmt;
use std::io::{BufRead, BufReader, Write};

/// A class of action a session may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// Open documents and read from them. Always held.
    Read,
    /// Change the open document in memory (`apply_commands`).
    Edit,
    /// Write the document to disk (`save_as`).
    Save,
    /// Create other files from a document: PDF export, new documents from templates.
    Export,
    /// Send data off this machine. Never held by default; the operator names it at start-up.
    ExecuteExternal,
}

impl Scope {
    pub const ALL: [Scope; 5] = [
        Scope::Read,
        Scope::Edit,
        Scope::Save,
        Scope::Export,
        Scope::ExecuteExternal,
    ];

    /// The stable name used in flags, tool arguments and the audit log.
    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Save => "save",
            Self::Export => "export",
            Self::ExecuteExternal => "execute-external",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == text)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What a confirmation is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmKind {
    /// A destructive command or the replacement of an existing file.
    Destructive,
    /// The agent asked for a scope it does not hold.
    ScopeRequest,
}

/// What the person is asked to approve. `summary` has been stripped of control characters and
/// cut to a safe length; it never contains document content.
#[derive(Debug, Clone)]
pub struct ConfirmRequest {
    pub kind: ConfirmKind,
    pub tool: String,
    pub summary: String,
}

/// The answer to a confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Approved; `by` says who or what (`operator-flag`, `terminal`, ...), for the audit log.
    Allow {
        by: String,
    },
    Deny {
        reason: String,
    },
}

/// The confirmation hook. The server calls it before a destructive action and before widening
/// the session's scopes, and never takes the answer from the agent.
pub trait Confirm: Send + fmt::Debug {
    fn confirm(&mut self, request: &ConfirmRequest) -> Decision;

    /// Whether `kind` is approved without asking, so a dry run can report it as permitted.
    fn pre_approves(&self, _kind: ConfirmKind) -> bool {
        false
    }
}

/// Refuses everything. The default when the operator gave neither a flag nor a terminal.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyAll;

impl Confirm for DenyAll {
    fn confirm(&mut self, _request: &ConfirmRequest) -> Decision {
        Decision::Deny {
            reason: "no confirmation channel is configured".to_owned(),
        }
    }
}

/// The start-up `--allow-destructive` flag: the operator approved destructive actions once,
/// for the whole session. It never approves a scope request.
#[derive(Debug, Default, Clone, Copy)]
pub struct OperatorFlag;

impl Confirm for OperatorFlag {
    fn confirm(&mut self, request: &ConfirmRequest) -> Decision {
        match request.kind {
            ConfirmKind::Destructive => Decision::Allow {
                by: "operator-flag".to_owned(),
            },
            ConfirmKind::ScopeRequest => Decision::Deny {
                reason: "scope requests need a confirmation channel (--confirm terminal)"
                    .to_owned(),
            },
        }
    }

    fn pre_approves(&self, kind: ConfirmKind) -> bool {
        kind == ConfirmKind::Destructive
    }
}

/// Asks on the operator's controlling terminal (`CONIN$`/`CONOUT$` on Windows, `/dev/tty`
/// elsewhere), which is separate from the JSON-RPC pipe the agent uses. With no terminal the
/// answer is a refusal.
#[derive(Debug, Default, Clone, Copy)]
pub struct TerminalConfirm {
    /// The operator also passed `--allow-destructive`: destructive actions are approved in
    /// advance, and only scope requests are asked on the terminal.
    pub preapprove_destructive: bool,
}

#[cfg(windows)]
const TTY_IN: &str = "CONIN$";
#[cfg(windows)]
const TTY_OUT: &str = "CONOUT$";
#[cfg(not(windows))]
const TTY_IN: &str = "/dev/tty";
#[cfg(not(windows))]
const TTY_OUT: &str = "/dev/tty";

impl Confirm for TerminalConfirm {
    fn pre_approves(&self, kind: ConfirmKind) -> bool {
        self.preapprove_destructive && kind == ConfirmKind::Destructive
    }

    fn confirm(&mut self, request: &ConfirmRequest) -> Decision {
        if self.pre_approves(request.kind) {
            return Decision::Allow {
                by: "operator-flag".to_owned(),
            };
        }
        let deny = |why: &str| Decision::Deny {
            reason: why.to_owned(),
        };
        let Ok(mut out) = std::fs::OpenOptions::new().write(true).open(TTY_OUT) else {
            return deny("no terminal to ask on");
        };
        let Ok(input) = std::fs::OpenOptions::new().read(true).open(TTY_IN) else {
            return deny("no terminal to ask on");
        };
        let what = match request.kind {
            ConfirmKind::Destructive => "destructive action",
            ConfirmKind::ScopeRequest => "scope request",
        };
        if write!(
            out,
            "\nOpen365: an agent wants a {what} ({}): {}\nAllow? [y/N] ",
            request.tool, request.summary
        )
        .and_then(|()| out.flush())
        .is_err()
        {
            return deny("no terminal to ask on");
        }
        let mut answer = String::new();
        if BufReader::new(input).read_line(&mut answer).is_err() {
            return deny("no answer from the terminal");
        }
        if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            Decision::Allow {
                by: "terminal".to_owned(),
            }
        } else {
            deny("declined at the terminal")
        }
    }
}

/// Longest summary shown to the person, in characters.
const MAX_SUMMARY: usize = 300;

/// Whether `c` must not reach a terminal or a log as itself: a control character (Cc), a
/// format character (Cf: bidirectional overrides such as U+202E, zero-width and invisible
/// characters, the BOM, tag characters), a line or paragraph separator (Zl, Zp) or a
/// space other than U+0020 (Zs), which can pad a line to hide text.
fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            u32::from(c),
            0x00A0 | 0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891
                | 0x08E2 | 0x1680 | 0x180E | 0x2000..=0x200F | 0x2028..=0x202F
                | 0x205F..=0x2064 | 0x2066..=0x206F | 0x3000 | 0xFEFF | 0xFFF9..=0xFFFB
                | 0x110BD | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3
                | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F
        )
}

/// Makes agent-controlled text safe to print on a terminal or write to a log: unsafe
/// characters (see `is_unsafe_char`) become `?` and the text is cut to a fixed length.
pub fn sanitize(text: &str) -> String {
    sanitize_with(text, MAX_SUMMARY)
}

/// [`sanitize`] with a chosen length limit in characters.
pub(crate) fn sanitize_with(text: &str, max: usize) -> String {
    let mut out: String = text
        .chars()
        .map(|c| if is_unsafe_char(c) { '?' } else { c })
        .take(max)
        .collect();
    if text.chars().count() > max {
        out.push_str("...");
    }
    out
}

/// The scopes a session starts with: `read`, the operator's named scopes and, for the legacy
/// `allow_edit` switch, `edit`, `save` and `export`.
pub fn initial_grants(named: &BTreeSet<Scope>, allow_edit: bool) -> BTreeSet<Scope> {
    let mut grants = named.clone();
    grants.insert(Scope::Read);
    if allow_edit {
        grants.extend([Scope::Edit, Scope::Save, Scope::Export]);
    }
    grants
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for s in Scope::ALL {
            assert_eq!(Scope::parse(s.name()), Some(s));
        }
        assert_eq!(Scope::parse("root"), None);
    }

    #[test]
    fn sanitize_removes_control_characters_and_cuts() {
        assert_eq!(sanitize("a\u{1b}[31mb\nc"), "a?[31mb?c");
        assert_eq!(sanitize(&"x".repeat(400)).chars().count(), MAX_SUMMARY + 3);
    }

    #[test]
    fn sanitize_removes_format_and_separator_characters() {
        // Right-to-left override, zero-width space, BOM, line and paragraph separators,
        // a no-break space, a tag character and an isolate.
        let tricky = "a\u{202E}b\u{200B}c\u{FEFF}d\u{2028}e\u{2029}f\u{00A0}g\u{E0041}h\u{2066}i";
        assert_eq!(sanitize(tricky), "a?b?c?d?e?f?g?h?i");
        assert!(sanitize("plain text, ünïcode ok").contains("ünïcode"));
    }

    #[test]
    fn operator_flag_never_approves_a_scope_request() {
        let mut flag = OperatorFlag;
        let ask = |kind| ConfirmRequest {
            kind,
            tool: "t".to_owned(),
            summary: String::new(),
        };
        assert!(matches!(
            flag.confirm(&ask(ConfirmKind::Destructive)),
            Decision::Allow { .. }
        ));
        assert!(matches!(
            flag.confirm(&ask(ConfirmKind::ScopeRequest)),
            Decision::Deny { .. }
        ));
    }
}
