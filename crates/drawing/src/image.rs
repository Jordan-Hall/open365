//! Raster and vector picture headers: format, pixel size and resolution, the natural size Excel gives
//! an inserted picture, and the PNG fallback of an SVG.
//!
//! The natural size was measured in Excel (`tools/office/drawing-ops-oracle.ps1`, references in
//! `fixtures/reference/xlsx/drawing-insert/insert.json`): a picture is `pixels / resolution` inches.
//! PNG reads the `pHYs` chunk (pixels per metre), JPEG the JFIF density (dots per inch or per
//! centimetre), BMP the header's pixels per metre; GIF, SVG and any file without a resolution are
//! 96 dpi. The extent in EMU is `round(pixels * 36 000 000 / pixels_per_metre)`, which reproduces
//! Excel's value to the EMU for pictures up to a few hundred pixels (`.NET` writes 3779 pixels per metre for 96 dpi, giving 914528 EMU for 96
//! pixels; a 300 dpi JPEG of 600 pixels is 1 828 800 EMU).

use std::fmt;

use resvg::{tiny_skia, usvg};

/// Pixels per metre of a 96 dpi picture, the default when a file has no resolution.
const PPM_96: f64 = 96.0 / 0.0254;
/// Largest picture side accepted (pixels), a guard against decompression bombs.
const MAX_SIDE: u32 = 65_535;

/// The picture formats Insert > Picture takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Svg,
}

impl ImageFormat {
    /// File extension used for the package part (`png`, `jpeg`, `gif`, `bmp`, `svg`), as Excel names them.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Svg => "svg",
        }
    }

    /// Media type of the part.
    #[must_use]
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
            Self::Svg => "image/svg+xml",
        }
    }
}

/// Why a picture could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// Not a PNG, JPEG, GIF, BMP or SVG file.
    UnknownFormat,
    /// The header is cut short or inconsistent.
    Truncated(&'static str),
    /// A side is zero or larger than the limit.
    BadSize,
    /// The SVG did not parse or has no size.
    Svg(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat => {
                f.write_str("The file is not a picture format that can be inserted.")
            }
            Self::Truncated(what) => write!(f, "The picture file is damaged ({what})."),
            Self::BadSize => f.write_str("The picture has no usable size."),
            Self::Svg(m) => write!(f, "The SVG could not be read ({m})."),
        }
    }
}

impl std::error::Error for ImageError {}

/// What the header of a picture file says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageInfo {
    pub format: ImageFormat,
    /// Pixels (for an SVG, CSS pixels at 96 dpi, rounded up).
    pub width: u32,
    pub height: u32,
    /// Horizontal and vertical pixels per metre; 96 dpi when the file does not say.
    pub ppm_x: f64,
    pub ppm_y: f64,
    /// Exact size of an SVG in CSS pixels.
    pub svg_size: Option<(f64, f64)>,
}

impl ImageInfo {
    /// The extent Excel gives the picture when inserted, in EMU `(cx, cy)`.
    #[must_use]
    pub fn natural_emu(&self) -> (i64, i64) {
        if let Some((w, h)) = self.svg_size {
            return ((w * 9_525.0).round() as i64, (h * 9_525.0).round() as i64);
        }
        // Exact in double precision. Excel's own values differ by at most 1 EMU on a 3000 pixel side (its
        // arithmetic is single precision somewhere), which no layout can show.
        let side = |px: u32, ppm: f64| (f64::from(px) * 36_000_000.0 / ppm).round() as i64;
        (side(self.width, self.ppm_x), side(self.height, self.ppm_y))
    }

    /// The extent in points.
    #[must_use]
    pub fn natural_pt(&self) -> (f64, f64) {
        let (cx, cy) = self.natural_emu();
        (cx as f64 / 12_700.0, cy as f64 / 12_700.0)
    }
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn checked(w: u32, h: u32) -> Result<(u32, u32), ImageError> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        Err(ImageError::BadSize)
    } else {
        Ok((w, h))
    }
}

