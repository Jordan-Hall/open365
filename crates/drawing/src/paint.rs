//! Colours, fills, lines, effects and style references.
//!
//! Colours keep their modifiers (`lumMod`, `tint`, ...) and resolve against a [`Theme`]
//! only when drawn, so a theme change re-colours the object without editing it.

/// 8-bit sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// 8-bit sRGB colour with alpha (255 = opaque).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Theme colour slots (`a:schemeClr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeColor {
    Dk1,
    Lt1,
    Dk2,
    Lt2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hlink,
    FolHlink,
    /// Placeholder colour inside a style matrix; replaced by the style reference colour.
    Phclr,
}

impl SchemeColor {
    /// Parses the `val` of `a:schemeClr`. `bg1`/`tx1`/`bg2`/`tx2` are mapped to the light and
    /// dark slots using the identity colour map; hosts with a custom map resolve them first.
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "dk1" | "tx1" => Self::Dk1,
            "lt1" | "bg1" => Self::Lt1,
            "dk2" | "tx2" => Self::Dk2,
            "lt2" | "bg2" => Self::Lt2,
            "accent1" => Self::Accent1,
            "accent2" => Self::Accent2,
            "accent3" => Self::Accent3,
            "accent4" => Self::Accent4,
            "accent5" => Self::Accent5,
            "accent6" => Self::Accent6,
            "hlink" => Self::Hlink,
            "folHlink" => Self::FolHlink,
            "phClr" => Self::Phclr,
            _ => return None,
        })
    }
}

/// Source of theme colours.
pub trait Theme {
    /// The colour of a slot. `Phclr` is never passed.
    fn scheme_color(&self, c: SchemeColor) -> Rgb;
}

/// A theme with fixed colours, for tests and for documents without a theme part.
#[derive(Debug, Clone, Copy)]
pub struct StaticTheme(pub [Rgb; 12]);

impl Theme for StaticTheme {
    fn scheme_color(&self, c: SchemeColor) -> Rgb {
        let i = match c {
            SchemeColor::Dk1 => 0,
            SchemeColor::Lt1 => 1,
            SchemeColor::Dk2 => 2,
            SchemeColor::Lt2 => 3,
            SchemeColor::Accent1 => 4,
            SchemeColor::Accent2 => 5,
            SchemeColor::Accent3 => 6,
            SchemeColor::Accent4 => 7,
            SchemeColor::Accent5 => 8,
            SchemeColor::Accent6 => 9,
            SchemeColor::Hlink => 10,
            SchemeColor::FolHlink | SchemeColor::Phclr => 11,
        };
        self.0[i]
    }
}

/// The base of a colour before modifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorBase {
    Srgb(Rgb),
    Scheme(SchemeColor),
    /// `a:sysClr`: `last` is the cached value written by Office.
    System {
        name: String,
        last: Option<Rgb>,
    },
    /// `a:prstClr`: a named colour; resolves to black until the name table is added.
    Preset(String),
}

/// A colour transform. Fractions are real numbers (`1.0` = 100 %).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorMod {
    Alpha(f64),
    AlphaMod(f64),
    AlphaOff(f64),
    LumMod(f64),
    LumOff(f64),
    SatMod(f64),
    SatOff(f64),
    Tint(f64),
    Shade(f64),
}

/// A colour with its ordered modifiers.
#[derive(Debug, Clone, PartialEq)]
pub struct Color {
    pub base: ColorBase,
    pub mods: Vec<ColorMod>,
}

impl Color {
    pub fn srgb(r: u8, g: u8, b: u8) -> Self {
        Self {
            base: ColorBase::Srgb(Rgb(r, g, b)),
            mods: Vec::new(),
        }
    }

    /// Resolves against `theme`; `ph` replaces a `phClr` base.
    pub fn resolve(&self, theme: &dyn Theme, ph: Option<Rgba>) -> Rgba {
        let (base, mut alpha) = match &self.base {
            ColorBase::Srgb(c) => (*c, 255u8),
            ColorBase::Scheme(SchemeColor::Phclr) => match ph {
                Some(p) => (Rgb(p.r, p.g, p.b), p.a),
                None => (Rgb(0, 0, 0), 255),
            },
            ColorBase::Scheme(s) => (theme.scheme_color(*s), 255),
            ColorBase::System { last, .. } => (last.unwrap_or(Rgb(0, 0, 0)), 255),
            ColorBase::Preset(_) => (Rgb(0, 0, 0), 255),
        };
        let mut a = f64::from(alpha) / 255.0;
        let (mut r, mut g, mut b) = (
            f64::from(base.0) / 255.0,
            f64::from(base.1) / 255.0,
            f64::from(base.2) / 255.0,
        );
        for m in &self.mods {
            match *m {
                ColorMod::Alpha(v) => a = v,
                ColorMod::AlphaMod(v) => a *= v,
                ColorMod::AlphaOff(v) => a += v,
                ColorMod::LumMod(v) | ColorMod::LumOff(v) => {
                    let (h, s, l) = rgb_to_hsl(r, g, b);
                    let l = if let ColorMod::LumMod(_) = m {
                        l * v
                    } else {
                        l + v
                    };
                    (r, g, b) = hsl_to_rgb(h, s, l.clamp(0.0, 1.0));
                }
                ColorMod::SatMod(v) | ColorMod::SatOff(v) => {
                    let (h, s, l) = rgb_to_hsl(r, g, b);
                    let s = if let ColorMod::SatMod(_) = m {
                        s * v
                    } else {
                        s + v
                    };
                    (r, g, b) = hsl_to_rgb(h, s.clamp(0.0, 1.0), l);
                }
                // Estimates: linear mix toward white (tint) or black (shade) in sRGB space.
                ColorMod::Tint(v) => {
                    r = r * v + (1.0 - v);
                    g = g * v + (1.0 - v);
                    b = b * v + (1.0 - v);
                }
                ColorMod::Shade(v) => {
                    r *= v;
                    g *= v;
                    b *= v;
                }
            }
        }
        alpha = to_u8(a);
        Rgba {
            r: to_u8(r),
            g: to_u8(g),
            b: to_u8(b),
            a: alpha,
        }
    }
}

