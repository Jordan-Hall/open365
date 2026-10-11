//! Colours and text sizes.
//!
//! The dark values were sampled from the oracle's own window (colour values only). The light
//! values could not be measured without changing the oracle's Office theme, so they are
//! estimates, marked as such in `docs/architecture/app-shell.md`. Both applications resolve the
//! mode with [`Mode::system`], the same source Office uses by default.

use egui::Color32;

/// Which application the chrome belongs to; it sets the accent colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    Writer,
    Sheets,
}

/// Light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

/// Every colour the chrome uses.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub mode: Mode,
    pub accent: Color32,
    /// Window frame behind the ribbon card (title row and tab row).
    pub frame: Color32,
    /// The ribbon body, formula bar and boxes.
    pub card: Color32,
    pub card_border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_disabled: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub checked: Color32,
    pub separator: Color32,
    pub status: Color32,
    pub field: Color32,
    pub field_border: Color32,
    /// The title-row search box (darker than the ribbon fields in dark mode).
    pub search_fill: Color32,
    pub search_border: Color32,
    /// The filled Share button.
    pub share: Color32,
    pub backstage_nav: Color32,
    pub backstage: Color32,
    pub on_accent: Color32,
    pub close_hover: Color32,
    /// Document surfaces. Office keeps Excel's cells white and Word's page light-on-light only in
    /// light mode; in dark mode Excel's cells stay white and Word's page turns dark.
    pub canvas: Color32,
    /// Word page fill and the colour of automatic-colour text on it.
    pub page: Color32,
    pub page_text: Color32,
    /// Word draws a 1 px outline around the page in dark mode instead of a shadow.
    pub page_border: Option<Color32>,
    pub cell_bg: Color32,
    pub cell_text: Color32,
    pub grid_line: Color32,
    pub grid_header: Color32,
    pub grid_header_selected: Color32,
    pub grid_header_text: Color32,
    pub grid_header_border: Color32,
    /// Excel's selected-header underline and active tab underline.
    pub grid_accent: Color32,
    pub tab_active: Color32,
    pub tab_active_text: Color32,
    pub scroll_track: Color32,
    pub scroll_thumb: Color32,
    /// Bright dropdown/function icons (formula strip) and dim idle icons (Cancel, Enter).
    pub icon: Color32,
    pub icon_off: Color32,
    /// Gallery cell lift and ring of the current or hovered cell.
    pub gallery_cell: Color32,
    pub gallery_ring: Color32,
    /// Drop-down menus: fill, 1 px border (sampled in dark mode; light values are estimates), hot row, colour cell ring.
    pub menu_bg: Color32,
    pub menu_border: Color32,
    pub menu_hot: Color32,
    pub menu_ring: Color32,
    /// Word's rulers: white track over the text area, the 1 px line under the band, marker fill and outline.
    /// Dark values are sampled (`word-ruler.json`); light values are estimates.
    pub ruler_track: Color32,
    pub ruler_line: Color32,
    pub ruler_marker: Color32,
    pub ruler_marker_border: Color32,
}

impl Mode {
    /// The mode Office uses with "Use system setting": Windows' app light/dark mode
    /// (`AppsUseLightTheme`, read only). Light on other platforms or when it cannot be read.
    #[must_use]
    pub fn system() -> Self {
        #[cfg(windows)]
        {
            let dark = windows_registry::CURRENT_USER
                .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
                .and_then(|k| k.get_u32("AppsUseLightTheme"))
                .is_ok_and(|v| v == 0);
            if dark {
                return Self::Dark;
            }
        }
        Self::Light
    }
}