fn png(b: &[u8]) -> Result<ImageInfo, ImageError> {
    const T: ImageError = ImageError::Truncated("PNG header");
    if b.get(12..16) != Some(b"IHDR") {
        return Err(T);
    }
    let (w, h) = checked(be32(b, 16).ok_or(T)?, be32(b, 20).ok_or(T)?)?;
    let (mut ppm_x, mut ppm_y) = (PPM_96, PPM_96);
    // Walk the chunks up to the first IDAT; pHYs must precede it.
    let mut at = 8;
    while let (Some(len), Some(kind)) = (be32(b, at), b.get(at + 4..at + 8)) {
        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        if kind == b"pHYs"
            && len >= 9
            && let (Some(x), Some(y), Some(unit)) =
                (be32(b, at + 8), be32(b, at + 12), b.get(at + 16))
            && *unit == 1
            && x > 0
            && y > 0
        {
            ppm_x = f64::from(x);
            ppm_y = f64::from(y);
        }
        at = at.saturating_add(len as usize).saturating_add(12);
    }
    Ok(ImageInfo {
        format: ImageFormat::Png,
        width: w,
        height: h,
        ppm_x,
        ppm_y,
        svg_size: None,
    })
}

fn jpeg(b: &[u8]) -> Result<ImageInfo, ImageError> {
    const T: ImageError = ImageError::Truncated("JPEG markers");
    let (mut ppm_x, mut ppm_y) = (PPM_96, PPM_96);
    let mut at = 2;
    loop {
        // Skip fill bytes up to the next marker.
        while b.get(at) == Some(&0xff) && b.get(at + 1) == Some(&0xff) {
            at += 1;
        }
        if b.get(at) != Some(&0xff) {
            return Err(T);
        }
        let marker = *b.get(at + 1).ok_or(T)?;
        let len = usize::from(be16(b, at + 2).ok_or(T)?);
        let body = at + 4;
        match marker {
            // APP0 JFIF: units, Xdensity, Ydensity.
            0xe0 if b.get(body..body + 5) == Some(b"JFIF\0") => {
                if let (Some(unit), Some(x), Some(y)) =
                    (b.get(body + 7), be16(b, body + 8), be16(b, body + 10))
                    && x > 0
                    && y > 0
                {
                    match unit {
                        1 => {
                            ppm_x = f64::from(x) / 0.0254;
                            ppm_y = f64::from(y) / 0.0254;
                        }
                        2 => {
                            ppm_x = f64::from(x) * 100.0;
                            ppm_y = f64::from(y) * 100.0;
                        }
                        _ => {}
                    }
                }
            }
            // Start of frame (baseline, extended, progressive, lossless; not DHT 0xc4, JPG 0xc8, DAC 0xcc).
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                let h = u32::from(be16(b, body + 1).ok_or(T)?);
                let w = u32::from(be16(b, body + 3).ok_or(T)?);
                let (w, h) = checked(w, h)?;
                return Ok(ImageInfo {
                    format: ImageFormat::Jpeg,
                    width: w,
                    height: h,
                    ppm_x,
                    ppm_y,
                    svg_size: None,
                });
            }
            0xd9 | 0xda => return Err(T),
            _ => {}
        }
        at = at.checked_add(2 + len).ok_or(T)?;
    }
}

fn gif(b: &[u8]) -> Result<ImageInfo, ImageError> {
    const T: ImageError = ImageError::Truncated("GIF header");
    let (w, h) = checked(
        u32::from(le16(b, 6).ok_or(T)?),
        u32::from(le16(b, 8).ok_or(T)?),
    )?;
    Ok(ImageInfo {
        format: ImageFormat::Gif,
        width: w,
        height: h,
        ppm_x: PPM_96,
        ppm_y: PPM_96,
        svg_size: None,
    })
}

fn bmp(b: &[u8]) -> Result<ImageInfo, ImageError> {
    const T: ImageError = ImageError::Truncated("BMP header");
    let header = le32(b, 14).ok_or(T)?;
    let (w, h, px, py) = if header == 12 {
        // OS/2 core header: 16-bit sizes, no resolution.
        (
            u32::from(le16(b, 18).ok_or(T)?),
            u32::from(le16(b, 20).ok_or(T)?),
            0,
            0,
        )
    } else {
        let w = le32(b, 18).ok_or(T)? as i32;
        let h = le32(b, 22).ok_or(T)? as i32;
        (
            w.unsigned_abs(),
            h.unsigned_abs(),
            le32(b, 38).ok_or(T)? as i32,
            le32(b, 42).ok_or(T)? as i32,
        )
    };
    let (w, h) = checked(w, h)?;
    let ppm = |v: i32| if v > 0 { f64::from(v) } else { PPM_96 };
    Ok(ImageInfo {
        format: ImageFormat::Bmp,
        width: w,
        height: h,
        ppm_x: ppm(px),
        ppm_y: ppm(py),
        svg_size: None,
    })
}

