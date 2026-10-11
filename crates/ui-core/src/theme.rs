//! Theme tokens. Light, dark and high contrast colour sets are JSON (`themes/*.json`; the chrome
//! JSON in `ui-shell/spec` holds geometry only, and the sampled dark values mirror
//! `ui-shell/src/theme.rs`). Geometry tokens come from the chrome specs through [`crate::chrome`].

use std::collections::HashMap;

use open365_gfx::Color;
use serde::Deserialize;

use crate::settings::PlatformSettings;

/// Which colour set is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeKind {
    Light,
    Dark,
    HighContrast,
}

/// Every colour token a widget may ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Token {
    Frame,
    Card,
    CardBorder,
    Text,
    TextDim,
    TextDisabled,
    Hover,
    Pressed,
    Checked,
    Separator,
    Field,
    FieldBorder,
    Accent,
    OnAccent,
    FocusRing,
    Selection,
    SelectionText,
    Caret,
    MenuBg,
    MenuBorder,
    MenuHot,
    ScrollTrack,
    ScrollThumb,
    TabText,
    TabUnderline,
    TooltipBg,
    TooltipBorder,
    TooltipText,
    Shadow,
    KeytipBg,
    KeytipText,
    KeytipBorder,
}

impl Token {
    pub const ALL: [Token; 32] = [
        Token::Frame,
        Token::Card,
        Token::CardBorder,
        Token::Text,
        Token::TextDim,
        Token::TextDisabled,
        Token::Hover,
        Token::Pressed,
        Token::Checked,
        Token::Separator,
        Token::Field,
        Token::FieldBorder,
        Token::Accent,
        Token::OnAccent,
        Token::FocusRing,
        Token::Selection,
        Token::SelectionText,
        Token::Caret,
        Token::MenuBg,
        Token::MenuBorder,
        Token::MenuHot,
        Token::ScrollTrack,
        Token::ScrollThumb,
        Token::TabText,
        Token::TabUnderline,
        Token::TooltipBg,
        Token::TooltipBorder,
        Token::TooltipText,
        Token::Shadow,
        Token::KeytipBg,
        Token::KeytipText,
        Token::KeytipBorder,
    ];

    /// The key used in the JSON files.
    pub fn key(self) -> &'static str {
        match self {
            Token::Frame => "frame",
            Token::Card => "card",
            Token::CardBorder => "card_border",
            Token::Text => "text",
            Token::TextDim => "text_dim",
            Token::TextDisabled => "text_disabled",
            Token::Hover => "hover",
            Token::Pressed => "pressed",
            Token::Checked => "checked",
            Token::Separator => "separator",
            Token::Field => "field",
            Token::FieldBorder => "field_border",
            Token::Accent => "accent",
            Token::OnAccent => "on_accent",
            Token::FocusRing => "focus_ring",
            Token::Selection => "selection",
            Token::SelectionText => "selection_text",
            Token::Caret => "caret",
            Token::MenuBg => "menu_bg",
            Token::MenuBorder => "menu_border",
            Token::MenuHot => "menu_hot",
            Token::ScrollTrack => "scroll_track",
            Token::ScrollThumb => "scroll_thumb",
            Token::TabText => "tab_text",
            Token::TabUnderline => "tab_underline",
            Token::TooltipBg => "tooltip_bg",
            Token::TooltipBorder => "tooltip_border",
            Token::TooltipText => "tooltip_text",
            Token::Shadow => "shadow",
            Token::KeytipBg => "keytip_bg",
            Token::KeytipText => "keytip_text",
            Token::KeytipBorder => "keytip_border",
        }
    }
}

#[derive(Deserialize)]
struct ThemeFile {
    colors: HashMap<String, String>,
}

/// A resolved colour set.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub kind: ThemeKind,
    colors: HashMap<Token, Color>,
}

