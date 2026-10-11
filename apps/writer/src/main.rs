//! Open365 Writer: `open365-writer [file.docx|.dotx|.docm|.dotm]`.

mod blank;

use std::path::{Path, PathBuf};

use egui::{Key, KeyboardShortcut, Modifiers, Ui};
use open365_commands::{Alignment, Command, RunPropsPatch, Target};
use open365_pdf::PdfMetadata;
use open365_ui_egui::{
    ClipboardState, DocView, FindDialog, GridView, Toggle, print_command, write_private_temp,
};
use open365_ui_shell::{AppKind, CommandState, MenuEntry, Mode, Shell, ShellHost, Theme, fonts};

const EXTENSIONS: [&str; 4] = ["docx", "dotx", "docm", "dotm"];

fn shortcut(m: Modifiers, k: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(m, k)
}

/// Everything except the chrome: the document, its views and the status line.
struct State {
    target: Option<Target>,
    doc: DocView,
    /// Only the Find dialog needs a grid; Writer never shows one.
    grid: GridView,
    find: FindDialog,
    clipboard: ClipboardState,
    status: String,
    recent: Vec<PathBuf>,
    untitled: u32,
    words: (u64, usize),
    /// Colour the font colour button applies (RRGGBB), the last one chosen.
    font_color: String,
}

impl State {
    fn new() -> Self {
        Self {
            target: None,
            doc: DocView::new(),
            grid: GridView::new(),
            find: FindDialog::new(),
            clipboard: ClipboardState::default(),
            status: String::new(),
            recent: Vec::new(),
            untitled: 0,
            words: (u64::MAX, 0),
            font_color: "FF0000".to_owned(),
        }
    }

    fn set_target(&mut self, target: Target) {
        self.target = Some(target);
        self.doc = DocView::new();
        self.doc.use_system_fonts();
        // Word opens at 100 %.
        self.doc.set_zoom(Some(1.0));
    }

    fn new_blank(&mut self) {
        match Target::open_docx(&blank::docx()) {
            Ok(t) => {
                self.untitled += 1;
                self.set_target(t);
                self.status = "New blank document".into();
            }
            Err(e) => self.status = format!("Could not create a document: {e}"),
        }
    }

    fn open_path(&mut self, path: &Path) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let result = if ext == "dotx" || ext == "dotm" {
            std::fs::read(path)
                .map_err(|e| e.to_string())
                .and_then(|b| Target::open_template(&b, &ext).map_err(|e| e.to_string()))
        } else {
            Target::open_path(path).map_err(|e| e.to_string())
        };
        match result {
            Ok(t) => {
                self.set_target(t);
                self.recent.retain(|p| p != path);
                self.recent.insert(0, path.to_path_buf());
                self.status = format!("Opened {}", path.display());
            }
            Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Documents", &EXTENSIONS)
            .set_title("Open")
            .pick_file()
        {
            self.open_path(&p);
        }
    }

