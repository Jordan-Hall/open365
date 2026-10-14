//! The windowed preview: a [`UiHandler`] around [`Shell`], with the window controls that a
//! borderless window needs (drag by the title row, resize at the edges, minimise, maximise).

use std::time::{Duration, Instant};

use open365_gfx::{DrawList, TextOptions};
use open365_ui_core::geom::Point;
use open365_ui_core::{Emit, PlatformSettings, Ui};
use open365_ui_host::winit::dpi::PhysicalSize;
use open365_ui_host::winit::event::{ElementState, MouseButton, WindowEvent};
use open365_ui_host::winit::keyboard::{Key, NamedKey};
use open365_ui_host::winit::window::{CursorIcon, ResizeDirection};
use open365_ui_host::{
    AdapterConfig, ColorScheme, Cx, HostConfig, HostError, TitleBarTheme, UiAdapter, UiHandler,
};

use crate::shell::{Shell, WinAct};
use crate::{AppKind, fonts};

/// Width of the resize border in logical pixels.
const BORDER: f64 = 5.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(450);

struct Handler {
    shell: Shell,
    pointer_phys: (f64, f64),
    last_title_click: Option<Instant>,
    /// A workbook named on the command line (Sheets).
    open: Option<std::path::PathBuf>,
    kind: AppKind,
}

impl Handler {
    fn edge(&self, cx: &Cx<'_>) -> Option<ResizeDirection> {
        if cx.window.is_maximized() {
            return None;
        }
        let PhysicalSize { width, height } = cx.window.inner_size();
        let (x, y) = self.pointer_phys;
        let b = BORDER * cx.scale_factor;
        let (w, h) = (f64::from(width), f64::from(height));
        let (l, r, t, bt) = (x < b, x >= w - b, y < b, y >= h - b);
        Some(match (l, r, t, bt) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (.., true) => ResizeDirection::South,
            _ => return None,
        })
    }

    fn apply(&mut self, ui: &mut Ui, cx: &mut Cx<'_>, acts: Vec<WinAct>) {
        for a in acts {
            match a {
                WinAct::Minimize => cx.window.set_minimized(true),
                WinAct::ToggleMaximize => {
                    let m = !cx.window.is_maximized();
                    cx.window.set_maximized(m);
                    self.shell.maximized = m;
                    self.shell.build_all(ui);
                }
                WinAct::Close => cx.request_exit(),
                WinAct::Title(t) => cx.set_title(&t),
                WinAct::OpenDocument(path) => open_in_new_window(&path),
            }
        }
    }
}

impl UiHandler for Handler {
    fn init(&mut self, ui: &mut Ui, cx: &mut Cx<'_>) {
        self.shell.init(ui);
        if self.kind == AppKind::Writer {
            let mut pane = match self.open.take() {
                Some(path) => crate::doc::WriterPane::open(&path).unwrap_or_else(|e| {
                    eprintln!("cannot open {}: {e}", path.display());
                    crate::doc::WriterPane::blank()
                }),
                None => crate::doc::WriterPane::blank(),
            };
            pane.use_system_fonts();
            pane.set_clipboard(Box::new(open365_ui_host::RichClipboard::new()));
            self.shell.set_writer(ui, pane);
        } else {
            self.shell.set_locale(ui, crate::sheet::barfmt::os_locale());
        }
        if self.kind == AppKind::Sheets
            && let Some(path) = self.open.take()
        {
            match crate::sheet::SheetModel::open(&path) {
                Ok(model) => self.shell.set_sheet(ui, model),
                Err(e) => eprintln!("cannot open {}: {e}", path.display()),
            }
        }
        if self.kind == AppKind::Sheets {
            // The OS dialogs; headless runs and tests leave the choosers unset.
            self.shell.install_sheet_choosers();
        }
        cx.set_title(&self.shell.title());
    }

    fn resized(&mut self, ui: &mut Ui, cx: &mut Cx<'_>) {
        self.shell.maximized = cx.window.is_maximized();
        self.shell.resize(ui);
    }

    fn settings_changed(&mut self, ui: &mut Ui, _cx: &mut Cx<'_>) {
        self.shell.settings_changed(ui);
    }

    fn emitted(&mut self, ui: &mut Ui, cx: &mut Cx<'_>, events: Vec<Emit>) {
        let acts = self.shell.handle(ui, events);
        self.apply(ui, cx, acts);
    }

