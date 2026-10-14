//! winit application host for the Open365 UI (ADR `docs/architecture/adr-ui-platform.md`).
//!
//! [`run`] opens a window with a wgpu surface, keeps per-monitor DPI, redraws on demand with
//! timed wake-ups (no busy loop), connects AccessKit, and forwards IME composition to an
//! [`ImeHandler`]. The application implements [`UiApp`] and draws with `open365-gfx`.
//!
//! The testable parts ([`Scheduler`], [`CaretBlink`], IME translation, theme mapping) are plain
//! Rust and run without a window.

mod adapter;
mod app;
pub mod bridge;
pub mod clipboard;
mod host;
pub mod ime;
mod schedule;
pub mod settings;
mod theme;

pub use accesskit;
pub use adapter::{AdapterConfig, UiAdapter, UiHandler, ime_input};
pub use app::{Cx, Frame, UiApp, UiEvent};
pub use bridge::InputBridge;
pub use clipboard::{RichClipboard, SystemClipboard};
pub use host::{HostConfig, HostError, WindowIcon, run};
pub use ime::{ImeCursorArea, ImeEvent, ImeHandler};
pub use schedule::{CaretBlink, Scheduler};
pub use settings::SettingsWatcher;
pub use theme::{ColorScheme, SystemMetrics, TitleBarTheme};
pub use winit;
