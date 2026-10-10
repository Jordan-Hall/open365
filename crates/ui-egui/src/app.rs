//! Application state: menu bar, status bar and the two document views.
//!
//! The application owns the open [`Target`]. Views read it and send edits as commands; nothing
//! here mutates a document directly.

use std::path::{Path, PathBuf};

use egui::{Context, Key, KeyboardShortcut, Modifiers, Ui, ViewportCommand};
use open365_commands::{Command, Target};

use crate::doc_view::DocView;
use crate::grid_view::GridView;

const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const REDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
const REDO_ALT: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
const BOLD: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::B);

/// The Open365 desktop application.
#[derive(Debug)]
pub struct App {
    target: Option<Target>,
    pub doc: DocView,
    pub grid: GridView,
    status: String,
    title: String,
}

impl App {
    pub fn new(initial: Option<PathBuf>) -> Self {
        let mut app = Self {
            target: None,
            doc: DocView::new(),
            grid: GridView::new(),
            status: "Open a .docx or .xlsx file (Ctrl+O)".to_owned(),
            title: String::new(),
        };
        if let Some(path) = initial {
            app.open_path(&path);
        }
        app
    }

    pub fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    /// Opens a file by path, replacing the current document.
    pub fn open_path(&mut self, path: &Path) {
        match Target::open_path(path) {
            Ok(target) => {
                self.status = format!("Opened {}", path.display());
                self.target = Some(target);
                self.doc = DocView::new();
                self.grid = GridView::new();
            }
            Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    pub fn save(&mut self) {
        let path = self
            .target
            .as_ref()
            .and_then(|t| t.path().map(Path::to_path_buf));
        match path {
            Some(path) => self.save_to(path),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) {
        let Some(target) = &self.target else { return };
        let (name, ext) = if target.docx().is_some() {
            ("Word document", "docx")
        } else {
            ("Excel workbook", "xlsx")
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(name, &[ext])
            .set_title("Save As")
            .save_file()
        {
            self.save_to(path);
        }
    }

    /// Saves through the `Save` command.
    pub fn save_to(&mut self, path: PathBuf) {
        let Some(target) = &mut self.target else {
            return;
        };
        match target.apply(Command::Save { path: path.clone() }) {
            Ok(out) => {
                self.status = format!(
                    "Saved {} ({} bytes, {:?})",
                    path.display(),
                    out.bytes_written,
                    out.effect
                );
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    pub fn undo(&mut self) {
        let Some(target) = &mut self.target else {
            return;
        };
        match target.undo() {
            Ok(_) => {
                self.doc.reset();
                self.status = "Undid last change".to_owned();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    pub fn redo(&mut self) {
        let Some(target) = &mut self.target else {
            return;
        };
        match target.redo() {
            Ok(_) => {
                self.doc.reset();
                self.status = "Redid change".to_owned();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    pub fn toggle_bold(&mut self) {
        let Some(target) = &mut self.target else {
            return;
        };
        if target.docx().is_some() {
            self.doc.toggle_bold(target);
        } else {
            self.status = "Bold applies to Word documents".to_owned();
        }
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Open365 documents", &["docx", "xlsx"])
            .set_title("Open")
            .pick_file()
        {
            self.open_path(&path);
        }
    }

    fn shortcuts(&mut self, ctx: &Context) {
        let take = |s: &KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(s));
        if take(&OPEN) {
            self.open_dialog();
        }
        if take(&SAVE_AS) {
            self.save_as();
        }
        if take(&SAVE) {
            self.save();
        }
        // While the formula bar is being edited, undo belongs to the text box.
        if !self.grid.is_editing() {
            if take(&REDO_ALT) || take(&REDO) {
                self.redo();
            }
            if take(&UNDO) {
                self.undo();
            }
        }
        if take(&BOLD) {
            self.toggle_bold();
        }
    }

    fn menu_bar(&mut self, ui: &mut Ui) {
        let (can_undo, can_redo, has_doc, is_docx) = self
            .target
            .as_ref()
            .map_or((false, false, false, false), |t| {
                (t.can_undo(), t.can_redo(), true, t.docx().is_some())
            });
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui
                    .add(
                        egui::Button::new("Open...").shortcut_text(ui.ctx().format_shortcut(&OPEN)),
                    )
                    .clicked()
                {
                    ui.close();
                    self.open_dialog();
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new("Save").shortcut_text(ui.ctx().format_shortcut(&SAVE)),
                    )
                    .clicked()
                {
                    ui.close();
                    self.save();
                }
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new("Save As...")
                            .shortcut_text(ui.ctx().format_shortcut(&SAVE_AS)),
                    )
                    .clicked()
                {
                    ui.close();
                    self.save_as();
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui
                    .add_enabled(
                        can_undo,
                        egui::Button::new("Undo").shortcut_text(ui.ctx().format_shortcut(&UNDO)),
                    )
                    .clicked()
                {
                    ui.close();
                    self.undo();
                }
                if ui
                    .add_enabled(
                        can_redo,
                        egui::Button::new("Redo").shortcut_text(ui.ctx().format_shortcut(&REDO)),
                    )
                    .clicked()
                {
                    ui.close();
                    self.redo();
                }
            });
            ui.menu_button("Format", |ui| {
                if ui
                    .add_enabled(
                        is_docx,
                        egui::Button::new("Bold").shortcut_text(ui.ctx().format_shortcut(&BOLD)),
                    )
                    .clicked()
                {
                    ui.close();
                    self.toggle_bold();
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let right = match &self.target {
            Some(t) if t.docx().is_some() => {
                let caret = self.doc.caret();
                format!(
                    "Page {} of {}  |  Paragraph {}, character {}  |  {:.1} ms",
                    self.doc.page_of(caret.para) + 1,
                    self.doc.page_count(),
                    caret.para + 1,
                    caret.offset + 1,
                    self.doc.last_frame_ms
                )
            }
            Some(t) if t.xlsx().is_some() => {
                let (row, col) = self.grid.active();
                format!(
                    "{}{}{}  |  {:.1} ms",
                    t.xlsx()
                        .and_then(|w| w.sheets().get(self.grid.sheet()))
                        .map_or(String::new(), |s| format!("{}!", s.name)),
                    open365_xlsx::col_letters(col),
                    row,
                    self.grid.last_frame_ms
                )
            }
            _ => String::new(),
        };
        let dirty = self.target.as_ref().is_some_and(Target::is_dirty);
        ui.horizontal(|ui| {
            ui.label(if dirty { "Modified" } else { "Saved" });
            ui.separator();
            ui.label(&self.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(right);
            });
        });
    }

    fn update_title(&mut self, ctx: &Context) {
        let name = self
            .target
            .as_ref()
            .and_then(|t| t.path())
            .and_then(|p| p.file_name())
            .map_or_else(
                || "Open365".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            );
        let dirty = self.target.as_ref().is_some_and(Target::is_dirty);
        let title = format!("{}{} - Open365", name, if dirty { " *" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// Draws the whole window into `ui`.
    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);

        egui::Panel::top("open365-menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("open365-status").show(ui, |ui| self.status_bar(ui));

        let mut repaint = false;
        egui::CentralPanel::default_margins().show(ui, |ui| match &mut self.target {
            Some(target) if target.docx().is_some() => {
                repaint = self.doc.show(ui, target);
            }
            Some(target) => self.grid.show(ui, target),
            None => {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.heading("Open365");
                    ui.label("Open a Word (.docx) or Excel (.xlsx) file to begin.");
                    if ui.button("Open...").clicked() {
                        self.open_dialog();
                    }
                });
            }
        });
        if let Some(s) = self.doc.status.take().or_else(|| self.grid.status.take()) {
            self.status = s;
        }
        if repaint {
            ctx.request_repaint();
        }
        self.update_title(&ctx);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        App::ui(self, ui);
    }
}
