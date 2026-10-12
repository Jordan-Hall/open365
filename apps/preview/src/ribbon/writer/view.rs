//! Writer ribbon: the View tab (`view.*`): views, show, zoom, window and macros.
//!
//! The view mode, the Show options and the window flags are kept for the pane
//! ([`WriterPane::view_state`]); the zoom presets change the pane zoom. A bare `view.zoom.zoom`
//! has no dialog host yet; `view.zoom.zoom=150` (a typed value) sets the zoom.

use crate::doc::WriterPane;
use crate::ribbon::{Handled, RibbonCtx, base};

/// The document views of the Views group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Read,
    PrintLayout,
    Web,
    Outline,
    Draft,
}

impl ViewMode {
    /// The name the status bar uses.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "Read Mode",
            Self::PrintLayout => "Print Layout",
            Self::Web => "Web Layout",
            Self::Outline => "Outline",
            Self::Draft => "Draft",
        }
    }
}

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "view.views.read-mode",
    "view.views.print-layout",
    "view.views.web-layout",
    "view.views.outline",
    "view.views.draft",
    "view.immersive.focus",
    "view.immersive.reader",
    "view.appearance.switch-theme",
    "view.page-movement.vertical",
    "view.page-movement.side-to-side",
    "view.show.ruler",
    "view.show.gridlines",
    "view.show.navigation-pane",
    "view.zoom.zoom",
    "view.zoom.100",
    "view.zoom.one-page",
    "view.zoom.multiple-pages",
    "view.zoom.page-width",
    "view.window.new-window",
    "view.window.arrange-all",
    "view.window.split",
    "view.window.view-side-by-side",
    "view.window.synchronous-scrolling",
    "view.window.reset-window-position",
    "view.window.switch-windows",
    "view.macros.macros",
    "view.server.properties",
];

fn note(p: &mut WriterPane, s: &str) {
    p.status = Some(s.to_owned());
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let typed = id.split_once('=').map(|(_, v)| v.trim().to_owned());
    let id = base(id);
    if !IDS.contains(&id) {
        return Handled::No;
    }
    let Some(p) = ctx.writer.as_deref_mut() else {
        return Handled::Yes;
    };
    match id {
        "view.views.read-mode" => p.set_view_mode(ViewMode::Read),
        "view.views.print-layout" => p.set_view_mode(ViewMode::PrintLayout),
        "view.views.web-layout" => p.set_view_mode(ViewMode::Web),
        "view.views.outline" => p.set_view_mode(ViewMode::Outline),
        "view.views.draft" => p.set_view_mode(ViewMode::Draft),
        "view.immersive.reader" => {
            let next = if p.view_state().mode == ViewMode::Read {
                ViewMode::PrintLayout
            } else {
                ViewMode::Read
            };
            p.set_view_mode(next);
        }
        "view.immersive.focus" => p.edit_view_state(|v| v.focus = !v.focus),
        "view.appearance.switch-theme" => p.edit_view_state(|v| v.dark = !v.dark),
        "view.page-movement.vertical" => p.edit_view_state(|v| v.side_to_side = false),
        "view.page-movement.side-to-side" => p.edit_view_state(|v| v.side_to_side = true),
        "view.show.ruler" => {
            let on = !p.ruler();
            p.set_ruler(on);
        }
        "view.show.gridlines" => p.edit_view_state(|v| v.gridlines = !v.gridlines),
        "view.show.navigation-pane" => p.edit_view_state(|v| v.nav_pane = !v.nav_pane),
        "view.zoom.zoom" => {
            match typed.and_then(|t| t.trim_end_matches('%').trim().parse::<f32>().ok()) {
                Some(percent) => p.set_zoom_percent(percent),
                None => note(p, "Zoom: type a percentage between 10 and 500"),
            }
        }
        "view.zoom.100" => p.set_zoom_percent(100.0),
        "view.zoom.one-page" => p.zoom_one_page(),
        "view.zoom.multiple-pages" => p.zoom_multiple_pages(),
        "view.zoom.page-width" => p.zoom_page_width(),
        "view.window.new-window" => note(p, "New Window needs the window host; not available yet"),
        "view.window.arrange-all" | "view.window.switch-windows" => {
            note(p, "Only one window is open")
        }
        "view.window.split" => p.edit_view_state(|v| v.split = !v.split),
        "view.window.view-side-by-side" => {
            p.edit_view_state(|v| v.side_by_side = false);
            note(p, "Open a second window to view side by side");
        }
        "view.window.synchronous-scrolling" => {
            p.edit_view_state(|v| v.sync_scroll = !v.sync_scroll)
        }
        "view.window.reset-window-position" => p.edit_view_state(|v| v.side_by_side = false),
        "view.macros.macros" => note(p, "Macros are disabled: documents never run code"),
        "view.server.properties" => note(p, "Server properties: this document is not on a server"),
        _ => return Handled::No,
    }
    *ctx.refocus_doc = true;
    Handled::Yes
}