    fn window_event(&mut self, ui: &mut Ui, cx: &mut Cx<'_>, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer_phys = (position.x, position.y);
                let icon = match self.edge(cx) {
                    Some(ResizeDirection::West | ResizeDirection::East) => CursorIcon::EwResize,
                    Some(ResizeDirection::North | ResizeDirection::South) => CursorIcon::NsResize,
                    Some(ResizeDirection::NorthWest | ResizeDirection::SouthEast) => {
                        CursorIcon::NwseResize
                    }
                    Some(ResizeDirection::NorthEast | ResizeDirection::SouthWest) => {
                        CursorIcon::NeswResize
                    }
                    None => CursorIcon::Default,
                };
                cx.window.set_cursor(icon);
                false
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(dir) = self.edge(cx) {
                    let _ = cx.window.drag_resize_window(dir);
                    return true;
                }
                let s = cx.scale_factor;
                let p = Point::new(
                    (self.pointer_phys.0 / s) as f32,
                    (self.pointer_phys.1 / s) as f32,
                );
                if self.shell.press_outside_popup(ui, p) {
                    self.shell.close_group_popup(ui);
                }
                if self.shell.is_drag_area(ui, p) {
                    let double = self
                        .last_title_click
                        .is_some_and(|t| cx.now.duration_since(t) < DOUBLE_CLICK);
                    self.last_title_click = Some(cx.now);
                    if double {
                        self.last_title_click = None;
                        self.apply(ui, cx, vec![WinAct::ToggleMaximize]);
                    } else {
                        let _ = cx.window.drag_window();
                    }
                    return true;
                }
                false
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                self.shell.escape(ui)
            }
            _ => false,
        }
    }

    fn paint_below(&mut self, ui: &mut Ui, list: &mut DrawList) {
        self.shell.paint_below(ui, list);
    }
}

fn parse_args() -> (Option<bool>, bool, Option<std::path::PathBuf>) {
    let mut dark = None;
    let mut grayscale = true;
    let mut path = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--dark" => dark = Some(true),
            "--light" => dark = Some(false),
            "--lcd" => grayscale = false,
            other if !other.starts_with("--") => path = Some(other.into()),
            _ => {}
        }
    }
    (dark, grayscale, path)
}

/// Opens the preview window for `kind`. `--dark` and `--light` force the app mode; the default
/// follows the system.
pub fn run(kind: AppKind) -> Result<(), HostError> {
    let (forced, grayscale, open) = parse_args();
    let dark = forced.unwrap_or_else(|| PlatformSettings::read().apps_dark);
    let f = fonts::load();
    let mut cfg = AdapterConfig::new(f.primary);
    cfg.font_index = f.index;
    cfg.fallbacks = f.fallbacks;
    cfg.text_options = Some(TextOptions::office(!grayscale));
    cfg.force_scheme = forced.map(|d| {
        if d {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        }
    });
    let name = match kind {
        AppKind::Writer => format!("{} (next)", open365_branding::WRITE_NAME),
        AppKind::Sheets => format!("{} (next)", open365_branding::SHEET_NAME),
    };
    let handler = Handler {
        shell: Shell::new(kind, dark),
        pointer_phys: (-1.0, -1.0),
        last_title_click: None,
        kind,
        open,
    };
    open365_ui_host::run(
        HostConfig {
            title: name,
            // 1600 x 1000 device pixels at 125 %, the size the specs were measured at.
            inner_size: (1280.0, 800.0),
            min_inner_size: Some((400.0, 300.0)),
            title_bar: if dark {
                TitleBarTheme::Dark
            } else {
                TitleBarTheme::Light
            },
            decorations: false,
            icon: window_icon(kind),
            ..HostConfig::default()
        },
        UiAdapter::new(handler, cfg),
    )
}

/// The window and taskbar icon of `kind` (the same art as the executable resource).
fn window_icon(kind: AppKind) -> Option<open365_ui_host::WindowIcon> {
    let png: &[u8] = match kind {
        AppKind::Writer => include_bytes!("../../../assets/icons/open-write-256.png"),
        AppKind::Sheets => include_bytes!("../../../assets/icons/open-sheet-256.png"),
    };
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let (width, height) = img.dimensions();
    Some(open365_ui_host::WindowIcon {
        rgba: img.into_raw(),
        width,
        height,
    })
}

/// Starts another instance of this program on `path`, as Word opens a merge result in its own
/// window. A failure is reported on stderr; the current document is untouched.
fn open_in_new_window(path: &std::path::Path) {
    let spawned =
        std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg(path).spawn());
    if let Err(e) = spawned {
        eprintln!("cannot open {} in a new window: {e}", path.display());
    }
}
