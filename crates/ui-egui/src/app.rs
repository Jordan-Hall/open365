//! Application state: menu bar, status bar and the two document views.
//!
//! The application owns the open [`Target`]. Views read it and send edits as commands; nothing
//! here mutates a document directly.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use egui::{Context, Key, KeyboardShortcut, Modifiers, RichText, Ui, ViewportCommand};
use open365_commands::{Alignment, Command, Target};
use open365_pdf::PdfMetadata;

use crate::doc_view::{DocView, Toggle};
use crate::fonts::FontBook;
use crate::grid_view::GridView;

const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const PRINT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::P);
const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const REDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
const REDO_ALT: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
const BOLD: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::B);
const ITALIC: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::I);
const UNDERLINE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::U);
const ALIGN_LEFT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::L);
const ALIGN_CENTER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::E);
const ALIGN_RIGHT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::R);
const ALIGN_JUSTIFY: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::J);

/// Paragraph styles offered in the toolbar: (style id, label). The empty id is the default
/// style.
const STYLES: [(&str, &str); 5] = [
    ("Normal", "Normal"),
    ("Heading1", "Heading 1"),
    ("Heading2", "Heading 2"),
    ("Heading3", "Heading 3"),
    ("Title", "Title"),
];

/// Writes `bytes` to a new file in a private `open365-print` directory under the temp dir.
///
/// The name has a random part and the file is created with `create_new`, so a planted file or
/// symlink at a guessable path is never followed or overwritten. On Unix the directory is 0700
/// and the file 0600.
pub fn write_private_temp(stem: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::io::Write;

    let dir = std::env::temp_dir().join("open365-print");
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&dir).map_err(|e| e.to_string())?;
    if std::fs::symlink_metadata(&dir)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("print directory is a symlink".to_owned());
    }
    for _ in 0..16 {
        let mut h = RandomState::new().build_hasher();
        h.write_u32(std::process::id());
        let path = dir.join(format!("{stem}-{:016x}.pdf", h.finish()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        match opts.open(&path) {
            Ok(mut f) => {
                f.write_all(bytes).map_err(|e| e.to_string())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("could not create a unique print file".to_owned())
}

/// The command that hands a PDF to the operating system's print path.
///
/// * Windows: PowerShell `Start-Process -Verb Print`, the shell's `print` verb (the default PDF
///   handler prints it to the default printer); when the handler has no print verb the file is
///   opened with it instead and the user prints from there. The path travels in an environment
///   variable, never in the script text.
/// * Elsewhere: `lp` (CUPS), which prints to the default printer.
///
/// Printing is not native: there is no print dialog, page range or printer choice of our own,
/// and the PDF handler decides what the user sees.
pub fn print_command(pdf: &Path) -> std::process::Command {
    if cfg!(windows) {
        let mut cmd = std::process::Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            "$f = $env:OPEN365_PRINT_FILE; try { Start-Process -FilePath $f -Verb Print -ErrorAction Stop } catch { Start-Process -FilePath $f }",
        ])
        .env("OPEN365_PRINT_FILE", pdf);
        cmd
    } else {
        let mut cmd = std::process::Command::new("lp");
        cmd.arg("--").arg(pdf);
        cmd
    }
}

/// A change requested from the toolbar, applied after the widgets are drawn.
#[derive(Debug, Clone, Copy)]
enum FormatAction {
    Style(&'static str),
    Toggle(Toggle),
    Align(Alignment),
}

/// The Open365 desktop application.
#[derive(Debug)]
pub struct App {
    target: Option<Target>,
    pub doc: DocView,
    pub grid: GridView,
    status: String,
    title: String,
    fonts: Rc<RefCell<FontBook>>,
    /// Documents use installed system fonts before the bundled ones.
    system_fonts: bool,
}

impl App {
    pub fn new(initial: Option<PathBuf>) -> Self {
        let fonts = Rc::new(RefCell::new(FontBook::new()));
        let mut app = Self {
            target: None,
            doc: DocView::new(),
            grid: GridView::with_fonts(Rc::clone(&fonts)),
            status: "Open a .docx or .xlsx file (Ctrl+O)".to_owned(),
            title: String::new(),
            fonts,
            system_fonts: false,
        };
        if let Some(path) = initial {
            app.open_path(&path);
        }
        app
    }

    /// Lays documents out with the fonts installed on this machine, falling back to the bundled
    /// ones, so the screen matches Word's layout when its fonts are installed.
    #[must_use]
    pub fn with_system_fonts(mut self) -> Self {
        self.system_fonts = true;
        self.doc.use_system_fonts();
        self
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
                if self.system_fonts {
                    self.doc.use_system_fonts();
                }
                self.grid = GridView::with_fonts(Rc::clone(&self.fonts));
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

    /// The PDF of the open Word document, made from the layout on screen.
    fn render_pdf(&mut self) -> Result<Vec<u8>, String> {
        let target = self.target.as_ref().ok_or("no document is open")?;
        let title = target
            .path()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned());
        let meta = PdfMetadata {
            title,
            ..PdfMetadata::default()
        };
        let out = self.doc.export_pdf(target, &meta)?;
        if !out.warnings.is_empty() {
            self.status = format!("PDF warnings: {}", out.warnings.join("; "));
        }
        Ok(out.bytes)
    }

    /// File, Export PDF: asks for a path and writes the PDF there.
    pub fn export_pdf(&mut self) {
        let Some(target) = &self.target else { return };
        if target.docx().is_none() {
            self.status = "Export PDF needs a Word document".to_owned();
            return;
        }
        let name = target.path().and_then(|p| p.file_stem()).map_or_else(
            || "document".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        );
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF document", &["pdf"])
            .set_file_name(format!("{name}.pdf"))
            .set_title("Export PDF")
            .save_file()
        {
            self.export_pdf_to(&path);
        }
    }

    /// Writes the PDF of the open document to `path`.
    pub fn export_pdf_to(&mut self, path: &Path) {
        self.status = match self.render_pdf().and_then(|bytes| {
            std::fs::write(path, &bytes)
                .map(|()| bytes.len())
                .map_err(|e| e.to_string())
        }) {
            Ok(n) => format!("Exported {} ({n} bytes)", path.display()),
            Err(e) => format!("Export PDF failed: {e}"),
        };
    }

    /// File, Print: renders the PDF to a temporary file and hands it to the operating system's
    /// print path (see [`print_command`]). The file stays in the temp directory because the
    /// handler reads it after this returns.
    pub fn print(&mut self) {
        let Some(target) = &self.target else { return };
        if target.docx().is_none() {
            self.status = "Print needs a Word document".to_owned();
            return;
        }
        let stem = target.path().and_then(|p| p.file_stem()).map_or_else(
            || "document".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        );
        let stem: String = stem
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
            .collect();
        let mut path = PathBuf::new();
        self.status = match self
            .render_pdf()
            .and_then(|bytes| {
                path = write_private_temp(&stem, &bytes)?;
                Ok(())
            })
            .and_then(|()| {
                print_command(&path)
                    .spawn()
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }) {
            Ok(()) => format!("Sent {} to the print handler", path.display()),
            Err(e) => format!("Print failed: {e}"),
        };
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
        self.toggle(Toggle::Bold);
    }

    /// Flips bold, italic or underline in a Word document.
    pub fn toggle(&mut self, which: Toggle) {
        let Some(target) = &mut self.target else {
            return;
        };
        if target.docx().is_some() {
            self.doc.toggle(target, which);
            self.doc.focus();
        } else {
            self.status = "Character formatting applies to Word documents".to_owned();
        }
    }

    /// Sets the alignment of the selected paragraphs of a Word document.
    pub fn set_alignment(&mut self, alignment: Alignment) {
        if let Some(target) = &mut self.target
            && target.docx().is_some()
        {
            self.doc.set_alignment(target, alignment);
            self.doc.focus();
        }
    }

    /// Applies a paragraph style by id (`Normal`, `Heading1`...).
    pub fn set_style(&mut self, style_id: &str) {
        if let Some(target) = &mut self.target
            && target.docx().is_some()
        {
            self.doc.set_style(target, style_id);
            self.doc.focus();
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
        if take(&PRINT) {
            self.print();
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
        if take(&ITALIC) {
            self.toggle(Toggle::Italic);
        }
        if take(&UNDERLINE) {
            self.toggle(Toggle::Underline);
        }
        for (shortcut, alignment) in [
            (&ALIGN_LEFT, Alignment::Left),
            (&ALIGN_CENTER, Alignment::Center),
            (&ALIGN_RIGHT, Alignment::Right),
            (&ALIGN_JUSTIFY, Alignment::Justify),
        ] {
            if take(shortcut) {
                self.set_alignment(alignment);
            }
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
                ui.separator();
                if ui
                    .add_enabled(is_docx, egui::Button::new("Export PDF..."))
                    .clicked()
                {
                    ui.close();
                    self.export_pdf();
                }
                if ui
                    .add_enabled(
                        is_docx,
                        egui::Button::new("Print...")
                            .shortcut_text(ui.ctx().format_shortcut(&PRINT)),
                    )
                    .clicked()
                {
                    ui.close();
                    self.print();
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
                if ui
                    .add_enabled(
                        is_docx,
                        egui::Button::new("Italic")
                            .shortcut_text(ui.ctx().format_shortcut(&ITALIC)),
                    )
                    .clicked()
                {
                    ui.close();
                    self.toggle(Toggle::Italic);
                }
                if ui
                    .add_enabled(
                        is_docx,
                        egui::Button::new("Underline")
                            .shortcut_text(ui.ctx().format_shortcut(&UNDERLINE)),
                    )
                    .clicked()
                {
                    ui.close();
                    self.toggle(Toggle::Underline);
                }
            });
        });
    }

    /// Style, character and alignment controls for a Word document.
    fn format_bar(&mut self, ui: &mut Ui) {
        let style = self.doc.caret_format();
        let align = self.doc.caret_alignment();
        let current = self.doc.caret_style_id().unwrap_or("Normal").to_owned();
        let label = STYLES
            .iter()
            .find(|(id, _)| *id == current)
            .map_or(current.as_str(), |(_, label)| *label)
            .to_owned();
        let mut action = None;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("open365-style")
                .selected_text(label)
                .width(110.0)
                .show_ui(ui, |ui| {
                    for (id, name) in STYLES {
                        if ui.selectable_label(current == id, name).clicked() {
                            action = Some(FormatAction::Style(id));
                        }
                    }
                });
            ui.separator();
            let toggles = [
                (
                    Toggle::Bold,
                    RichText::new("Bold").strong(),
                    style.bold,
                    "Bold (Ctrl+B)",
                ),
                (
                    Toggle::Italic,
                    RichText::new("Italic").italics(),
                    style.italic,
                    "Italic (Ctrl+I)",
                ),
                (
                    Toggle::Underline,
                    RichText::new("Underline").underline(),
                    style.underline,
                    "Underline (Ctrl+U)",
                ),
            ];
            for (which, text, on, hint) in toggles {
                if ui
                    .add(egui::Button::new(text).selected(on))
                    .on_hover_text(hint)
                    .clicked()
                {
                    action = Some(FormatAction::Toggle(which));
                }
            }
            ui.separator();
            for (a, name, hint) in [
                (Alignment::Left, "Left", "Align left (Ctrl+L)"),
                (Alignment::Center, "Center", "Center (Ctrl+E)"),
                (Alignment::Right, "Right", "Align right (Ctrl+R)"),
                (Alignment::Justify, "Justify", "Justify (Ctrl+J)"),
            ] {
                if ui
                    .add(egui::Button::new(name).selected(align == a))
                    .on_hover_text(hint)
                    .clicked()
                {
                    action = Some(FormatAction::Align(a));
                }
            }
        });
        match action {
            Some(FormatAction::Style(id)) => self.set_style(id),
            Some(FormatAction::Toggle(which)) => self.toggle(which),
            Some(FormatAction::Align(a)) => self.set_alignment(a),
            None => {}
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let right = match &self.target {
            Some(t) if t.docx().is_some() => {
                let caret = self.doc.caret();
                let subs = self.doc.font_substitutions();
                let fonts = match subs.as_slice() {
                    [] => String::new(),
                    [(from, to), rest @ ..] => format!(
                        "  |  Substituted font: {from} -> {to}{}",
                        if rest.is_empty() {
                            String::new()
                        } else {
                            format!(" (+{} more)", rest.len())
                        }
                    ),
                };
                format!(
                    "Page {} of {}  |  Paragraph {}, character {}{fonts}  |  {:.1} ms",
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
        self.fonts.borrow_mut().sync(&ctx);
        self.shortcuts(&ctx);

        egui::Panel::top("open365-menu").show(ui, |ui| self.menu_bar(ui));
        if self.target.as_ref().is_some_and(|t| t.docx().is_some()) {
            egui::Panel::top("open365-format").show(ui, |ui| self.format_bar(ui));
        }
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
