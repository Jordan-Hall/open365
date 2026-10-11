//! The text caret as Windows configures it: blink interval and width (read only, never changed).
//!
//! `GetCaretBlinkTime` and `GetCaretWidth` return the values of `HKCU\Control Panel\Desktop`
//! (`CursorBlinkRate`, `CaretWidth`); they are read here through the safe `windows-registry` crate (read only),
//! because the workspace forbids `unsafe`. Defaults: 530 ms and 1 px at 96 dpi.

use std::sync::OnceLock;

/// Caret appearance shared by the document caret and the text editors (cell editor, formula bar).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretSettings {
    /// Milliseconds the caret is shown, then hidden. `None`: Windows reports INFINITE, the caret does not blink.
    pub blink_ms: Option<u32>,
    /// Width in points (device-independent pixels): 1 px at 96 dpi, scaled with the display.
    pub width: f32,
}

impl Default for CaretSettings {
    fn default() -> Self {
        Self {
            blink_ms: Some(530),
            width: 1.0,
        }
    }
}

impl CaretSettings {
    /// Parses the registry values as text (`"530"`, `"2"`; a trailing token of a `reg query` line also works).
    /// A missing or unreadable value keeps the default; a negative blink rate means "never blink".
    #[must_use]
    pub fn parse(blink: Option<&str>, width: Option<&str>) -> Self {
        let mut out = Self::default();
        if let Some(v) = blink.and_then(last_token) {
            match v.parse::<i64>() {
                Ok(n) if n < 0 || n == i64::from(u32::MAX) => out.blink_ms = None,
                Ok(n) if n > 0 => out.blink_ms = u32::try_from(n).ok(),
                _ => {}
            }
        }
        if let Some(v) = width.and_then(last_token) {
            let n = match v.strip_prefix("0x") {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => v.parse::<u32>().ok(),
            };
            if let Some(n) = n.filter(|n| (1..=32).contains(n)) {
                out.width = n as f32;
            }
        }
        out
    }

    /// The system's settings, read once.
    #[must_use]
    pub fn system() -> Self {
        static CELL: OnceLock<CaretSettings> = OnceLock::new();
        *CELL.get_or_init(|| {
            Self::parse(
                query("CursorBlinkRate").as_deref(),
                query("CaretWidth").as_deref(),
            )
        })
    }

    /// Seconds of the visible (and of the hidden) half of the blink cycle; `None` when the caret does not blink.
    #[must_use]
    pub fn blink_secs(&self) -> Option<f64> {
        self.blink_ms.map(|ms| f64::from(ms) / 1000.0)
    }
}

fn last_token(text: &str) -> Option<&str> {
    text.split_whitespace().last()
}

/// Reads one value of `HKCU\Control Panel\Desktop` as text: `REG_SZ` as is, `REG_DWORD` in decimal.
#[cfg(windows)]
fn query(name: &str) -> Option<String> {
    let key = windows_registry::CURRENT_USER
        .open(r"Control Panel\Desktop")
        .ok()?;
    key.get_string(name)
        .ok()
        .or_else(|| key.get_u32(name).ok().map(|n| n.to_string()))
}

#[cfg(not(windows))]
fn query(_name: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_windows_defaults() {
        let d = CaretSettings::parse(None, None);
        assert_eq!(d.blink_ms, Some(530));
        assert!((d.width - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn registry_values_are_read() {
        let s = CaretSettings::parse(Some("400"), Some("2"));
        assert_eq!(s.blink_ms, Some(400));
        assert!((s.width - 2.0).abs() < f32::EPSILON);
    }

    /// The cell editor and formula bar are egui text edits: their caret comes from the theme's visuals.
    #[test]
    fn text_editors_follow_the_system_caret() {
        use crate::{AppKind, Mode, Theme};
        let v = Theme::new(AppKind::Sheets, Mode::Dark).visuals();
        let c = CaretSettings::system();
        assert_eq!(v.text_cursor.blink, c.blink_ms.is_some());
        assert!((v.text_cursor.stroke.width - c.width).abs() < f32::EPSILON);
        if let Some(s) = c.blink_secs() {
            assert!((f64::from(v.text_cursor.on_duration) - s).abs() < 1e-6);
        }
    }

    #[test]
    fn infinite_blink_time_means_no_blink() {
        assert_eq!(CaretSettings::parse(Some("-1"), None).blink_ms, None);
        assert_eq!(
            CaretSettings::parse(Some("4294967295"), None).blink_ms,
            None
        );
    }
}