fn svg_tree(b: &[u8]) -> Result<usvg::Tree, ImageError> {
    usvg::Tree::from_data(b, &usvg::Options::default()).map_err(|e| ImageError::Svg(e.to_string()))
}

fn svg(b: &[u8]) -> Result<ImageInfo, ImageError> {
    let tree = svg_tree(b)?;
    let (w, h) = (
        f64::from(tree.size().width()),
        f64::from(tree.size().height()),
    );
    if !(w >= 1.0 && h >= 1.0 && w <= f64::from(MAX_SIDE) && h <= f64::from(MAX_SIDE)) {
        return Err(ImageError::BadSize);
    }
    Ok(ImageInfo {
        format: ImageFormat::Svg,
        width: w.ceil() as u32,
        height: h.ceil() as u32,
        ppm_x: PPM_96,
        ppm_y: PPM_96,
        svg_size: Some((w, h)),
    })
}

/// True when the bytes look like an SVG document (an `<svg` element after optional markup).
fn looks_like_svg(b: &[u8]) -> bool {
    let head = &b[..b.len().min(4096)];
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    t.starts_with("<svg") || (t.starts_with('<') && t.contains("<svg"))
}

/// Reads the header of a picture file.
///
/// # Errors
/// The bytes are not a supported picture, or the header is damaged.
pub fn sniff(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg(bytes)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        gif(bytes)
    } else if bytes.starts_with(b"BM") {
        bmp(bytes)
    } else if looks_like_svg(bytes) {
        svg(bytes)
    } else {
        Err(ImageError::UnknownFormat)
    }
}

