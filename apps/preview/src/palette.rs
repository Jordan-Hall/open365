//! Colours beyond the `ui-core` tokens: the document surfaces, the status bar and the other
//! fills the chrome uses. The dark values were sampled from Office by the egui shell
//! (`ui-shell/src/theme.rs`, colour values only); the light ones are estimates there too.

use open365_gfx::Color;

use crate::AppKind;

const fn c(r: u8, g: u8, b: u8) -> Color {
    Color::rgb(r, g, b)
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub accent: Color,
    pub on_accent: Color,
    /// Window frame behind the ribbon card (title row and tab row).
    pub frame: Color,
    pub status: Color,
    pub search_fill: Color,
    pub search_border: Color,
    pub share: Color,
    pub backstage_nav: Color,
    pub backstage: Color,
    pub close_hover: Color,
    pub canvas: Color,
    pub page: Color,
    pub page_text: Color,
    pub page_border: Option<Color>,
    pub cell_bg: Color,
    pub cell_text: Color,
    pub grid_line: Color,
    pub grid_header: Color,
    pub grid_header_selected: Color,
    pub grid_header_text: Color,
    pub grid_header_border: Color,
    pub grid_accent: Color,
    pub tab_active: Color,
    pub tab_active_text: Color,
}

impl Palette {
    pub fn new(kind: AppKind, dark: bool) -> Self {
        if dark {
            Self {
                dark,
                accent: match kind {
                    AppKind::Writer => c(0x82, 0xab, 0xf1),
                    AppKind::Sheets => c(0x60, 0xbd, 0x82),
                },
                on_accent: c(0, 0, 0),
                frame: c(0x09, 0x09, 0x09),
                status: c(0x14, 0x14, 0x14),
                search_fill: c(0x14, 0x14, 0x14),
                search_border: c(0x42, 0x42, 0x42),
                share: match kind {
                    AppKind::Writer => c(0x59, 0x8f, 0xec),
                    AppKind::Sheets => c(0x37, 0xa6, 0x60),
                },
                backstage_nav: c(0x09, 0x09, 0x09),
                backstage: c(0x14, 0x14, 0x14),
                close_hover: c(0xc4, 0x2b, 0x1c),
                canvas: c(0x09, 0x09, 0x09),
                page: c(0x26, 0x26, 0x26),
                page_text: c(255, 255, 255),
                page_border: Some(c(0x8c, 0x8c, 0x8c)),
                cell_bg: c(255, 255, 255),
                cell_text: c(0, 0, 0),
                grid_line: c(0xe0, 0xe0, 0xe0),
                grid_header: c(0x0a, 0x0a, 0x0a),
                grid_header_selected: c(0x64, 0x64, 0x64),
                grid_header_text: c(0xda, 0xda, 0xda),
                grid_header_border: c(0x99, 0x99, 0x99),
                grid_accent: c(0x21, 0x73, 0x46),
                tab_active: c(255, 255, 255),
                tab_active_text: c(0x24, 0x24, 0x24),
            }
        } else {
            Self {
                dark,
                accent: match kind {
                    AppKind::Writer => c(0x18, 0x5a, 0xbd),
                    AppKind::Sheets => c(0x10, 0x7c, 0x41),
                },
                on_accent: c(255, 255, 255),
                frame: c(0xf5, 0xf5, 0xf5),
                status: c(0xf5, 0xf5, 0xf5),
                search_fill: c(255, 255, 255),
                search_border: c(0xc8, 0xc8, 0xc8),
                share: match kind {
                    AppKind::Writer => c(0x18, 0x5a, 0xbd),
                    AppKind::Sheets => c(0x10, 0x7c, 0x41),
                },
                backstage_nav: c(0xf5, 0xf5, 0xf5),
                backstage: c(255, 255, 255),
                close_hover: c(0xc4, 0x2b, 0x1c),
                canvas: c(0xe6, 0xe6, 0xe6),
                page: c(255, 255, 255),
                page_text: c(0, 0, 0),
                page_border: None,
                cell_bg: c(255, 255, 255),
                cell_text: c(0, 0, 0),
                grid_line: c(0xe0, 0xe0, 0xe0),
                grid_header: c(0xf5, 0xf5, 0xf5),
                grid_header_selected: c(0xd2, 0xd2, 0xd2),
                grid_header_text: c(0x44, 0x44, 0x44),
                grid_header_border: c(0xc8, 0xc8, 0xc8),
                grid_accent: c(0x21, 0x73, 0x46),
                tab_active: c(255, 255, 255),
                tab_active_text: c(0x24, 0x24, 0x24),
            }
        }
    }
}
