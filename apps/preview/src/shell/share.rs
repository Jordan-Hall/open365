//! The Share dialog (ADR-013 Stage 0): local sharing only. Every action is a command of the
//! command layer (`share_send_copy`, `share_send_pdf`, `share_inspect`, `share_sanitize_copy`,
//! `share_os_sheet`); the dialog only shows what they report. Link and people sections are
//! drawn disabled with "Connect a server".

use std::collections::BTreeMap;
use std::path::PathBuf;

use open365_commands::{Command, HiddenKind, Outcome};
use open365_gfx::Color;
use open365_ui_core::{Content, Look, NodeId, Token, Ui, Widget};

use super::{Act, PxRect, Shell};
use crate::AppKind;

/// What the dialog remembers between rebuilds.
#[derive(Debug, Default)]
pub struct ShareDialog {
    /// Items found by the inspector, by kind: (count, removable).
    pub found: Option<BTreeMap<&'static str, (usize, bool)>>,
    pub authors: Vec<String>,
    /// Kinds the next "remove and send" takes out.
    pub remove: Vec<HiddenKind>,
    pub status: String,
    /// The last copy written, for "Show in folder".
    pub last: Option<PathBuf>,
}

impl ShareDialog {
    fn kind_label(k: HiddenKind) -> &'static str {
        match k {
            HiddenKind::Comments => "Comments",
            HiddenKind::TrackedChanges => "Tracked changes",
            HiddenKind::HiddenText => "Hidden text, sheets, rows and columns",
            HiddenKind::Properties => "Document properties",
            HiddenKind::PersonalInfo => "Author names",
        }
    }
}

const PREFIX: &str = "share.dialog.";

impl Shell {
    pub fn share_open(&self) -> bool {
        self.share.is_some()
    }

    pub fn open_share(&mut self, ui: &mut Ui) {
        self.group_popup = None;
        self.backstage = None;
        self.share = Some(ShareDialog::default());
        self.build_body(ui);
    }

    pub(super) fn close_share(&mut self) -> bool {
        self.share.take().is_some()
    }

    /// Runs one command on the open document and returns its outcome.
    fn share_apply(&mut self, c: Command) -> Result<Outcome, String> {
        match (self.kind, self.writer.as_mut()) {
            (AppKind::Writer, Some(p)) => p.apply_share(c),
            (AppKind::Writer, None) => Err("no document is open".to_owned()),
            (AppKind::Sheets, _) => self.sheet.apply_share(c),
        }
    }

    /// A click on a dialog control (`share.dialog.<what>`).
    pub(super) fn share_action(&mut self, ui: &mut Ui, node: NodeId, id: &str) -> bool {
        let Some(what) = id.strip_prefix(PREFIX) else {
            return false;
        };
        let checked = match ui.tree.get(node).map(|n| &n.widget) {
            Some(Widget::Checkbox { checked }) => Some(*checked),
            _ => None,
        };
        let mut dlg = self.share.take().unwrap_or_default();
        match what {
            "close" => return true,
            "send-copy" => match self.share_apply(Command::ShareSendCopy {
                path: None,
                overwrite: false,
            }) {
                Ok(o) => {
                    dlg.status = match &o.saved_to {
                        Some(p) => format!("Copy written to {}", p.display()),
                        None => "Copy written".to_owned(),
                    };
                    dlg.last = o.saved_to;
                }
                Err(e) => dlg.status = e,
            },
            "send-pdf" => match self.share_apply(Command::ShareSendPdf {
                path: None,
                overwrite: false,
            }) {
                Ok(o) => {
                    dlg.status = match &o.saved_to {
                        Some(p) => format!("PDF written to {} ({} pages)", p.display(), o.pages),
                        None => "PDF written".to_owned(),
                    };
                    dlg.last = o.saved_to;
                }
                Err(e) => dlg.status = e,
            },
            "inspect" => match self.share_apply(Command::ShareInspect) {
                Ok(o) => {
                    let mut found: BTreeMap<&'static str, (usize, bool)> = BTreeMap::new();
                    let mut authors = Vec::new();
                    if let Some(r) = &o.report {
                        for item in r["items"].as_array().into_iter().flatten() {
                            let name = item["kind"].as_str().unwrap_or_default();
                            if let Some(k) = HiddenKind::ALL.iter().find(|k| k.name() == name) {
                                let e = found.entry(k.name()).or_insert((0, true));
                                e.0 += item["count"].as_u64().unwrap_or(1) as usize;
                                e.1 &= item["removable"].as_bool().unwrap_or(false);
                            }
                        }
                        authors = r["authors"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|a| a.as_str().map(str::to_owned))
                            .collect();
                    }
                    dlg.remove = HiddenKind::ALL
                        .iter()
                        .copied()
                        .filter(|k| found.get(k.name()).is_some_and(|(_, r)| *r))
                        .collect();
                    dlg.status = if found.is_empty() {
                        "No comments, tracked changes, hidden content or personal information found."
                            .to_owned()
                    } else {
                        "Choose what to remove from the copy. The document itself is not changed."
                            .to_owned()
                    };
                    dlg.found = Some(found);
                    dlg.authors = authors;
                }
                Err(e) => dlg.status = e,
            },
            "sanitize" => {
                let remove = dlg.remove.clone();
                match self.share_apply(Command::ShareSanitizeCopy {
                    path: None,
                    overwrite: false,
                    remove,
                }) {
                    Ok(o) => {
                        dlg.status = format!(
                            "Clean copy written to {}. {}",
                            o.saved_to
                                .as_ref()
                                .map_or_else(String::new, |p| p.display().to_string()),
                            o.notes.join("; ")
                        );
                        dlg.last = o.saved_to;
                    }
                    Err(e) => dlg.status = e,
                }
            }
            "reveal" => {
                if let Some(p) = dlg.last.clone() {
                    dlg.status = match self.share_apply(Command::ShareOsSheet { path: p.clone() }) {
                        Ok(_) => reveal(&p),
                        Err(e) => e,
                    };
                }
            }
            other => {
                if let Some(name) = other.strip_prefix("kind.")
                    && let Some(k) = HiddenKind::ALL.iter().copied().find(|k| k.name() == name)
                {
                    dlg.remove.retain(|r| *r != k);
                    if checked.unwrap_or(false) {
                        dlg.remove.push(k);
                    }
                }
            }
        }
        self.share = Some(dlg);
        self.build_body(ui);
        false
    }