impl Theme {
    #[must_use]
    pub fn new(kind: AppKind, mode: Mode) -> Self {
        match mode {
            Mode::Dark => Self {
                mode,
                accent: match kind {
                    AppKind::Writer => Color32::from_rgb(0x82, 0xab, 0xf1),
                    AppKind::Sheets => Color32::from_rgb(0x60, 0xbd, 0x82),
                },
                frame: Color32::from_rgb(0x09, 0x09, 0x09),
                card: Color32::from_rgb(0x29, 0x29, 0x29),
                card_border: Color32::from_rgb(0x3d, 0x3d, 0x3d),
                text: Color32::WHITE,
                text_dim: Color32::from_rgb(0xb2, 0xb2, 0xb2),
                text_disabled: Color32::from_rgb(0x6b, 0x6b, 0x6b),
                hover: Color32::from_rgb(0x3a, 0x3a, 0x3a),
                pressed: Color32::from_rgb(0x4a, 0x4a, 0x4a),
                checked: Color32::from_rgb(0x4d, 0x4d, 0x4d),
                separator: Color32::from_rgb(0x3d, 0x3d, 0x3d),
                status: Color32::from_rgb(0x14, 0x14, 0x14),
                field: Color32::from_rgb(0x29, 0x29, 0x29),
                field_border: Color32::from_rgb(0x75, 0x75, 0x75),
                search_fill: Color32::from_rgb(0x14, 0x14, 0x14),
                search_border: Color32::from_rgb(0x42, 0x42, 0x42),
                share: match kind {
                    AppKind::Writer => Color32::from_rgb(0x59, 0x8f, 0xec),
                    AppKind::Sheets => Color32::from_rgb(0x37, 0xa6, 0x60),
                },
                backstage_nav: Color32::from_rgb(0x09, 0x09, 0x09),
                backstage: Color32::from_rgb(0x14, 0x14, 0x14),
                on_accent: Color32::BLACK,
                close_hover: Color32::from_rgb(0xc4, 0x2b, 0x1c),
                // Sampled from the oracle (dark mode): Word canvas 090909, page 262626 with a 8c8c8c
                // outline; Excel cells ffffff on e0e0e0 lines, headers 0a0a0a, selected header 646464
                // with a 217346 underline, scrollbar thumb 999999 on 141414.
                canvas: Color32::from_rgb(0x09, 0x09, 0x09),
                page: Color32::from_rgb(0x26, 0x26, 0x26),
                page_text: Color32::WHITE,
                page_border: Some(Color32::from_rgb(0x8c, 0x8c, 0x8c)),
                cell_bg: Color32::WHITE,
                cell_text: Color32::BLACK,
                grid_line: Color32::from_rgb(0xe0, 0xe0, 0xe0),
                grid_header: Color32::from_rgb(0x0a, 0x0a, 0x0a),
                grid_header_selected: Color32::from_rgb(0x64, 0x64, 0x64),
                grid_header_text: Color32::from_rgb(0xda, 0xda, 0xda),
                grid_header_border: Color32::from_rgb(0x99, 0x99, 0x99),
                grid_accent: Color32::from_rgb(0x21, 0x73, 0x46),
                tab_active: Color32::WHITE,
                tab_active_text: Color32::from_rgb(0x24, 0x24, 0x24),
                scroll_track: Color32::from_rgb(0x14, 0x14, 0x14),
                scroll_thumb: Color32::from_rgb(0x99, 0x99, 0x99),
                icon: Color32::from_rgb(0xd4, 0xd4, 0xd4),
                icon_off: Color32::from_rgb(0x83, 0x83, 0x83),
                gallery_cell: Color32::from_rgb(0x38, 0x38, 0x38),
                gallery_ring: Color32::from_rgb(0xad, 0xad, 0xad),
                menu_bg: Color32::from_rgb(0x29, 0x29, 0x29),
                menu_border: Color32::from_rgb(0xad, 0xad, 0xad),
                menu_hot: Color32::from_rgb(0x40, 0x40, 0x40),
                menu_ring: Color32::from_rgb(0x40, 0x40, 0x40),
                ruler_track: Color32::from_rgb(0x29, 0x29, 0x29),
                ruler_line: Color32::from_rgb(0x61, 0x61, 0x61),
                ruler_marker: Color32::from_rgb(0xbd, 0xbd, 0xbd),
                ruler_marker_border: Color32::from_rgb(0x61, 0x61, 0x61),
            },
            Mode::Light => Self {
                mode,
                accent: match kind {
                    AppKind::Writer => Color32::from_rgb(0x18, 0x5a, 0xbd),
                    AppKind::Sheets => Color32::from_rgb(0x10, 0x7c, 0x41),
                },
                frame: Color32::from_rgb(0xf5, 0xf5, 0xf5),
                card: Color32::WHITE,
                card_border: Color32::from_rgb(0xe0, 0xe0, 0xe0),
                text: Color32::from_rgb(0x24, 0x24, 0x24),
                text_dim: Color32::from_rgb(0x61, 0x61, 0x61),
                text_disabled: Color32::from_rgb(0xa8, 0xa8, 0xa8),
                hover: Color32::from_rgb(0xeb, 0xeb, 0xeb),
                pressed: Color32::from_rgb(0xdc, 0xdc, 0xdc),
                checked: Color32::from_rgb(0xdd, 0xe8, 0xf6),
                separator: Color32::from_rgb(0xe0, 0xe0, 0xe0),
                status: Color32::from_rgb(0xf5, 0xf5, 0xf5),
                field: Color32::WHITE,
                field_border: Color32::from_rgb(0xc8, 0xc8, 0xc8),
                search_fill: Color32::WHITE,
                search_border: Color32::from_rgb(0xc8, 0xc8, 0xc8),
                share: match kind {
                    AppKind::Writer => Color32::from_rgb(0x18, 0x5a, 0xbd),
                    AppKind::Sheets => Color32::from_rgb(0x10, 0x7c, 0x41),
                },
                backstage_nav: Color32::from_rgb(0xf5, 0xf5, 0xf5),
                backstage: Color32::WHITE,
                on_accent: Color32::WHITE,
                close_hover: Color32::from_rgb(0xc4, 0x2b, 0x1c),
                // ESTIMATES (light mode cannot be sampled without changing Office's theme).
                canvas: Color32::from_rgb(0xe6, 0xe6, 0xe6),
                page: Color32::WHITE,
                page_text: Color32::BLACK,
                page_border: None,
                cell_bg: Color32::WHITE,
                cell_text: Color32::BLACK,
                grid_line: Color32::from_rgb(0xe0, 0xe0, 0xe0),
                grid_header: Color32::from_rgb(0xf5, 0xf5, 0xf5),
                grid_header_selected: Color32::from_rgb(0xd2, 0xd2, 0xd2),
                grid_header_text: Color32::from_rgb(0x44, 0x44, 0x44),
                grid_header_border: Color32::from_rgb(0xc8, 0xc8, 0xc8),
                grid_accent: Color32::from_rgb(0x21, 0x73, 0x46),
                tab_active: Color32::WHITE,
                tab_active_text: Color32::from_rgb(0x24, 0x24, 0x24),
                scroll_track: Color32::from_rgb(0xf5, 0xf5, 0xf5),
                scroll_thumb: Color32::from_rgb(0xc2, 0xc2, 0xc2),
                icon: Color32::from_rgb(0x42, 0x42, 0x42),
                icon_off: Color32::from_rgb(0x8a, 0x8a, 0x8a),
                gallery_cell: Color32::from_rgb(0xf0, 0xf0, 0xf0),
                gallery_ring: Color32::from_rgb(0x8a, 0x8a, 0x8a),
                menu_bg: Color32::from_rgb(0xff, 0xff, 0xff),
                menu_border: Color32::from_rgb(0xc6, 0xc6, 0xc6),
                menu_hot: Color32::from_rgb(0xe6, 0xe6, 0xe6),
                menu_ring: Color32::from_rgb(0xe1, 0xe1, 0xe1),
                ruler_track: Color32::WHITE,
                ruler_line: Color32::from_rgb(0xc6, 0xc6, 0xc6),
                ruler_marker: Color32::from_rgb(0xf3, 0xf3, 0xf3),
                ruler_marker_border: Color32::from_rgb(0x80, 0x80, 0x80),
            },
        }
    }