fn to_u8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

/// One gradient stop; `pos` is `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct GradStop {
    pub pos: f64,
    pub color: Color,
}

/// Fill of a shape or line.
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    None,
    Solid(Color),
    Gradient {
        stops: Vec<GradStop>,
        /// `a:lin ang` in 60000ths of a degree.
        linear_angle: Option<i64>,
        /// `a:path path` (`circle`, `rect`, `shape`).
        path: Option<String>,
    },
    /// Picture fill: relationship id of the image part.
    Blip {
        embed: String,
    },
    Pattern {
        preset: String,
        fg: Option<Color>,
        bg: Option<Color>,
    },
    /// `a:grpFill`: take the group's fill.
    Group,
}

/// An arrowhead (`a:headEnd` / `a:tailEnd`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEnd {
    pub kind: String,
    pub width: Option<String>,
    pub length: Option<String>,
}

/// Outline (`a:ln`). Absent fields inherit from the style reference.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Line {
    /// Width in EMU.
    pub width: Option<i64>,
    pub fill: Option<Fill>,
    /// `prstDash` value.
    pub dash: Option<String>,
    pub cap: Option<String>,
    pub join: Option<String>,
    pub head_end: Option<LineEnd>,
    pub tail_end: Option<LineEnd>,
}

/// An effect (`a:outerShdw`, `a:glow`, ...), parameters kept as written.
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub kind: String,
    pub params: Vec<(String, String)>,
    pub color: Option<Color>,
}

/// One slot of `p:style`/`wps:style` (`lnRef`, `fillRef`, `effectRef`, `fontRef`).
#[derive(Debug, Clone, PartialEq)]
pub struct StyleSlot {
    pub idx: u32,
    pub color: Option<Color>,
}

/// The style references of a shape.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleRef {
    pub ln: Option<StyleSlot>,
    pub fill: Option<StyleSlot>,
    pub effect: Option<StyleSlot>,
    pub font: Option<StyleSlot>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> StaticTheme {
        StaticTheme([Rgb(10, 20, 30); 12])
    }

    #[test]
    fn srgb_roundtrips_and_alpha_applies() {
        let mut c = Color::srgb(255, 128, 0);
        assert_eq!(
            c.resolve(&theme(), None),
            Rgba {
                r: 255,
                g: 128,
                b: 0,
                a: 255
            }
        );
        c.mods.push(ColorMod::Alpha(0.5));
        assert_eq!(c.resolve(&theme(), None).a, 128);
    }

    #[test]
    fn lum_mod_halves_lightness() {
        let mut c = Color::srgb(255, 0, 0);
        c.mods.push(ColorMod::LumMod(0.5));
        let r = c.resolve(&theme(), None);
        assert_eq!((r.r, r.g, r.b), (128, 0, 0));
    }

    #[test]
    fn scheme_and_placeholder() {
        let c = Color {
            base: ColorBase::Scheme(SchemeColor::Accent1),
            mods: vec![],
        };
        assert_eq!(c.resolve(&theme(), None).r, 10);
        let ph = Color {
            base: ColorBase::Scheme(SchemeColor::Phclr),
            mods: vec![],
        };
        let p = Rgba {
            r: 1,
            g: 2,
            b: 3,
            a: 255,
        };
        assert_eq!(ph.resolve(&theme(), Some(p)), p);
    }

    #[test]
    fn shade_and_tint() {
        let mut c = Color::srgb(200, 100, 50);
        c.mods.push(ColorMod::Shade(0.5));
        assert_eq!(c.resolve(&theme(), None).r, 100);
        let mut t = Color::srgb(0, 0, 0);
        t.mods.push(ColorMod::Tint(0.5));
        assert_eq!(t.resolve(&theme(), None).r, 128);
    }
}
