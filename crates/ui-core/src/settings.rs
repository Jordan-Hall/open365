//! OS settings read through the safe `windows-registry` crate (read only, never written):
//! caret blink rate and width, high contrast, the app light/dark mode and the high contrast
//! colours. Other platforms report the Windows defaults.

use std::time::Duration;

/// A colour as the registry stores it (`"r g b"` decimal).
pub type Rgb = [u8; 3];

/// What the OS currently says about text carets, contrast and theme.
#[derive(Debug, Clone, PartialEq)]
pub struct PlatformSettings {
    /// Caret on/off interval; `None` keeps the caret steady.
    pub caret_blink: Option<Duration>,
    /// Caret width in device pixels at 100 %.
    pub caret_width: u32,
    pub high_contrast: bool,
    /// The app mode is dark (`AppsUseLightTheme` = 0).
    pub apps_dark: bool,
    /// System colours of the active high contrast theme: window, window text, highlight,
    /// highlight text, gray text, button face.
    pub hc_colors: Option<HighContrastColors>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighContrastColors {
    pub window: Rgb,
    pub window_text: Rgb,
    pub highlight: Rgb,
    pub highlight_text: Rgb,
    pub gray_text: Rgb,
    pub button_face: Rgb,
}

impl Default for PlatformSettings {
    fn default() -> Self {
        Self {
            caret_blink: Some(Duration::from_millis(530)),
            caret_width: 1,
            high_contrast: false,
            apps_dark: false,
            hc_colors: None,
        }
    }
}

/// `CursorBlinkRate`: milliseconds as text; `-1` or the maximum value means a steady caret.
pub fn parse_blink(v: &str) -> Option<Option<Duration>> {
    let n: i64 = v.trim().parse().ok()?;
    match n {
        n if n < 0 || n == i64::from(u32::MAX) => Some(None),
        0 => None,
        n => Some(Some(Duration::from_millis(n as u64))),
    }
}

/// `HighContrast\Flags`: bit 0 (`HCF_HIGHCONTRASTON`) is set while the theme is on.
pub fn parse_hc_flags(v: &str) -> bool {
    v.trim().parse::<u32>().is_ok_and(|f| f & 1 != 0)
}

/// `"255 128 0"` to `[255, 128, 0]`.
pub fn parse_rgb(v: &str) -> Option<Rgb> {
    let mut it = v.split_whitespace().map(|t| t.parse::<u8>());
    let c = [it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?];
    it.next().is_none().then_some(c)
}

impl PlatformSettings {
    /// Builds settings from raw registry text values (`None` when absent).
    #[allow(clippy::too_many_arguments)]
    pub fn from_values(
        blink: Option<&str>,
        caret_width: Option<u32>,
        hc_flags: Option<&str>,
        apps_light: Option<u32>,
        colors: &dyn Fn(&str) -> Option<String>,
    ) -> Self {
        let mut s = Self::default();
        if let Some(b) = blink.and_then(parse_blink) {
            s.caret_blink = b;
        }
        if let Some(w) = caret_width.filter(|w| (1..=32).contains(w)) {
            s.caret_width = w;
        }
        s.high_contrast = hc_flags.is_some_and(parse_hc_flags);
        s.apps_dark = apps_light == Some(0);
        if s.high_contrast {
            let get = |n: &str| colors(n).as_deref().and_then(parse_rgb);
            if let (Some(window), Some(window_text), Some(highlight), Some(highlight_text)) = (
                get("Window"),
                get("WindowText"),
                get("Hilight"),
                get("HilightText"),
            ) {
                s.hc_colors = Some(HighContrastColors {
                    window,
                    window_text,
                    highlight,
                    highlight_text,
                    gray_text: get("GrayText").unwrap_or(window_text),
                    button_face: get("ButtonFace").unwrap_or(window),
                });
            }
        }
        s
    }

    /// Reads the live settings (Windows registry); defaults elsewhere.
    pub fn read() -> Self {
        read_impl()
    }
}

/// The regional settings of the signed-in person (`HKCU\Control Panel\International`), read
/// only: the language tag and the short date and time patterns they may have customised.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsLocale {
    /// `en-GB`.
    pub name: Option<String>,
    /// `dd/MM/yyyy`.
    pub short_date: Option<String>,
    /// `HH:mm:ss`.
    pub time: Option<String>,
}

/// Reads the regional settings; empty off Windows.
pub fn os_locale() -> OsLocale {
    os_locale_impl()
}

#[cfg(windows)]
fn os_locale_impl() -> OsLocale {
    use windows_registry::CURRENT_USER;
    let Ok(key) = CURRENT_USER.open(r"Control Panel\International") else {
        return OsLocale::default();
    };
    let get = |n: &str| key.get_string(n).ok().filter(|v| !v.is_empty());
    OsLocale {
        name: get("LocaleName"),
        short_date: get("sShortDate"),
        time: get("sTimeFormat"),
    }
}

#[cfg(not(windows))]
fn os_locale_impl() -> OsLocale {
    let tag = ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .and_then(|v| {
            let base = v.split(['.', '@']).next()?.replace('_', "-");
            (base.len() >= 2 && base != "C" && base != "POSIX").then_some(base)
        });
    OsLocale {
        name: tag,
        ..OsLocale::default()
    }
}

