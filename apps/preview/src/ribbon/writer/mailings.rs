//! Writer ribbon: the Mailings tab (`mailings.*`): Create (envelopes, labels), Start Mail Merge,
//! Write & Insert Fields, Preview Results and Finish. The work is in the pane
//! (`doc/writer/ext_mailings.rs`) and the command layer (`open365_commands::mailings_ops`).
//!
//! A drop-down item has the id `<control>.<item>`; a typed value follows `=`
//! (`mailings.preview-results.record=3`, `mailings.preview-results.find-recipient=Ada`,
//! `mailings.write-insert-fields.match-fields=_FIRST0_:Given`). The menus are listed by
//! [`menu_items`]. The pane has no dialog host yet, so the controls Word answers with a dialog
//! take Word's default values (Size 10 envelope, Avery 5160 labels, the default address block).
//!
//! Not wired: Choose from Outlook Contacts, the Step-by-Step wizard, Print Documents and Send
//! Email Messages (no printing or mail transport). The merge state is not saved in the file.

use open365_commands::mailings_ops::{LabelDef, MergeKind};
use open365_ui_core::MenuItem;

use crate::doc::WriterPane;
use crate::ribbon::{Handled, RibbonCtx, base};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "mailings.create.envelopes",
    "mailings.create.labels",
    "mailings.start-mail-merge.start-mail-merge",
    "mailings.start-mail-merge.select-recipients",
    "mailings.start-mail-merge.edit-recipient-list",
    "mailings.write-insert-fields.highlight-merge-fields",
    "mailings.write-insert-fields.address-block",
    "mailings.write-insert-fields.greeting-line",
    "mailings.write-insert-fields.insert-merge-field",
    "mailings.write-insert-fields.rules",
    "mailings.write-insert-fields.match-fields",
    "mailings.write-insert-fields.update-labels",
    "mailings.preview-results.preview-results",
    "mailings.preview-results.first",
    "mailings.preview-results.previous",
    "mailings.preview-results.record",
    "mailings.preview-results.next",
    "mailings.preview-results.find-recipient",
    "mailings.preview-results.check-for-errors",
    "mailings.preview-results.last",
    "mailings.finish.finish-merge",
];

const START: &str = "mailings.start-mail-merge.start-mail-merge";
const SELECT: &str = "mailings.start-mail-merge.select-recipients";
const FIELD: &str = "mailings.write-insert-fields.insert-merge-field";
const RULES: &str = "mailings.write-insert-fields.rules";
const FINISH: &str = "mailings.finish.finish-merge";

fn item(control: &str, id: &str, label: &str) -> MenuItem {
    MenuItem::new(&format!("{control}.{id}"), label)
}

fn disabled(control: &str, id: &str, label: &str) -> MenuItem {
    MenuItem {
        enabled: false,
        ..item(control, id, label)
    }
}

/// The drop-down of Mailings control `control`, `None` for a control without one.
#[must_use]
pub fn menu_items(control: &str) -> Option<Vec<MenuItem>> {
    let c = control;
    Some(match c {
        START => vec![
            item(c, "letters", "Letters"),
            item(c, "e-mail-messages", "E-mail Messages"),
            item(c, "envelopes", "Envelopes..."),
            item(c, "labels", "Labels..."),
            item(c, "directory", "Directory"),
            MenuItem::separator(),
            item(c, "normal-document", "Normal Document"),
        ],
        SELECT => vec![
            item(c, "type-a-new-list", "Type a New List..."),
            item(c, "use-an-existing-list", "Use an Existing List..."),
            disabled(c, "outlook-contacts", "Choose from Outlook Contacts..."),
        ],
        RULES => vec![
            item(c, "ask", "Ask..."),
            item(c, "fill-in", "Fill-in..."),
            item(c, "if-then-else", "If...Then...Else..."),
            item(c, "merge-record", "Merge Record #"),
            item(c, "merge-sequence", "Merge Sequence #"),
            item(c, "next-record", "Next Record"),
            item(c, "next-record-if", "Next Record If..."),
            item(c, "set-bookmark", "Set Bookmark..."),
            item(c, "skip-record-if", "Skip Record If..."),
        ],
        FIELD => WriterPane::current_merge_columns()
            .iter()
            .map(|col| item(c, col, col))
            .collect(),
        FINISH => vec![
            item(
                c,
                "edit-individual-documents",
                "Edit Individual Documents...",
            ),
            disabled(c, "print-documents", "Print Documents..."),
            disabled(c, "send-email-messages", "Send Email Messages..."),
        ],
        _ => return None,
    })
}

/// The control `id` belongs to and the menu choice or typed value after it (`""` for none).
fn split(id: &str) -> Option<(&'static str, String)> {
    let typed = id.split_once('=').map(|(_, v)| v.to_owned());
    let plain = base(id);
    IDS.iter().find_map(|c| {
        if plain == *c {
            Some((*c, typed.clone().unwrap_or_default()))
        } else {
            plain
                .strip_prefix(*c)
                .and_then(|r| r.strip_prefix('.'))
                .map(|v| (*c, v.to_owned()))
        }
    })
}

