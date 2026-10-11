//! Authorisation for tool calls: scope checks, the confirmation hook, and the `request_scope`
//! tool. The agent never supplies a grant or a confirmation; both come from the operator.

use serde_json::{Value, json};

use crate::scope::{
    Confirm, ConfirmKind, ConfirmRequest, Decision, Scope, sanitize, sanitize_with,
};
use crate::server::Server;
use crate::tools::{Detail, ToolError, str_arg};

impl Server {
    /// The scopes this session holds now.
    pub fn granted(&self) -> Vec<Scope> {
        self.grants.iter().copied().collect()
    }

    /// Adds a scope to this session. For the embedding host, never reachable from a tool call.
    pub fn grant(&mut self, scope: Scope) {
        self.grants.insert(scope);
    }

    /// Removes a scope from this session. `read` cannot be removed.
    pub fn revoke(&mut self, scope: Scope) {
        if scope != Scope::Read {
            self.grants.remove(&scope);
        }
    }

    /// Replaces the confirmation hook (the default follows `Config::allow_destructive`).
    pub fn set_confirmer(&mut self, confirmer: Box<dyn Confirm>) {
        self.confirm = confirmer;
    }

    pub(crate) fn holds(&self, scope: Scope) -> bool {
        self.grants.contains(&scope)
    }

    /// Fails unless the session holds `scope`.
    pub(crate) fn require(&self, scope: Scope) -> Result<(), ToolError> {
        if self.holds(scope) {
            return Ok(());
        }
        Err(ToolError::refused(match scope {
            Scope::ExecuteExternal => {
                "scope execute-external is not granted; only the operator can grant it at start-up \
                 (--grant execute-external)"
                    .to_owned()
            }
            other => format!(
                "scope {other} is not granted; editing is disabled; start the server with \
                 --allow-edit or --grant {other}"
            ),
        })
        .coded("scope_not_granted"))
    }

    /// Whether a dry run should report a destructive action as permitted.
    pub(crate) fn pre_approved(&self) -> bool {
        self.confirm.pre_approves(ConfirmKind::Destructive)
    }

    /// Asks the confirmation hook to approve a destructive action. Recorded in the audit line
    /// whatever the answer.
    pub(crate) fn confirm_destructive(
        &mut self,
        tool: &str,
        summary: &str,
        detail: &mut Detail,
    ) -> Result<(), ToolError> {
        self.ask(ConfirmKind::Destructive, tool, summary, detail)
    }

    fn ask(
        &mut self,
        kind: ConfirmKind,
        tool: &str,
        summary: &str,
        detail: &mut Detail,
    ) -> Result<(), ToolError> {
        let request = ConfirmRequest {
            kind,
            tool: tool.to_owned(),
            summary: sanitize(summary),
        };
        let kind_name = match kind {
            ConfirmKind::Destructive => "destructive",
            ConfirmKind::ScopeRequest => "scope_request",
        };
        match self.confirm.confirm(&request) {
            Decision::Allow { by } => {
                detail.insert(
                    "confirmation".to_owned(),
                    json!({"kind": kind_name, "decision": "allow", "by": by}),
                );
                Ok(())
            }
            Decision::Deny { reason } => {
                detail.insert(
                    "confirmation".to_owned(),
                    json!({"kind": kind_name, "decision": "deny", "reason": sanitize(&reason)}),
                );
                Err(ToolError::refused(match kind {
                    ConfirmKind::Destructive => format!(
                        "destructive effects need confirmation, which was not given ({}); start \
                         the server with --allow-destructive or --confirm terminal",
                        sanitize(&reason)
                    ),
                    ConfirmKind::ScopeRequest => {
                        format!("the scope request was not approved ({})", sanitize(&reason))
                    }
                })
                .coded("confirmation_denied"))
            }
        }
    }

    /// `request_scope {scope, reason?}`: the agent asks for a scope; the operator decides.
    pub(crate) fn request_scope(
        &mut self,
        args: &Value,
        detail: &mut Detail,
    ) -> Result<Value, ToolError> {
        detail.insert("effect".to_owned(), json!("read"));
        let name = str_arg(args, "scope")?;
        let scope = Scope::parse(name).ok_or_else(|| {
            ToolError::invalid(format!("unknown scope {}", sanitize(name))).coded("unknown_scope")
        })?;
        detail.insert("requested_scope".to_owned(), json!(scope.name()));
        if self.holds(scope) {
            return Ok(json!({"scope": scope.name(), "granted": true, "already_held": true}));
        }
        if scope == Scope::ExecuteExternal {
            return Err(ToolError::refused(
                "execute-external is never granted at an agent's request; the operator names it \
                 at start-up (--grant execute-external)",
            )
            .coded("scope_not_requestable"));
        }
        // The agent's reason is untrusted text: shown only quoted, labelled, cut short and with
        // control, format and separator characters and quotation marks replaced, so it cannot
        // pass for part of the prompt. It is never written to the audit log.
        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .map(|r| sanitize_with(&r.replace('"', "'"), 120));
        let summary = match reason {
            Some(r) if !r.is_empty() => format!(
                "grant scope {scope} for this session. The agent's own reason, unverified and \
                 untrusted: \"{r}\""
            ),
            _ => format!("grant scope {scope} for this session. The agent gave no reason"),
        };
        self.ask(ConfirmKind::ScopeRequest, "request_scope", &summary, detail)?;
        self.grants.insert(scope);
        Ok(json!({"scope": scope.name(), "granted": true, "already_held": false}))
    }
}
