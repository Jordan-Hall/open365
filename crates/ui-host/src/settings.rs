//! Re-reading OS settings while the application runs.
//!
//! winit 0.30 exposes no `WM_SETTINGCHANGE` hook (and installing one would need `unsafe`), so
//! the host polls instead: the registry is read when the window gains focus and when winit
//! reports a theme change (winit handles `WM_SETTINGCHANGE` itself and forwards only that).
//! Caret blink and width, high contrast and the app colour mode change in Windows settings,
//! which means the user has left the application, so a focus read catches every change by the
//! time they come back.

use open365_ui_core::PlatformSettings;

use crate::theme::ColorScheme;

/// Remembers the last settings and reports when a read differs.
#[derive(Debug, Clone)]
pub struct SettingsWatcher {
    last: PlatformSettings,
}

impl SettingsWatcher {
    pub fn new(initial: PlatformSettings) -> Self {
        Self { last: initial }
    }

    pub fn current(&self) -> &PlatformSettings {
        &self.last
    }

    /// Takes `fresh` (a new read) and returns it when it differs from the previous one.
    pub fn update(&mut self, fresh: PlatformSettings) -> Option<PlatformSettings> {
        if fresh == self.last {
            None
        } else {
            self.last = fresh.clone();
            Some(fresh)
        }
    }

    /// Reads the registry and applies [`SettingsWatcher::update`].
    pub fn poll(&mut self, scheme: ColorScheme) -> Option<PlatformSettings> {
        self.update(read_with_scheme(scheme))
    }
}

/// The live settings with the app mode taken from winit's theme (which the window already
/// follows) rather than from the registry value, so a title bar and the chrome cannot disagree.
pub fn read_with_scheme(scheme: ColorScheme) -> PlatformSettings {
    with_scheme(PlatformSettings::read(), scheme)
}

pub fn with_scheme(mut s: PlatformSettings, scheme: ColorScheme) -> PlatformSettings {
    s.apps_dark = scheme == ColorScheme::Dark;
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn unchanged_reads_report_nothing_and_changes_report_once() {
        let base = PlatformSettings::default();
        let mut w = SettingsWatcher::new(base.clone());
        assert!(w.update(base.clone()).is_none());
        let hc = PlatformSettings {
            high_contrast: true,
            ..base.clone()
        };
        assert_eq!(w.update(hc.clone()), Some(hc.clone()));
        assert!(w.update(hc).is_none(), "same read again");
        let slow = PlatformSettings {
            caret_blink: Some(Duration::from_millis(1200)),
            caret_width: 3,
            ..base
        };
        assert_eq!(w.update(slow.clone()), Some(slow));
        assert_eq!(w.current().caret_width, 3);
    }

    #[test]
    fn the_scheme_overrides_the_registry_app_mode() {
        let s = with_scheme(PlatformSettings::default(), ColorScheme::Dark);
        assert!(s.apps_dark);
        assert!(!with_scheme(s, ColorScheme::Light).apps_dark);
    }
}
