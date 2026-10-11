//! Translation of winit window events into `open365-ui-core` [`InputEvent`]s.
//!
//! The bridge owns the little state the translation needs (pointer position, modifiers, scale
//! factor). Everything that touches a winit type that tests cannot construct (`KeyEvent`) is
//! reduced to plain values first, so the rest is unit-tested.
//!
//! Bare Alt: winit forwards `WM_SYSKEYDOWN` of Alt to `DefWindowProc` but swallows its
//! `WM_SYSKEYUP` for windows without a native menu (winit 0.30.13, `event_loop.rs`), so the system
//! menu mode that a bare Alt press normally enters never starts. The bridge reports the press and
//! release as [`Key::Alt`] events and drops auto-repeats, which is what the Alt key tip state
//! machine expects.

use open365_ui_core::geom::Point;
use open365_ui_core::{InputEvent, Key, Modifiers, PointerButton};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

/// Pixels of a precision-touchpad scroll that count as one wheel notch.
const PIXELS_PER_NOTCH: f32 = 40.0;

/// Keeps the state needed to turn window events into [`InputEvent`]s.
#[derive(Debug, Clone)]
pub struct InputBridge {
    scale: f64,
    pointer: Point,
    mods: Modifiers,
}

impl Default for InputBridge {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// Maps a winit named key to a widget key.
pub fn map_named(k: NamedKey) -> Option<Key> {
    Some(match k {
        NamedKey::Tab => Key::Tab,
        NamedKey::Enter => Key::Enter,
        NamedKey::Escape => Key::Escape,
        NamedKey::Space => Key::Space,
        NamedKey::Backspace => Key::Backspace,
        NamedKey::Delete => Key::Delete,
        NamedKey::ArrowLeft => Key::Left,
        NamedKey::ArrowRight => Key::Right,
        NamedKey::ArrowUp => Key::Up,
        NamedKey::ArrowDown => Key::Down,
        NamedKey::Home => Key::Home,
        NamedKey::End => Key::End,
        NamedKey::PageUp => Key::PageUp,
        NamedKey::PageDown => Key::PageDown,
        NamedKey::F2 => Key::F2,
        NamedKey::F10 => Key::F10,
        NamedKey::F12 => Key::F12,
        NamedKey::Alt => Key::Alt,
        NamedKey::Shift => Key::Shift,
        NamedKey::Control => Key::Ctrl,
        _ => return None,
    })
}

/// Maps a winit logical key to a widget key; characters are upper-cased.
pub fn map_key(k: &WKey) -> Option<Key> {
    match k {
        WKey::Named(n) => map_named(*n),
        WKey::Character(s) => {
            let mut it = s.chars();
            let c = it.next()?;
            if it.next().is_some() || c.is_control() {
                return None;
            }
            Some(Key::Char(c.to_uppercase().next().unwrap_or(c)))
        }
        _ => None,
    }
}

pub fn map_modifiers(m: ModifiersState) -> Modifiers {
    Modifiers {
        shift: m.shift_key(),
        ctrl: m.control_key(),
        alt: m.alt_key(),
    }
}

impl InputBridge {
    pub fn new(scale: f64) -> Self {
        Self {
            scale: scale.max(0.1),
            pointer: Point::default(),
            mods: Modifiers::default(),
        }
    }

    /// Device pixels per logical unit; pointer positions are divided by it.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale.max(0.1);
    }

    pub fn modifiers(&self) -> Modifiers {
        self.mods
    }

    pub fn pointer(&self) -> Point {
        self.pointer
    }

    fn logical(&self, x: f64, y: f64) -> Point {
        Point::new((x / self.scale) as f32, (y / self.scale) as f32)
    }

    /// Translates one key press or release given as plain values. `text` is what the key typed
    /// after the keyboard layout (`KeyEvent::text`).
    pub fn key_event(
        &mut self,
        key: Option<Key>,
        pressed: bool,
        repeat: bool,
        text: Option<&str>,
        out: &mut Vec<InputEvent>,
    ) {
        let m = self.mods;
        if let Some(key) = key {
            // An auto-repeating Alt would keep restarting the key tip state machine.
            if !(repeat && key == Key::Alt) {
                out.push(InputEvent::Key {
                    key,
                    pressed,
                    modifiers: m,
                });
            }
        }
        // Typed text: not shortcuts (Ctrl) and not Alt accelerators; AltGr arrives as Ctrl+Alt.
        let shortcut = (m.ctrl && !m.alt) || (m.alt && !m.ctrl);
        if pressed
            && !shortcut
            && let Some(t) = text.filter(|t| t.chars().any(|c| !c.is_control()))
        {
            out.push(InputEvent::Text(t.to_owned()));
        }
    }

