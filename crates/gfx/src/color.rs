/// An sRGB colour with straight (non-premultiplied) alpha, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// The same colour with its alpha multiplied by `opacity` (0 to 1).
    pub fn with_opacity(self, opacity: f32) -> Self {
        let a = (f32::from(self.a) * opacity.clamp(0.0, 1.0)).round();
        Self { a: a as u8, ..self }
    }
}

/// sRGB electro-optical transfer function for one 8-bit channel.
pub fn srgb_to_linear(v: u8) -> f32 {
    let c = f32::from(v) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Inverse of [`srgb_to_linear`], rounded to the nearest 8-bit value.
pub fn linear_to_srgb(l: f32) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips_every_byte() {
        for v in 0..=255u8 {
            assert_eq!(linear_to_srgb(srgb_to_linear(v)), v);
        }
    }

    #[test]
    fn mid_grey_is_dark_in_linear_light() {
        let l = srgb_to_linear(128);
        assert!((l - 0.2159).abs() < 1e-3, "{l}");
    }

    #[test]
    fn opacity_scales_alpha_only() {
        let c = Color::rgb(10, 20, 30).with_opacity(0.5);
        assert_eq!(c, Color::rgba(10, 20, 30, 128));
    }
}