    pub(super) fn build_share(&mut self, ui: &mut Ui) {
        let Some(dlg) = self.share.take() else { return };
        let root = ui.tree.root();
        let (ww, wh) = self.win_px;
        let o = (0, 0);
        let back = self.panel(
            ui,
            root,
            o,
            PxRect {
                x: 0,
                y: 0,
                w: ww,
                h: wh,
            },
            Look {
                fill_rgb: Some(Color::rgba(0, 0, 0, 120)),
                still: true,
                ..Look::default()
            },
        );
        self.body_ids.push(back);
        let w = self.px(640).min(ww - self.px(24));
        let h = self.px(600).min(wh - self.px(24));
        let (x0, y0) = ((ww - w) / 2, ((wh - h) / 2).max(0));
        self.share_o = (x0, y0);
        let card = self.panel(
            ui,
            back,
            o,
            PxRect { x: x0, y: y0, w, h },
            Look {
                fill: Some(Token::MenuBg),
                border: Some(Token::MenuBorder),
                still: true,
                ..Look::default()
            },
        );
        let pad = self.px(24);
        let inner = w - 2 * pad;
        let mut y = y0 + self.px(18);
        let x = x0 + pad;

        self.share_label(ui, card, x, y, inner, 44, "Share", 22.0, Token::Text);
        let cb = self.px(32);
        let close = self.add(
            ui,
            card,
            self.share_o,
            PxRect {
                x: x0 + w - cb - self.px(10),
                y: y0 + self.px(10),
                w: cb,
                h: cb,
            },
            Widget::Button { toggled: None },
            "Close",
            Look {
                icon: self.icon("x"),
                content: Content::IconOnly,
                ..Look::default()
            },
            Act::Cmd(format!("{PREFIX}close")),
        );
        let _ = close;
        y += self.px(52);

        self.share_label(ui, card, x, y, inner, 28, "Send a copy", 15.0, Token::Text);
        y += self.px(34);
        let bh = self.px(36);
        let mut bx = x;
        let mut buttons: Vec<(&str, &str, bool)> = vec![("send-copy", "Send a copy", true)];
        if self.kind == AppKind::Writer {
            buttons.push(("send-pdf", "Send as PDF", true));
        }
        buttons.push(("reveal", "Show in folder", dlg.last.is_some()));
        for (id, label, enabled) in buttons {
            let bw = self.px(176);
            self.share_button(
                ui,
                card,
                PxRect {
                    x: bx,
                    y,
                    w: bw,
                    h: bh,
                },
                id,
                label,
                enabled,
            );
            bx += bw + self.px(8);
        }
        y += bh + self.px(22);

        self.share_label(
            ui,
            card,
            x,
            y,
            inner,
            28,
            "Check before sending",
            15.0,
            Token::Text,
        );
        y += self.px(34);
        self.share_button(
            ui,
            card,
            PxRect {
                x,
                y,
                w: self.px(300),
                h: bh,
            },
            "inspect",
            "Inspect for hidden content",
            true,
        );
        y += bh + self.px(8);
        if let Some(found) = &dlg.found {
            for k in HiddenKind::ALL {
                let Some((n, removable)) = found.get(k.name()).copied() else {
                    continue;
                };
                let label = format!(
                    "{} ({n}){}",
                    ShareDialog::kind_label(k),
                    if removable {
                        ""
                    } else {
                        " - cannot be removed"
                    }
                );
                let id = self.add(
                    ui,
                    card,
                    self.share_o,
                    PxRect {
                        x,
                        y,
                        w: inner,
                        h: self.px(30),
                    },
                    Widget::Checkbox {
                        checked: dlg.remove.contains(&k),
                    },
                    &label,
                    Look::default(),
                    Act::Cmd(format!("{PREFIX}kind.{}", k.name())),
                );
                if !removable && let Some(n) = ui.tree.get_mut(id) {
                    n.enabled = false;
                }
                y += self.px(30);
            }
            if !dlg.authors.is_empty() {
                let text = format!("Authors: {}", dlg.authors.join(", "));
                self.share_label(ui, card, x, y, inner, 26, &text, 13.0, Token::TextDim);
                y += self.px(28);
            }
            if !found.is_empty() {
                self.share_button(
                    ui,
                    card,
                    PxRect {
                        x,
                        y,
                        w: self.px(300),
                        h: bh,
                    },
                    "sanitize",
                    "Remove selected and send a copy",
                    !dlg.remove.is_empty(),
                );
                y += bh + self.px(8);
            }
        }
        y += self.px(14);

        self.share_label(
            ui,
            card,
            x,
            y,
            inner,
            28,
            "Links and people",
            15.0,
            Token::TextDisabled,
        );
        y += self.px(34);
        let mut bx = x;
        for (id, label) in [("people", "Share with people"), ("link", "Copy link")] {
            let bw = self.px(176);
            self.share_button(
                ui,
                card,
                PxRect {
                    x: bx,
                    y,
                    w: bw,
                    h: bh,
                },
                id,
                label,
                false,
            );
            bx += bw + self.px(8);
        }
        y += bh + self.px(6);
        self.share_label(
            ui,
            card,
            x,
            y,
            inner,
            26,
            "Connect a server",
            13.0,
            Token::TextDim,
        );

        let sy = y0 + h - self.px(64);
        self.share_label(
            ui,
            card,
            x,
            sy,
            inner,
            48,
            &dlg.status,
            13.0,
            Token::TextDim,
        );
        self.share = Some(dlg);
    }

