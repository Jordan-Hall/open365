//! Colour scheme and OS metrics the host reports to the application.

use std::time::Duration;

use winit::window::Theme;

/// The app mode (light or dark) the window is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
}

impl From<Theme> for ColorScheme {
    fn from(t: Theme) -> Self {
        match t {
            Theme::Light => Self::Light,
            Theme::Dark => Self::Dark,
        }
    }
}

/// How the native title bar is themed.
///
/// On Windows winit sets the DWM immersive dark mode attribute for the window and updates it
/// when the user flips "Choose your default app mode", so `FollowSystem` needs no `windows-sys`
/// call (and no `unsafe`) in this workspace. `Light` and `Dark` force the title bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitleBarTheme {
    #[default]
    FollowSystem,
    Light,
    Dark,
}

impl TitleBarTheme {
    pub(crate) fn to_winit(self) -> Option<Theme> {
        match self {
            Self::FollowSystem => None,
            Self::Light => Some(Theme::Light),
            Self::Dark => Some(Theme::Dark),
        }
    }
}

/// OS text-input metrics, read through the safe `windows-registry` crate (see
/// `open365_ui_core::settings`): caret blink and width, high contrast. Other platforms report the
/// Windows defaults. See the ADR section "Platform integration".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemMetrics {
    /// Time between caret on/off toggles; `None` for a steady caret.
    pub caret_blink: Option<Duration>,
    /// Caret width in device pixels.
    pub caret_width_px: u32,
    /// Windows high contrast theme active.
    pub high_contrast: bool,
}

impl SystemMetrics {
    /// Windows defaults: 530 ms blink, 1 px caret, no high contrast.
    pub const FALLBACK: Self = Self {
        caret_blink: Some(Duration::from_millis(530)),
        caret_width_px: 1,
        high_contrast: false,
    };
}

impl SystemMetrics {
    /// The live OS values (defaults where they cannot be read).
    pub fn read() -> Self {
        Self::from_platform(&open365_ui_core::PlatformSettings::read())
    }

    pub fn from_platform(p: &open365_ui_core::PlatformSettings) -> Self {
        Self {
            caret_blink: p.caret_blink,
            caret_width_px: p.caret_width,
            high_contrast: p.high_contrast,
        }
    }
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self::FALLBACK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winit_theme_maps_to_scheme() {
        assert_eq!(ColorScheme::from(Theme::Dark), ColorScheme::Dark);
        assert_eq!(ColorScheme::from(Theme::Light), ColorScheme::Light);
    }

    #[test]
    fn follow_system_leaves_the_window_theme_unset() {
        assert_eq!(TitleBarTheme::FollowSystem.to_winit(), None);
        assert_eq!(TitleBarTheme::Dark.to_winit(), Some(Theme::Dark));
        assert_eq!(TitleBarTheme::Light.to_winit(), Some(Theme::Light));
    }

    #[test]
    fn platform_settings_map_to_metrics() {
        let p = open365_ui_core::PlatformSettings {
            caret_blink: None,
            caret_width: 2,
            high_contrast: true,
            ..open365_ui_core::PlatformSettings::default()
        };
        let m = SystemMetrics::from_platform(&p);
        assert_eq!(
            (m.caret_blink, m.caret_width_px, m.high_contrast),
            (None, 2, true)
        );
        assert!(SystemMetrics::read().caret_width_px >= 1);
    }

    #[test]
    fn fallback_metrics_match_windows_defaults() {
        let m = SystemMetrics::default();
        assert_eq!(m.caret_blink, Some(Duration::from_millis(530)));
        assert_eq!(m.caret_width_px, 1);
        assert!(!m.high_contrast);
    }
}