/// `#rrggbb` or `#rrggbbaa`.
pub fn parse_color(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

impl Theme {
    /// Parses a theme file. Every token must be present.
    pub fn from_json(kind: ThemeKind, json: &str) -> Result<Self, String> {
        let file: ThemeFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut colors = HashMap::new();
        for t in Token::ALL {
            let raw = file
                .colors
                .get(t.key())
                .ok_or_else(|| format!("missing token {}", t.key()))?;
            colors.insert(
                t,
                parse_color(raw).ok_or_else(|| format!("bad colour for {}", t.key()))?,
            );
        }
        Ok(Self { kind, colors })
    }

    pub fn light() -> Self {
        Self::from_json(ThemeKind::Light, include_str!("../themes/light.json"))
            .expect("embedded light theme")
    }

    pub fn dark() -> Self {
        Self::from_json(ThemeKind::Dark, include_str!("../themes/dark.json"))
            .expect("embedded dark theme")
    }

    /// The default high contrast set; [`Theme::for_settings`] replaces it by the user's colours.
    pub fn high_contrast() -> Self {
        Self::from_json(
            ThemeKind::HighContrast,
            include_str!("../themes/high-contrast.json"),
        )
        .expect("embedded high contrast theme")
    }

    /// The set in force for `settings`: high contrast wins, then the app mode. With system
    /// colours available the high contrast set uses them.
    pub fn for_settings(s: &PlatformSettings) -> Self {
        if s.high_contrast {
            let mut t = Self::high_contrast();
            if let Some(c) = s.hc_colors {
                let rgb = |v: [u8; 3]| Color::rgb(v[0], v[1], v[2]);
                for (tok, v) in [
                    (Token::Frame, c.window),
                    (Token::Card, c.window),
                    (Token::MenuBg, c.window),
                    (Token::Field, c.window),
                    (Token::TooltipBg, c.window),
                    (Token::Text, c.window_text),
                    (Token::TextDim, c.window_text),
                    (Token::CardBorder, c.window_text),
                    (Token::FieldBorder, c.window_text),
                    (Token::MenuBorder, c.window_text),
                    (Token::Separator, c.window_text),
                    (Token::TabText, c.window_text),
                    (Token::TooltipText, c.window_text),
                    (Token::Caret, c.window_text),
                    (Token::TextDisabled, c.gray_text),
                    (Token::Hover, c.highlight),
                    (Token::Pressed, c.highlight),
                    (Token::MenuHot, c.highlight),
                    (Token::Selection, c.highlight),
                    (Token::SelectionText, c.highlight_text),
                    (Token::ScrollThumb, c.window_text),
                    (Token::ScrollTrack, c.window),
                ] {
                    t.colors.insert(tok, rgb(v));
                }
            }
            t
        } else if s.apps_dark {
            Self::dark()
        } else {
            Self::light()
        }
    }

    /// Replaces one colour (an application accent, for example).
    pub fn set(&mut self, t: Token, c: Color) {
        self.colors.insert(t, c);
    }

    pub fn color(&self, t: Token) -> Color {
        self.colors.get(&t).copied().unwrap_or(Color::TRANSPARENT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_sets_define_every_token() {
        for t in [Theme::light(), Theme::dark(), Theme::high_contrast()] {
            for tok in Token::ALL {
                assert!(t.colors.contains_key(&tok), "{:?} lacks {tok:?}", t.kind);
            }
        }
    }

    #[test]
    fn dark_values_are_the_sampled_office_dark_values() {
        let d = Theme::dark();
        assert_eq!(d.color(Token::Frame), Color::rgb(0x09, 0x09, 0x09));
        assert_eq!(d.color(Token::Card), Color::rgb(0x29, 0x29, 0x29));
        assert_eq!(d.color(Token::MenuBorder), Color::rgb(0xad, 0xad, 0xad));
    }

    #[test]
    fn high_contrast_wins_over_dark_mode() {
        let s = PlatformSettings {
            high_contrast: true,
            apps_dark: true,
            ..PlatformSettings::default()
        };
        assert_eq!(Theme::for_settings(&s).kind, ThemeKind::HighContrast);
        let s = PlatformSettings {
            apps_dark: true,
            ..PlatformSettings::default()
        };
        assert_eq!(Theme::for_settings(&s).kind, ThemeKind::Dark);
        assert_eq!(
            Theme::for_settings(&PlatformSettings::default()).kind,
            ThemeKind::Light
        );
    }

    #[test]
    fn system_colours_override_the_default_high_contrast_set() {
        use crate::settings::HighContrastColors;
        let s = PlatformSettings {
            high_contrast: true,
            hc_colors: Some(HighContrastColors {
                window: [1, 2, 3],
                window_text: [250, 251, 252],
                highlight: [9, 8, 7],
                highlight_text: [6, 5, 4],
                gray_text: [100, 100, 100],
                button_face: [1, 2, 3],
            }),
            ..PlatformSettings::default()
        };
        let t = Theme::for_settings(&s);
        assert_eq!(t.color(Token::Text), Color::rgb(250, 251, 252));
        assert_eq!(t.color(Token::MenuHot), Color::rgb(9, 8, 7));
    }

    #[test]
    fn colour_parser_rejects_garbage() {
        assert_eq!(parse_color("#102030"), Some(Color::rgb(0x10, 0x20, 0x30)));
        assert_eq!(
            parse_color("#10203040"),
            Some(Color::rgba(0x10, 0x20, 0x30, 0x40))
        );
        assert_eq!(parse_color("102030"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#zz0000"), None);
    }

    #[test]
    fn missing_token_is_an_error() {
        assert!(Theme::from_json(ThemeKind::Light, r##"{"colors":{"text":"#000000"}}"##).is_err());
    }
}