/// The local time zone's offset from UTC in minutes (east positive) right now, daylight saving
/// included. Windows reports `ActiveTimeBias` (UTC minus local, a signed DWORD) in the registry,
/// read through the same safe crate as the other settings. Elsewhere it is 0 (UTC) unless
/// `OPEN365_UTC_OFFSET_MINUTES` names an offset.
pub fn utc_offset_minutes() -> i32 {
    offset_impl()
}

/// `ActiveTimeBias` (minutes, UTC = local + bias, stored as an unsigned DWORD) as an offset.
pub fn offset_from_bias(bias: u32) -> i32 {
    -(bias as i32)
}

#[cfg(windows)]
fn offset_impl() -> i32 {
    use windows_registry::LOCAL_MACHINE;
    LOCAL_MACHINE
        .open(r"SYSTEM\CurrentControlSet\Control\TimeZoneInformation")
        .ok()
        .and_then(|k| k.get_u32("ActiveTimeBias").ok())
        .map_or(0, offset_from_bias)
}

#[cfg(not(windows))]
fn offset_impl() -> i32 {
    std::env::var("OPEN365_UTC_OFFSET_MINUTES")
        .ok()
        .and_then(|v| v.trim().parse::<i32>().ok())
        .filter(|m| m.abs() <= 14 * 60)
        .unwrap_or(0)
}

#[cfg(windows)]
fn read_impl() -> PlatformSettings {
    use windows_registry::CURRENT_USER;
    let text = |path: &str, name: &str| -> Option<String> {
        let key = CURRENT_USER.open(path).ok()?;
        key.get_string(name)
            .ok()
            .or_else(|| key.get_u32(name).ok().map(|n| n.to_string()))
    };
    let blink = text(r"Control Panel\Desktop", "CursorBlinkRate");
    let width = CURRENT_USER
        .open(r"Control Panel\Desktop")
        .ok()
        .and_then(|k| k.get_u32("CaretWidth").ok());
    let flags = text(r"Control Panel\Accessibility\HighContrast", "Flags");
    let apps_light = CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .ok()
        .and_then(|k| k.get_u32("AppsUseLightTheme").ok());
    PlatformSettings::from_values(
        blink.as_deref(),
        width,
        flags.as_deref(),
        apps_light,
        &|n| text(r"Control Panel\Colors", n),
    )
}

#[cfg(not(windows))]
fn read_impl() -> PlatformSettings {
    PlatformSettings::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_rate_parses_like_windows() {
        assert_eq!(parse_blink("530"), Some(Some(Duration::from_millis(530))));
        assert_eq!(parse_blink("-1"), Some(None));
        assert_eq!(parse_blink("4294967295"), Some(None));
        assert_eq!(parse_blink("abc"), None);
    }

    #[test]
    fn high_contrast_flag_is_bit_zero() {
        assert!(!parse_hc_flags("126"), "126 has bit 0 clear");
        assert!(parse_hc_flags("127"));
    }

    #[test]
    fn registry_text_builds_settings() {
        let none = |_: &str| None;
        let s = PlatformSettings::from_values(Some("400"), Some(2), Some("127"), Some(0), &none);
        assert_eq!(s.caret_blink, Some(Duration::from_millis(400)));
        assert_eq!(s.caret_width, 2);
        assert!(s.high_contrast && s.apps_dark);
        assert!(s.hc_colors.is_none(), "no colours in the registry");
        let colors = |n: &str| match n {
            "Window" => Some("0 0 0".to_string()),
            "WindowText" => Some("255 255 255".to_string()),
            "Hilight" => Some("26 235 255".to_string()),
            "HilightText" => Some("0 0 0".to_string()),
            _ => None,
        };
        let s = PlatformSettings::from_values(None, None, Some("127"), Some(1), &colors);
        assert_eq!(s.hc_colors.expect("colours").highlight, [26, 235, 255]);
        assert!(!s.apps_dark);
    }

    #[test]
    fn out_of_range_values_keep_defaults() {
        let none = |_: &str| None;
        let s = PlatformSettings::from_values(Some("x"), Some(99), None, None, &none);
        assert_eq!(s, PlatformSettings::default());
    }

    #[test]
    fn rgb_parses_three_bytes_only() {
        assert_eq!(parse_rgb("1 2 3"), Some([1, 2, 3]));
        assert_eq!(parse_rgb("1 2"), None);
        assert_eq!(parse_rgb("1 2 3 4"), None);
        assert_eq!(parse_rgb("256 0 0"), None);
    }

    #[test]
    fn registry_bias_is_the_negated_offset() {
        // UTC+1 stores a bias of -60 (as an unsigned DWORD); UTC-5 stores 300.
        assert_eq!(offset_from_bias((-60i32) as u32), 60);
        assert_eq!(offset_from_bias(300), -300);
        assert_eq!(offset_from_bias(0), 0);
        assert!(utc_offset_minutes().abs() <= 14 * 60);
    }

    #[test]
    fn live_read_never_panics() {
        let s = PlatformSettings::read();
        assert!(s.caret_width >= 1);
    }
}