    fn save(&mut self) {
        let path = self
            .target
            .as_ref()
            .and_then(|t| t.path().map(Path::to_path_buf));
        match path {
            Some(p) => self.save_to(p),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        if self.target.is_none() {
            return;
        }
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Document", &["docx"])
            .set_title("Save As")
            .save_file()
        {
            self.save_to(p);
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        let Some(t) = &mut self.target else { return };
        self.status = match t.apply(Command::Save { path: path.clone() }) {
            Ok(out) => format!("Saved {} ({} bytes)", path.display(), out.bytes_written),
            Err(e) => format!("Save failed: {e}"),
        };
    }

    fn stem(&self) -> String {
        self.target
            .as_ref()
            .and_then(|t| t.path())
            .and_then(|p| p.file_stem())
            .map_or_else(
                || "document".to_owned(),
                |s| s.to_string_lossy().into_owned(),
            )
    }

    fn export_pdf(&mut self) {
        if self.target.is_none() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF document", &["pdf"])
            .set_file_name(format!("{}.pdf", self.stem()))
            .set_title("Export PDF")
            .save_file()
        {
            self.export_pdf_to(&path);
        }
    }

    fn export_pdf_to(&mut self, path: &Path) {
        let title = Some(self.stem());
        let Some(t) = self.target.as_mut() else {
            return;
        };
        t.set_pdf_options(open365_commands::PdfOptions {
            metadata: PdfMetadata {
                title,
                ..PdfMetadata::default()
            },
            system_fonts: true,
            ..open365_commands::PdfOptions::default()
        });
        self.status = match t.apply(Command::ExportPdf {
            path: path.to_path_buf(),
            overwrite: true,
        }) {
            Ok(out) => format!("Exported {} ({} bytes)", path.display(), out.bytes_written),
            Err(e) => format!("Export PDF failed: {e}"),
        };
    }

    /// File, Print: renders the PDF to a private temporary file and hands it to the OS.
    fn print(&mut self) {
        let meta = PdfMetadata {
            title: Some(self.stem()),
            ..PdfMetadata::default()
        };
        let stem = self.stem();
        let Some(t) = &self.target else { return };
        let result = self
            .doc
            .export_pdf(t, &meta)
            .and_then(|out| write_private_temp(&stem, &out.bytes))
            .and_then(|p| {
                print_command(&p)
                    .spawn()
                    .map(|_| p)
                    .map_err(|e| e.to_string())
            });
        self.status = match result {
            Ok(p) => format!("Sent {} to the print handler", p.display()),
            Err(e) => format!("Print failed: {e}"),
        };
    }

    fn undo(&mut self) {
        if let Some(t) = &mut self.target
            && let Err(e) = self.doc.undo(t)
        {
            self.status = e.to_string();
        }
    }

    fn redo(&mut self) {
        if let Some(t) = &mut self.target
            && let Err(e) = self.doc.redo(t)
        {
            self.status = e.to_string();
        }
    }

    fn toggle(&mut self, which: Toggle) {
        if let Some(t) = &mut self.target {
            self.doc.toggle(t, which);
            self.doc.focus();
        }
    }

    fn run_props(&mut self, patch: RunPropsPatch) {
        if let Some(t) = &mut self.target {
            self.doc.set_run_props(t, patch);
            self.doc.focus();
        }
    }

    /// Font size one step up or down the size list.
    fn step_size(&mut self, up: bool) {
        let cur = self.doc.caret_format().size_half_points / 2;
        let next = if up {
            SIZES.iter().find(|s| **s > cur).copied()
        } else {
            SIZES.iter().rev().find(|s| **s < cur).copied()
        };
        if let Some(n) = next {
            self.run_props(RunPropsPatch {
                size_half_points: Some(n * 2),
                ..RunPropsPatch::default()
            });
        }
    }

    fn align(&mut self, a: Alignment) {
        if let Some(t) = &mut self.target {
            self.doc.set_alignment(t, a);
            self.doc.focus();
        }
    }

    fn copy(&mut self) {
        if let Some(t) = &self.target {
            self.clipboard.copy_doc(t, &self.doc);
        }
    }

    fn cut(&mut self) {
        if let Some(t) = &mut self.target
            && self.clipboard.copy_doc(t, &self.doc)
        {
            self.doc.delete_selection(t);
        }
    }

    fn paste(&mut self) {
        let text = arboard::Clipboard::new()
            .ok()
            .and_then(|mut c| c.get_text().ok());
        if let (Some(t), Some(text)) = (&mut self.target, text) {
            self.doc.paste_text(t, &text.replace("\r\n", "\n"));
        }
    }

    fn word_count(&mut self) -> usize {
        let Some(t) = &self.target else { return 0 };
        let rev = t.revision();
        if self.words.0 != rev {
            let n = t.docx().map_or(0, |d| {
                (0..self.doc.paragraph_count())
                    .filter_map(|i| d.paragraph_text(i))
                    .map(|p| p.split_whitespace().count())
                    .sum()
            });
            self.words = (rev, n);
        }
        self.words.1
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let take = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        let c = Modifiers::COMMAND;
        if take(shortcut(c, Key::N)) {
            self.new_blank();
        }
        if take(shortcut(c, Key::O)) {
            self.open_dialog();
        }
        if take(shortcut(c.plus(Modifiers::SHIFT), Key::S)) {
            self.save_as();
        }
        if take(shortcut(c, Key::S)) {
            self.save();
        }
        if take(shortcut(c, Key::P)) {
            self.print();
        }
        if self.target.is_some() {
            if take(shortcut(c, Key::F)) {
                self.find.show_dialog(false);
            }
            if take(shortcut(c, Key::H)) {
                self.find.show_dialog(true);
            }
        }
        if take(shortcut(c.plus(Modifiers::SHIFT), Key::Z)) || take(shortcut(c, Key::Y)) {
            self.redo();
        }
        if take(shortcut(c, Key::Z)) {
            self.undo();
        }
        if take(shortcut(c, Key::B)) {
            self.toggle(Toggle::Bold);
        }
        if take(shortcut(c, Key::I)) {
            self.toggle(Toggle::Italic);
        }
        if take(shortcut(c, Key::U)) {
            self.toggle(Toggle::Underline);
        }
        for (k, a) in [
            (Key::L, Alignment::Left),
            (Key::E, Alignment::Center),
            (Key::R, Alignment::Right),
            (Key::J, Alignment::Justify),
        ] {
            if take(shortcut(c, k)) {
                self.align(a);
            }
        }
        // Copy, cut and paste arrive as events; the document view must not also act on them.
        if self.find.text_focused || ctx.current_pass_index() != 0 {
            return;
        }
        if self.target.is_none() || !ctx.memory(|m| m.has_focus(self.doc.id())) {
            return;
        }
        let events: Vec<egui::Event> = ctx.input_mut(|i| {
            let mut taken = Vec::new();
            i.events.retain(|e| {
                let ours = matches!(
                    e,
                    egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)
                );
                if ours {
                    taken.push(e.clone());
                }
                !ours
            });
            taken
        });
        for e in events {
            match e {
                egui::Event::Copy => self.copy(),
                egui::Event::Cut => self.cut(),
                egui::Event::Paste(text) => {
                    if let Some(t) = &mut self.target {
                        self.doc.paste_text(t, &text);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Fonts offered by the font box (the document keeps whatever name it has; these are suggestions).
const FONTS: [&str; 10] = [
    "Calibri",
    "Cambria",
    "Arial",
    "Times New Roman",
    "Courier New",
    "Georgia",
    "Verdana",
    "Segoe UI",
    "Consolas",
    "Tahoma",
];
/// The font size list of the size box, in points.
const SIZES: [u32; 16] = [
    8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 36, 48, 72, 96, 120,
];
/// Standard font colours (RRGGBB): the menu of the font colour button.
const COLORS: [(&str, &str); 10] = [
    ("C00000", "Dark Red"),
    ("FF0000", "Red"),
    ("FFC000", "Orange"),
    ("FFFF00", "Yellow"),
    ("92D050", "Light Green"),
    ("00B050", "Green"),
    ("00B0F0", "Light Blue"),
    ("0070C0", "Blue"),
    ("002060", "Dark Blue"),
    ("7030A0", "Purple"),
];

const STYLES: [(&str, &str); 5] = [
    ("Normal", "Normal"),
    ("Heading1", "Heading 1"),
    ("Heading2", "Heading 2"),
    ("Heading3", "Heading 3"),
    ("Title", "Title"),
];

struct Host<'a> {
    s: &'a mut State,
    theme: Theme,
}

impl ShellHost for Host<'_> {
    fn state(&self, id: &str) -> CommandState {
        let s = &*self.s;
        let t = s.target.as_ref();
        let has = t.is_some();
        let sel = s.doc.has_selection();
        match id {
            "qat.filesave" | "file.save" | "file.save-as" | "file.print" | "file.export"
            | "file.close" => {
                if has {
                    CommandState::ENABLED
                } else {
                    CommandState::DISABLED
                }
            }
            "file.new" | "file.open" => CommandState::ENABLED,
            "qat.undo" => CommandState {
                enabled: t.is_some_and(Target::can_undo),
                checked: false,
            },
            "qat.redoorrepeat" | "qat.redo" => CommandState {
                enabled: t.is_some_and(Target::can_redo),
                checked: false,
            },
            "home.clipboard.paste" => {
                if has {
                    CommandState::ENABLED
                } else {
                    CommandState::DISABLED
                }
            }
            "home.clipboard.cut" | "home.clipboard.copy" => CommandState {
                enabled: sel,
                checked: false,
            },
            "home.font.bold" => CommandState {
                enabled: has,
                checked: s.doc.caret_format().bold,
            },
            "home.font.italic" => CommandState {
                enabled: has,
                checked: s.doc.caret_format().italic,
            },
            "home.font.underline" => CommandState {
                enabled: has,
                checked: s.doc.caret_format().underline,
            },
            "home.font.strikethrough" => CommandState {
                enabled: has,
                checked: s.doc.caret_format().strike,
            },
            "home.font.font"
            | "home.font.font-size"
            | "home.font.grow-font"
            | "home.font.shrink-font"
            | "home.font.font-colour" => {
                if has {
                    CommandState::ENABLED
                } else {
                    CommandState::DISABLED
                }
            }
            "home.paragraph.align-left" => CommandState {
                enabled: has,
                checked: s.doc.caret_alignment() == Alignment::Left,
            },
            "home.paragraph.centre" => CommandState {
                enabled: has,
                checked: s.doc.caret_alignment() == Alignment::Center,
            },
            "home.paragraph.align-right" => CommandState {
                enabled: has,
                checked: s.doc.caret_alignment() == Alignment::Right,
            },
            "home.paragraph.justify" => CommandState {
                enabled: has,
                checked: s.doc.caret_alignment() == Alignment::Justify,
            },
            "home.styles.quickstylesgallery"
            | "home.editing.editing"
            | "view.zoom.100"
            | "view.zoom.page-width" => {
                if has {
                    CommandState::ENABLED
                } else {
                    CommandState::DISABLED
                }
            }
            "status.print-layout" => CommandState::checked(true),
            _ => CommandState::DISABLED,
        }
    }

    fn run(&mut self, id: &str, value: Option<f64>) {
        let s = &mut *self.s;
        if let Some(rest) = id.strip_prefix("home.styles.quickstylesgallery#") {
            if let Some(t) = &mut s.target {
                s.doc.set_style(t, rest);
                s.doc.focus();
            }
            return;
        }
        if let Some(name) = id.strip_prefix("home.font.font#") {
            s.run_props(RunPropsPatch {
                font: Some(name.to_owned()),
                ..RunPropsPatch::default()
            });
            return;
        }
        if let Some(n) = id
            .strip_prefix("home.font.font-size#")
            .and_then(|n| n.parse::<u32>().ok())
        {
            s.run_props(RunPropsPatch {
                size_half_points: Some(n * 2),
                ..RunPropsPatch::default()
            });
            return;
        }
        if let Some(hex) = id.strip_prefix("home.font.font-colour#") {
            if hex != "auto" {
                s.font_color = hex.to_owned();
            }
            s.run_props(RunPropsPatch {
                color: Some(hex.to_owned()),
                ..RunPropsPatch::default()
            });
            return;
        }
        if let Some(i) = id
            .strip_prefix("file.open-recent#")
            .and_then(|i| i.parse::<usize>().ok())
        {
            if let Some(p) = s.recent.get(i).cloned() {
                s.open_path(&p);
            }
            return;
        }
        match id {
            "file.new" => s.new_blank(),
            "file.open" => s.open_dialog(),
            "qat.filesave" | "file.save" => s.save(),
            "file.save-as" => s.save_as(),
            "file.print" => s.print(),
            "file.export" => s.export_pdf(),
            "file.close" => s.target = None,
            "qat.undo" => s.undo(),
            "qat.redoorrepeat" | "qat.redo" => s.redo(),
            "home.clipboard.paste" => s.paste(),
            "home.clipboard.cut" => s.cut(),
            "home.clipboard.copy" => s.copy(),
            "home.font.bold" => s.toggle(Toggle::Bold),
            "home.font.italic" => s.toggle(Toggle::Italic),
            "home.font.underline" => s.toggle(Toggle::Underline),
            "home.font.strikethrough" => s.toggle(Toggle::Strike),
            "home.font.grow-font" => s.step_size(true),
            "home.font.shrink-font" => s.step_size(false),
            "home.font.font-colour" => {
                let c = s.font_color.clone();
                s.run_props(RunPropsPatch {
                    color: Some(c),
                    ..RunPropsPatch::default()
                });
            }
            "home.paragraph.align-left" => s.align(Alignment::Left),
            "home.paragraph.centre" => s.align(Alignment::Center),
            "home.paragraph.align-right" => s.align(Alignment::Right),
            "home.paragraph.justify" => s.align(Alignment::Justify),
            "edit.find" => s.find.show_dialog(false),
            "edit.replace" => s.find.show_dialog(true),
            "view.zoom.100" => s.doc.set_zoom(Some(1.0)),
            "view.zoom.page-width" => s.doc.set_zoom(None),
            "view.zoom" => s.doc.set_zoom(value.map(|z| z as f32)),
            _ => {}
        }
    }

    fn title(&self) -> String {
        match self.s.target.as_ref().and_then(|t| t.path()) {
            Some(p) => p
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            None if self.s.target.is_some() => format!("Document{}", self.s.untitled),
            None => "Open365".to_owned(),
        }
    }

    fn status_text(&self, id: &str) -> Option<String> {
        let s = &*self.s;
        s.target.as_ref()?;
        match id {
            "status.page" => {
                let c = s.doc.caret();
                Some(format!(
                    "Page {} of {}",
                    s.doc.page_of(c.para) + 1,
                    s.doc.page_count()
                ))
            }
            "status.words" => Some(format!("{} words", s.words.1)),
            "status.language" => Some("English".to_owned()),
            _ => None,
        }
    }

    fn zoom(&self) -> f32 {
        self.s.doc.zoom().unwrap_or_else(|| self.s.doc.scale())
    }

    fn menu(&self, id: &str) -> Option<Vec<MenuEntry>> {
        let has = self.s.target.is_some();
        match id {
            "home.editing.editing" => Some(vec![
                MenuEntry::new("edit.find", "Find", has),
                MenuEntry::new("edit.replace", "Replace", has),
            ]),
            "home.font.font" => Some(
                FONTS
                    .iter()
                    .map(|f| MenuEntry::new(&format!("home.font.font#{f}"), f, has))
                    .collect(),
            ),
            "home.font.font-size" => Some(
                SIZES
                    .iter()
                    .map(|n| {
                        MenuEntry::new(&format!("home.font.font-size#{n}"), &n.to_string(), has)
                    })
                    .collect(),
            ),
            "home.font.font-colour" => {
                let mut v = vec![MenuEntry::new(
                    "home.font.font-colour#auto",
                    "Automatic",
                    has,
                )];
                v.extend(COLORS.iter().map(|(hex, name)| {
                    MenuEntry::new(&format!("home.font.font-colour#{hex}"), name, has)
                }));
                Some(v)
            }
            "home.styles.quickstylesgallery" => Some(
                STYLES
                    .iter()
                    .map(|(sid, label)| {
                        MenuEntry::new(&format!("home.styles.quickstylesgallery#{sid}"), label, has)
                    })
                    .collect(),
            ),
            _ => None,
        }
    }

    fn value(&self, id: &str) -> Option<String> {
        match id {
            "home.font.font" if self.s.target.is_some() => {
                Some(self.s.doc.caret_format().font.to_string())
            }
            "home.font.font-size" if self.s.target.is_some() => {
                let h = self.s.doc.caret_format().size_half_points;
                Some(if h.is_multiple_of(2) {
                    (h / 2).to_string()
                } else {
                    format!("{}.5", h / 2)
                })
            }
            "home.styles.quickstylesgallery" => {
                Some(self.s.doc.caret_style_id().unwrap_or("Normal").to_owned())
            }
            _ => None,
        }
    }

    fn gallery(&self, id: &str) -> Vec<(String, String)> {
        if id == "home.styles.quickstylesgallery" {
            STYLES
                .iter()
                .take(3)
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect()
        } else {
            Vec::new()
        }
    }

    fn recent(&self) -> Vec<String> {
        self.s
            .recent
            .iter()
            .map(|p| p.display().to_string())
            .collect()
    }

    fn info(&self) -> Vec<(String, String)> {
        let s = &*self.s;
        let Some(t) = &s.target else {
            return Vec::new();
        };
        vec![
            ("Name".into(), self.title()),
            (
                "Location".into(),
                t.path()
                    .map_or("Not saved yet".into(), |p| p.display().to_string()),
            ),
            (
                "Unsaved changes".into(),
                if t.is_dirty() { "Yes" } else { "No" }.into(),
            ),
            ("Pages".into(), s.doc.page_count().to_string()),
        ]
    }

    fn central(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let s = &mut *self.s;
        s.shortcuts(&ctx);
        let _ = s.word_count();
        let mut repaint = false;
        match &mut s.target {
            Some(t) => {
                let th = &self.theme;
                s.doc.set_page_colors(th.page, th.page_text, th.page_border);
                repaint = s.doc.show(ui, t);
            }
            None => {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.heading("Open365 Writer");
                    ui.label("Create or open a document to begin.");
                    if ui.button("New blank document").clicked() {
                        s.new_blank();
                    }
                    if ui.button("Open...").clicked() {
                        s.open_dialog();
                    }
                });
            }
        }
        if let Some(m) = s.doc.status.take() {
            s.status = m;
        }
        let is_docx = s.target.is_some();
        if let Some(action) = s.find.ui(&ctx, is_docx)
            && let Some(t) = &mut s.target
        {
            s.status = s.find.run(action, t, &mut s.doc, &mut s.grid);
        }
        if repaint {
            ctx.request_repaint();
        }
    }
}

struct WriterApp {
    shell: Shell,
    state: State,
    title: String,
}

impl eframe::App for WriterApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let mut host = Host {
            s: &mut self.state,
            theme: self.shell.theme,
        };
        self.shell.show(ui, &mut host);
        let title = format!("{} - Open365 Writer", host.title());
        if title != self.title {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

fn main() -> eframe::Result {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Open365 Writer")
            .with_inner_size([1550.0, 926.0])
            .with_decorations(false)
            .with_maximized(true)
            .with_min_inner_size([160.0, 160.0]),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Open365 Writer",
        options,
        Box::new(move |cc| {
            fonts::install(&cc.egui_ctx);
            let mut state = State::new();
            match initial {
                Some(p) => state.open_path(&p),
                None => state.new_blank(),
            }
            Ok(Box::new(WriterApp {
                shell: {
                    let mut shell = Shell::new(AppKind::Writer, Mode::system());
                    shell.options.custom_frame = true;
                    shell
                },
                state,
                title: String::new(),
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both applications take the mode from the same source (Windows' app mode, as Office does).
    #[test]
    fn theme_follows_the_system_mode() {
        let shell = Shell::new(AppKind::Writer, Mode::system());
        assert_eq!(shell.theme.mode, Mode::system());
    }

    #[test]
    fn blank_document_opens_and_saves() {
        let mut t = Target::open_docx(&blank::docx()).expect("blank docx opens");
        assert!(t.docx().is_some());
        assert!(t.snapshot_bytes().is_ok());
    }
}