fn merge_kind(choice: &str) -> Option<MergeKind> {
    Some(match choice {
        "" | "letters" => MergeKind::Letters,
        "e-mail-messages" => MergeKind::Email,
        "envelopes" => MergeKind::Envelopes,
        "labels" => MergeKind::Labels,
        "directory" => MergeKind::Directory,
        "normal-document" => MergeKind::Normal,
        _ => return None,
    })
}

fn need_merge(p: &mut WriterPane) -> bool {
    if p.mail_started() {
        return true;
    }
    p.status = Some("Start Mail Merge first".to_owned());
    false
}

fn need_list(p: &mut WriterPane) -> bool {
    if !need_merge(p) {
        return false;
    }
    if p.mail_has_recipients() {
        return true;
    }
    p.status = Some("Select Recipients first".to_owned());
    false
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let Some((control, choice)) = split(id) else {
        return Handled::No;
    };
    let choice = if choice == "run" {
        String::new()
    } else {
        choice
    };
    let Some(p) = ctx.writer.as_deref_mut() else {
        return Handled::Yes;
    };
    let done = match (control, choice.as_str()) {
        ("mailings.create.envelopes", _) => p.create_envelope(),
        ("mailings.create.labels", _) => p.create_labels(None, &LabelDef::named("5160")),
        (START, c) => match merge_kind(c) {
            Some(kind) => p.start_mail_merge(kind),
            None => return Handled::No,
        },
        (SELECT, "type-a-new-list") => {
            need_merge(p) && {
                p.new_recipient_list();
                true
            }
        }
        (SELECT, "" | "use-an-existing-list") => need_merge(p) && p.choose_recipients(),
        (SELECT, _) => return Handled::No,
        ("mailings.start-mail-merge.edit-recipient-list", _) => {
            if need_list(p) {
                p.mail(|m| m.dialog_requested = Some("edit-recipient-list"));
                let n = p.merge_record_count();
                p.status = Some(format!("{n} recipients selected"));
            }
            true
        }
        ("mailings.write-insert-fields.highlight-merge-fields", _) => {
            if need_merge(p) {
                p.toggle_highlight_merge_fields();
            }
            true
        }
        ("mailings.write-insert-fields.address-block", _) => {
            need_list(p) && p.insert_address_block()
        }
        ("mailings.write-insert-fields.greeting-line", _) => {
            need_list(p) && p.insert_greeting_line()
        }
        (FIELD, "") => {
            if need_list(p) {
                p.status = Some("Choose a field from the list".to_owned());
            }
            true
        }
        (FIELD, col) => need_list(p) && p.insert_merge_field(col),
        (RULES, "") => true,
        (RULES, rule) => need_list(p) && p.insert_rule(rule),
        ("mailings.write-insert-fields.match-fields", typed) => {
            need_list(p)
                && match typed.split_once(':') {
                    Some((key, col)) => {
                        p.match_field(key.trim(), Some(col.trim()).filter(|c| !c.is_empty()))
                    }
                    None => {
                        p.mail(|m| m.dialog_requested = Some("match-fields"));
                        true
                    }
                }
        }
        ("mailings.write-insert-fields.update-labels", _) => need_merge(p) && p.update_labels(),
        ("mailings.preview-results.preview-results", _) => need_list(p) && p.toggle_preview(),
        ("mailings.preview-results.first", _) => need_list(p) && p.goto_record(1),
        ("mailings.preview-results.previous", _) => {
            need_list(p) && {
                let at = p.merge_record();
                p.goto_record(at.saturating_sub(1).max(1))
            }
        }
        ("mailings.preview-results.next", _) => {
            need_list(p) && {
                let at = p.merge_record();
                p.goto_record(at + 1)
            }
        }
        ("mailings.preview-results.last", _) => {
            need_list(p) && {
                let n = p.merge_record_count();
                p.goto_record(n)
            }
        }
        ("mailings.preview-results.record", typed) => {
            need_list(p)
                && typed
                    .trim()
                    .parse::<usize>()
                    .is_ok_and(|n| p.goto_record(n))
        }
        ("mailings.preview-results.find-recipient", typed) => {
            need_list(p) && !typed.is_empty() && p.find_recipient(typed.trim(), None)
        }
        ("mailings.preview-results.check-for-errors", _) => {
            need_merge(p) && p.check_merge_errors().is_empty()
        }
        (FINISH, "" | "edit-individual-documents") => need_list(p) && p.finish_merge(None),
        (FINISH, _) => return Handled::No,
        _ => return Handled::No,
    };
    if done {
        *ctx.refocus_doc = true;
    }
    Handled::Yes
}
