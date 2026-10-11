//! Image XObjects. Pictures come from untrusted documents: sizes are capped before any pixel
//! buffer is allocated, JPEG is passed through only after its header is validated, and every
//! other format is decoded by the `image` crate under allocation limits.

use std::io::Cursor;

use crate::writer::{Pdf, deflate};

/// Largest accepted picture, in pixels (a 5000 x 5000 image).
pub const MAX_PIXELS: u64 = 25_000_000;
/// Largest accepted side, in pixels.
pub const MAX_SIDE: u32 = 20_000;
/// Largest accepted image part, in bytes.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
/// Allocation ceiling handed to the decoder.
const MAX_ALLOC: u64 = 256 * 1024 * 1024;

/// Writes the image as an XObject and returns its object number. With `flatten` (PDF/A-1, which
/// has no transparency) a picture with an alpha channel is composited onto white instead of
/// getting a soft mask.
pub fn embed(pdf: &mut Pdf, bytes: &[u8], flatten: bool) -> Result<u32, String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!(
            "image part of {} bytes exceeds the limit",
            bytes.len()
        ));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        embed_jpeg(pdf, bytes)
    } else {
        embed_decoded(pdf, bytes, flatten)
    }
}

fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("image size {width}x{height} is out of range"));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!(
            "image size {width}x{height} exceeds the pixel limit"
        ));
    }
    Ok(())
}

struct JpegInfo {
    width: u32,
    height: u32,
    components: u8,
    adobe_inverted: bool,
}

/// Walks the marker segments up to the start of scan; rejects what a viewer could not show.
fn parse_jpeg(d: &[u8]) -> Result<JpegInfo, String> {
    let mut pos = 2;
    let mut adobe = false;
    let mut sof = None;
    loop {
        // Marker: 0xFF, optional fill 0xFF bytes, code.
        if d.get(pos) != Some(&0xFF) {
            return Err("malformed JPEG marker".into());
        }
        while d.get(pos) == Some(&0xFF) {
            pos += 1;
        }
        let code = *d.get(pos).ok_or("truncated JPEG")?;
        pos += 1;
        if code == 0x01 || (0xD0..=0xD8).contains(&code) {
            continue;
        }
        if code == 0xD9 {
            return Err("JPEG has no scan".into());
        }
        let len = usize::from(u16::from_be_bytes([
            *d.get(pos).ok_or("truncated JPEG")?,
            *d.get(pos + 1).ok_or("truncated JPEG")?,
        ]));
        if len < 2 {
            return Err("bad JPEG segment length".into());
        }
        let body = d.get(pos + 2..pos + len).ok_or("truncated JPEG segment")?;
        match code {
            0xC0..=0xC2 => {
                if body.len() < 6 {
                    return Err("short JPEG frame header".into());
                }
                if body[0] != 8 {
                    return Err(format!("{}-bit JPEG is not supported", body[0]));
                }
                let height = u32::from(u16::from_be_bytes([body[1], body[2]]));
                let width = u32::from(u16::from_be_bytes([body[3], body[4]]));
                sof = Some((width, height, body[5]));
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err("JPEG coding process is not supported".into());
            }
            0xEE if body.starts_with(b"Adobe") => adobe = true,
            0xDA => break,
            _ => {}
        }
        pos += len;
    }
    let (width, height, components) = sof.ok_or("JPEG has no frame header")?;
    check_size(width, height)?;
    if !matches!(components, 1 | 3 | 4) {
        return Err(format!("{components}-component JPEG is not supported"));
    }
    Ok(JpegInfo {
        width,
        height,
        components,
        adobe_inverted: adobe && components == 4,
    })
}

fn embed_jpeg(pdf: &mut Pdf, bytes: &[u8]) -> Result<u32, String> {
    let info = parse_jpeg(bytes)?;
    let (space, decode) = match (info.components, info.adobe_inverted) {
        (1, _) => ("/DeviceGray", ""),
        (3, _) => ("/DeviceRGB", ""),
        (_, true) => ("/DeviceCMYK", " /Decode [1 0 1 0 1 0 1 0]"),
        _ => ("/DeviceCMYK", ""),
    };
    Ok(pdf.add_stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {space} /BitsPerComponent 8 /Filter /DCTDecode{decode}",
            info.width, info.height
        ),
        bytes,
        false,
    ))
}