/// Renders an SVG to a PNG whose longer side is `max_side` pixels (at most 4096): small icons are
/// enlarged and huge drawings reduced, so the bitmap is always a sensible size. Excel stores such a PNG
/// next to the SVG so viewers without SVG support can draw the picture; its pixel size is not
/// specified by the format, so this crate picks `max_side`.
///
/// # Errors
/// The SVG does not parse, or the bitmap cannot be made or encoded.
pub fn svg_to_png(svg: &[u8], max_side: u32) -> Result<Vec<u8>, ImageError> {
    let tree = svg_tree(svg)?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if !(w > 0.0 && h > 0.0) {
        return Err(ImageError::BadSize);
    }
    let scale = max_side.clamp(1, 4096) as f32 / w.max(h);
    let (pw, ph) = (
        (w * scale).ceil().max(1.0) as u32,
        (h * scale).ceil().max(1.0) as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or(ImageError::BadSize)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|e| ImageError::Svg(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32, ppm: Option<u32>) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        if let Some(p) = ppm {
            v.extend_from_slice(&9u32.to_be_bytes());
            v.extend_from_slice(b"pHYs");
            v.extend_from_slice(&p.to_be_bytes());
            v.extend_from_slice(&p.to_be_bytes());
            v.push(1);
            v.extend_from_slice(&[0; 4]);
        }
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(b"IDAT");
        v.extend_from_slice(&[0; 4]);
        v
    }

    #[test]
    fn png_natural_size_matches_excel_to_the_emu() {
        // Values read back from Excel for System.Drawing PNGs (insert.json).
        let i = sniff(&png_bytes(96, 64, Some(3779))).unwrap();
        assert_eq!(i.natural_emu(), (914_528, 609_685));
        let i = sniff(&png_bytes(96, 64, Some(7559))).unwrap();
        assert_eq!(i.natural_emu(), (457_203, 304_802));
        // Excel reads 28 578 988 here: one EMU of single-precision noise.
        let i = sniff(&png_bytes(3000, 2000, Some(3779))).unwrap();
        let (cx, cy) = i.natural_emu();
        assert!((cx - 28_578_988).abs() <= 2 && (cy - 19_052_659).abs() <= 2);
    }

    #[test]
    fn png_without_resolution_is_96_dpi() {
        let i = sniff(&png_bytes(96, 64, None)).unwrap();
        assert_eq!(i.natural_emu(), (914_400, 609_600));
        assert_eq!(i.natural_pt(), (72.0, 48.0));
    }

    #[test]
    fn jpeg_density_sets_the_size() {
        let jpeg = |unit: u8, d: u16| {
            let mut v = vec![0xff, 0xd8, 0xff, 0xe0, 0, 16];
            v.extend_from_slice(b"JFIF\0");
            v.extend_from_slice(&[1, 1, unit]);
            v.extend_from_slice(&d.to_be_bytes());
            v.extend_from_slice(&d.to_be_bytes());
            v.extend_from_slice(&[0, 0]);
            v.extend_from_slice(&[0xff, 0xc0, 0, 11, 8]);
            v.extend_from_slice(&400u16.to_be_bytes());
            v.extend_from_slice(&600u16.to_be_bytes());
            v.extend_from_slice(&[1, 1, 0x11, 0]);
            v
        };
        assert_eq!(
            sniff(&jpeg(1, 300)).unwrap().natural_emu(),
            (1_828_800, 1_219_200)
        );
        assert_eq!(
            sniff(&jpeg(1, 72)).unwrap().natural_emu(),
            (7_620_000, 5_080_000)
        );
        // Dots per centimetre: 118 dpcm is 299.72 dpi.
        let i = sniff(&jpeg(2, 118)).unwrap();
        assert_eq!(
            i.natural_emu().0,
            (600.0_f64 * 36_000_000.0 / 11_800.0).round() as i64
        );
        // Aspect-only density (unit 0) means 96 dpi.
        assert_eq!(
            sniff(&jpeg(0, 1)).unwrap().natural_emu(),
            (5_715_000, 3_810_000)
        );
    }

    #[test]
    fn gif_and_bmp_headers() {
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&96u16.to_le_bytes());
        g.extend_from_slice(&64u16.to_le_bytes());
        assert_eq!(sniff(&g).unwrap().natural_emu(), (914_400, 609_600));
        let mut b = vec![0u8; 54];
        b[0] = b'B';
        b[1] = b'M';
        b[14..18].copy_from_slice(&40u32.to_le_bytes());
        b[18..22].copy_from_slice(&96i32.to_le_bytes());
        b[22..26].copy_from_slice(&(-64i32).to_le_bytes());
        b[38..42].copy_from_slice(&3780i32.to_le_bytes());
        b[42..46].copy_from_slice(&3780i32.to_le_bytes());
        let i = sniff(&b).unwrap();
        assert_eq!((i.width, i.height), (96, 64));
        assert_eq!(i.natural_emu(), (914_286, 609_524));
    }

    #[test]
    fn svg_size_is_css_pixels_and_has_a_png_fallback() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="32" viewBox="0 0 48 32"><rect width="48" height="32" fill="#2a6"/></svg>"##;
        let i = sniff(svg).unwrap();
        assert_eq!(i.format, ImageFormat::Svg);
        assert_eq!(i.natural_emu(), (457_200, 304_800));
        let png = svg_to_png(svg, 192).unwrap();
        let p = sniff(&png).unwrap();
        assert_eq!(p.format, ImageFormat::Png);
        assert_eq!((p.width, p.height), (192, 128));
    }

    #[test]
    fn a_huge_svg_gets_a_small_fallback() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20000" height="10000"><rect width="20000" height="10000" fill="#000"/></svg>"##;
        let png = svg_to_png(svg, 512).unwrap();
        let p = sniff(&png).unwrap();
        assert_eq!((p.width, p.height), (512, 256));
    }

    #[test]
    fn rejects_unknown_and_damaged_files() {
        assert_eq!(sniff(b"hello"), Err(ImageError::UnknownFormat));
        assert!(matches!(
            sniff(b"\x89PNG\r\n\x1a\nxx"),
            Err(ImageError::Truncated(_))
        ));
        assert_eq!(sniff(&png_bytes(0, 5, None)), Err(ImageError::BadSize));
        assert!(matches!(sniff(b"<svg"), Err(ImageError::Svg(_))));
    }
}