    #[allow(clippy::too_many_arguments)]
    fn share_label(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        text: &str,
        size: f32,
        ink: Token,
    ) {
        let h = self.px(h);
        self.add(
            ui,
            parent,
            self.share_o,
            PxRect { x, y, w, h },
            Widget::Label,
            text,
            Look {
                content: Content::TextOnly,
                align: 0.0,
                pad: 0.0,
                text_size: Some(size),
                ink: Some(ink),
                ..Look::default()
            },
            Act::Inert,
        );
    }

    fn share_button(
        &mut self,
        ui: &mut Ui,
        parent: NodeId,
        r: PxRect,
        id: &str,
        label: &str,
        enabled: bool,
    ) {
        let n = self.add(
            ui,
            parent,
            self.share_o,
            r,
            Widget::Button { toggled: None },
            label,
            Look {
                content: Content::TextOnly,
                fill: Some(Token::Field),
                border: Some(Token::FieldBorder),
                radius: 2.0,
                ..Look::default()
            },
            Act::Cmd(format!("{PREFIX}{id}")),
        );
        if !enabled && let Some(node) = ui.tree.get_mut(n) {
            node.enabled = false;
        }
    }
}

/// Opens the folder of a written copy with the file selected; the fallback of the OS share sheet.
fn reveal(path: &std::path::Path) -> String {
    #[cfg(windows)]
    {
        match std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn()
        {
            Ok(_) => format!("Shown in the folder: {}", path.display()),
            Err(e) => format!("Cannot open the folder: {e}"),
        }
    }
    #[cfg(not(windows))]
    {
        format!("Copy is at {}", path.display())
    }
}