fn embed_decoded(pdf: &mut Pdf, bytes: &[u8], flatten: bool) -> Result<u32, String> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
    check_size(w, h)?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    check_size(w, h)?;
    let n = (w as usize) * (h as usize);
    let mut rgb = Vec::with_capacity(n * 3);
    let mut alpha = Vec::with_capacity(n);
    for px in rgba.pixels() {
        let a = u32::from(px.0[3]);
        if flatten && a != 255 {
            for c in &px.0[..3] {
                rgb.push(((u32::from(*c) * a + 255 * (255 - a)) / 255) as u8);
            }
            alpha.push(255);
        } else {
            rgb.extend_from_slice(&px.0[..3]);
            alpha.push(px.0[3]);
        }
    }
    let smask = if alpha.iter().any(|a| *a != 255) {
        let id = pdf.add_stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8"
            ),
            &alpha,
            true,
        );
        format!(" /SMask {id} 0 R")
    } else {
        String::new()
    };
    let data = deflate(&rgb);
    let mut dict = format!(
        "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode{smask}"
    );
    // Already compressed above.
    let id = pdf.reserve();
    dict.push_str(&format!(" /Length {}", data.len()));
    let mut body = format!("<< {dict} >>\nstream\n").into_bytes();
    body.extend_from_slice(&data);
    body.extend_from_slice(b"\nendstream");
    pdf.set(id, body);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, alpha: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([x as u8, y as u8, 7, if alpha { 128 } else { 255 }])
        });
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("encode");
        out
    }

    fn file(pdf: Pdf) -> String {
        String::from_utf8_lossy(&pdf.finish(1, "", [0; 16])).into_owned()
    }

    #[test]
    fn png_alpha_becomes_a_soft_mask() {
        let mut pdf = Pdf::default();
        embed(&mut pdf, &png(4, 3, true), false).expect("embed");
        let text = file(pdf);
        assert!(text.contains("/SMask"));
        assert!(text.contains("/FlateDecode"));
        assert!(text.contains("/Width 4 /Height 3"));
    }

    #[test]
    fn flattened_png_has_no_soft_mask() {
        let mut pdf = Pdf::default();
        embed(&mut pdf, &png(4, 3, true), true).expect("embed");
        assert!(!file(pdf).contains("/SMask"));
    }

    #[test]
    fn opaque_png_has_no_soft_mask() {
        let mut pdf = Pdf::default();
        embed(&mut pdf, &png(4, 3, false), false).expect("embed");
        assert!(!file(pdf).contains("/SMask"));
    }

    #[test]
    fn jpeg_passes_through_with_dct() {
        let mut jpeg = vec![
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x02, 0x00, 0x03, 0x03,
        ];
        jpeg.extend_from_slice(&[1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
        let mut pdf = Pdf::default();
        embed(&mut pdf, &jpeg, false).expect("embed");
        let text = file(pdf);
        assert!(text.contains("/DCTDecode"));
        assert!(text.contains("/Width 3 /Height 2"));
    }

    #[test]
    fn hostile_images_are_rejected_without_allocating() {
        let mut pdf = Pdf::default();
        // JPEG claiming 65535 x 65535.
        let mut jpeg = vec![
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0x03,
        ];
        jpeg.extend_from_slice(&[1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        assert!(embed(&mut pdf, &jpeg, false).is_err());
        // Truncated and garbage input.
        assert!(embed(&mut pdf, &jpeg[..10], false).is_err());
        assert!(embed(&mut pdf, &[0xFF, 0xD8], false).is_err());
        assert!(embed(&mut pdf, b"not an image at all", false).is_err());
        assert!(embed(&mut pdf, &[], false).is_err());
        // A PNG whose header declares 100000 x 100000 pixels (CRC repaired).
        let mut bad = png(1, 1, false);
        bad[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        bad[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        let mut crc = flate2::Crc::new();
        crc.update(&bad[12..29]);
        bad[29..33].copy_from_slice(&crc.sum().to_be_bytes());
        let err = embed(&mut pdf, &bad, false).expect_err("too large");
        assert!(
            err.contains("out of range") || err.contains("limit"),
            "{err}"
        );
    }
}