    /// egui visuals that agree with this theme (dialogs, text fields and popups follow the mode).
    #[must_use]
    pub fn visuals(&self) -> egui::Visuals {
        let mut v = match self.mode {
            Mode::Dark => egui::Visuals::dark(),
            Mode::Light => egui::Visuals::light(),
        };
        v.panel_fill = self.canvas;
        v.window_fill = self.card;
        v.extreme_bg_color = self.field;
        // Text inside egui widgets (Name Box, formula field) is the chrome's text colour, not egui's grey.
        v.widgets.noninteractive.fg_stroke.color = self.text;
        v.widgets.inactive.fg_stroke.color = self.text;
        v.selection.stroke.color = self.accent;
        // The text caret of the cell editor and formula bar follows Windows: blink time and width, drawn in the text colour.
        let caret = crate::caret::CaretSettings::system();
        v.text_cursor.stroke = egui::Stroke::new(caret.width, self.text);
        v.text_cursor.blink = caret.blink_ms.is_some();
        if let Some(s) = caret.blink_secs() {
            v.text_cursor.on_duration = s as f32;
            v.text_cursor.off_duration = s as f32;
        }
        v
    }

    /// Text colour for a control: normal, dimmed when disabled.
    #[must_use]
    pub fn ink(&self, enabled: bool) -> Color32 {
        if enabled {
            self.text
        } else {
            self.text_disabled
        }
    }
}

/// Text sizes in points (device-independent pixels).
pub mod size {
    /// Ribbon control and group labels (9 pt).
    pub const LABEL: f32 = 12.0;
    /// Tab names.
    pub const TAB: f32 = 14.0;
    /// Document name in the title bar.
    pub const TITLE: f32 = 12.0;
    /// Group names under the ribbon (measured narrower than control labels).
    pub const GROUP: f32 = 11.0;
    /// Status bar text.
    pub const STATUS: f32 = 10.8;
    /// Backstage navigation.
    pub const NAV: f32 = 14.0;
    /// Backstage headings.
    pub const HEADING: f32 = 22.0;
}