    /// Translates `event`; the results are appended to `out`. Events with no widget meaning
    /// (resize, redraw, IME) add nothing.
    pub fn translate(&mut self, event: &WindowEvent, out: &mut Vec<InputEvent>) {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = self.logical(position.x, position.y);
                out.push(InputEvent::PointerMoved(self.pointer));
            }
            WindowEvent::CursorLeft { .. } => out.push(InputEvent::PointerLeft),
            WindowEvent::MouseInput { state, button, .. } => {
                let b = match button {
                    MouseButton::Left => PointerButton::Primary,
                    MouseButton::Right => PointerButton::Secondary,
                    _ => return,
                };
                out.push(match state {
                    ElementState::Pressed => InputEvent::PointerDown(b, self.pointer),
                    ElementState::Released => InputEvent::PointerUp(b, self.pointer),
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta_y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / PIXELS_PER_NOTCH,
                };
                if delta_y != 0.0 {
                    out.push(InputEvent::Wheel { delta_y });
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = map_modifiers(m.state()),
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                // `key_without_modifiers` keeps Ctrl+C a `C` instead of a control character.
                let key = match &event.logical_key {
                    WKey::Character(_) => map_key(&event.key_without_modifiers())
                        .or_else(|| map_key(&event.logical_key)),
                    other => map_key(other),
                };
                let text = event.text.as_ref().map(|t| t.as_str());
                self.key_event(key, pressed, event.repeat, text, out);
            }
            WindowEvent::Focused(f) => {
                if !f {
                    self.mods = Modifiers::default();
                }
                out.push(InputEvent::FocusChanged(*f));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;
    use winit::event::DeviceId;

    fn mv(b: &mut InputBridge, x: f64, y: f64) -> Vec<InputEvent> {
        let mut out = Vec::new();
        b.translate(
            &WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(x, y),
            },
            &mut out,
        );
        out
    }

    #[test]
    fn pointer_positions_are_logical() {
        let mut b = InputBridge::new(1.25);
        assert_eq!(
            mv(&mut b, 125.0, 250.0),
            [InputEvent::PointerMoved(Point::new(100.0, 200.0))]
        );
        let mut out = Vec::new();
        b.translate(
            &WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Pressed,
                button: MouseButton::Left,
            },
            &mut out,
        );
        b.translate(
            &WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Released,
                button: MouseButton::Left,
            },
            &mut out,
        );
        assert_eq!(
            out,
            [
                InputEvent::PointerDown(PointerButton::Primary, Point::new(100.0, 200.0)),
                InputEvent::PointerUp(PointerButton::Primary, Point::new(100.0, 200.0)),
            ]
        );
        b.set_scale(2.0);
        assert_eq!(
            mv(&mut b, 100.0, 40.0),
            [InputEvent::PointerMoved(Point::new(50.0, 20.0))]
        );
    }

    #[test]
    fn wheel_notches_and_pixels() {
        let mut b = InputBridge::default();
        let mut out = Vec::new();
        for delta in [
            MouseScrollDelta::LineDelta(0.0, -1.0),
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 20.0)),
            MouseScrollDelta::LineDelta(1.0, 0.0),
        ] {
            b.translate(
                &WindowEvent::MouseWheel {
                    device_id: DeviceId::dummy(),
                    delta,
                    phase: winit::event::TouchPhase::Moved,
                },
                &mut out,
            );
        }
        assert_eq!(
            out,
            [
                InputEvent::Wheel { delta_y: -1.0 },
                InputEvent::Wheel { delta_y: 0.5 }
            ]
        );
    }

    #[test]
    fn keys_map_by_name_and_letters_fold_to_upper_case() {
        assert_eq!(map_key(&WKey::Named(NamedKey::Escape)), Some(Key::Escape));
        assert_eq!(map_key(&WKey::Named(NamedKey::Alt)), Some(Key::Alt));
        assert_eq!(map_key(&WKey::Named(NamedKey::F10)), Some(Key::F10));
        assert_eq!(map_key(&WKey::Named(NamedKey::F5)), None);
        assert_eq!(map_key(&WKey::Character("h".into())), Some(Key::Char('H')));
        assert_eq!(map_key(&WKey::Character("ab".into())), None);
        assert_eq!(map_key(&WKey::Character("\u{3}".into())), None);
    }

    #[test]
    fn alt_press_and_release_drop_repeats_and_carry_modifiers() {
        let mut b = InputBridge {
            mods: Modifiers {
                alt: true,
                ..Modifiers::default()
            },
            ..InputBridge::default()
        };
        let mut out = Vec::new();
        b.key_event(Some(Key::Alt), true, false, None, &mut out);
        b.key_event(Some(Key::Alt), true, true, None, &mut out);
        b.key_event(Some(Key::Alt), true, true, None, &mut out);
        b.mods = Modifiers::default();
        b.key_event(Some(Key::Alt), false, false, None, &mut out);
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(
            out,
            [
                InputEvent::Key {
                    key: Key::Alt,
                    pressed: true,
                    modifiers: alt
                },
                InputEvent::Key {
                    key: Key::Alt,
                    pressed: false,
                    modifiers: Modifiers::default()
                },
            ]
        );
    }

    #[test]
    fn typed_text_skips_shortcuts_but_keeps_altgr() {
        let mut b = InputBridge::default();
        let mut out = Vec::new();
        b.key_event(Some(Key::Char('A')), true, false, Some("a"), &mut out);
        assert_eq!(out.last(), Some(&InputEvent::Text("a".into())));
        out.clear();
        b.mods.ctrl = true;
        b.key_event(Some(Key::Char('C')), true, false, Some("\u{3}"), &mut out);
        assert_eq!(out.len(), 1, "Ctrl+C is a key, not text");
        out.clear();
        b.mods = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        b.key_event(Some(Key::Char('H')), true, false, Some("h"), &mut out);
        assert_eq!(out.len(), 1, "Alt+H is an accelerator, not text");
        out.clear();
        b.mods = Modifiers {
            alt: true,
            ctrl: true,
            shift: false,
        };
        b.key_event(Some(Key::Char('Q')), true, false, Some("@"), &mut out);
        assert_eq!(out.last(), Some(&InputEvent::Text("@".into())), "AltGr");
        out.clear();
        b.mods = Modifiers::default();
        b.key_event(Some(Key::Char('A')), false, false, Some("a"), &mut out);
        assert_eq!(out.len(), 1, "no text on release");
    }

    #[test]
    fn losing_focus_clears_modifiers() {
        let mut b = InputBridge::default();
        b.mods.alt = true;
        let mut out = Vec::new();
        b.translate(&WindowEvent::Focused(false), &mut out);
        assert_eq!(out, [InputEvent::FocusChanged(false)]);
        assert_eq!(b.modifiers(), Modifiers::default());
    }
}
