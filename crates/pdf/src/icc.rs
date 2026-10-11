//! An sRGB ICC profile (version 2, matrix/TRC), built from the public sRGB definition (IEC
//! 61966-2-1: the primaries adapted to D50 and the piecewise transfer curve). PDF/A needs an
//! output intent with a profile when pages use `DeviceRGB`; writing the profile ourselves keeps
//! a third-party profile (and its licence) out of the repository.

/// s15Fixed16 number.
fn fixed(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn xyz_tag(x: f64, y: f64, z: f64) -> Vec<u8> {
    let mut t = b"XYZ \0\0\0\0".to_vec();
    for v in [x, y, z] {
        t.extend_from_slice(&fixed(v));
    }
    t
}

/// The sRGB electro-optical transfer: encoded value to linear light.
fn srgb_to_linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn curve_tag() -> Vec<u8> {
    const N: usize = 1024;
    let mut t = b"curv\0\0\0\0".to_vec();
    t.extend_from_slice(&(N as u32).to_be_bytes());
    for i in 0..N {
        let y = srgb_to_linear(i as f64 / (N - 1) as f64);
        t.extend_from_slice(&((y * 65535.0).round() as u16).to_be_bytes());
    }
    t
}

fn text_tag(s: &str) -> Vec<u8> {
    let mut t = b"text\0\0\0\0".to_vec();
    t.extend_from_slice(s.as_bytes());
    t.push(0);
    t
}

fn desc_tag(s: &str) -> Vec<u8> {
    let mut t = b"desc\0\0\0\0".to_vec();
    t.extend_from_slice(&(s.len() as u32 + 1).to_be_bytes());
    t.extend_from_slice(s.as_bytes());
    t.push(0);
    // Unicode and Macintosh descriptions: empty.
    t.extend_from_slice(&[0; 4 + 4 + 2 + 1 + 67]);
    t
}

/// The profile bytes. The date in the header is fixed so the output is reproducible.
pub fn srgb_profile() -> Vec<u8> {
    let curve = curve_tag();
    let tags: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"desc", desc_tag("sRGB IEC61966-2.1")),
        (
            *b"cprt",
            text_tag("Public domain, built from the sRGB definition by Open365"),
        ),
        (*b"wtpt", xyz_tag(0.9642, 1.0, 0.8249)),
        (*b"rXYZ", xyz_tag(0.4360, 0.2225, 0.0139)),
        (*b"gXYZ", xyz_tag(0.3851, 0.7169, 0.0971)),
        (*b"bXYZ", xyz_tag(0.1431, 0.0606, 0.7141)),
        (*b"rTRC", curve.clone()),
        (*b"gTRC", curve.clone()),
        (*b"bTRC", curve),
    ];
    let table_len = 4 + 12 * tags.len();
    let mut offset = 128 + table_len;
    let mut table = (tags.len() as u32).to_be_bytes().to_vec();
    let mut body = Vec::new();
    for (sig, data) in &tags {
        table.extend_from_slice(sig);
        table.extend_from_slice(&(offset as u32).to_be_bytes());
        table.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 128 + table_len + body.len();
    }
    let total = 128 + table.len() + body.len();
    let mut h = Vec::with_capacity(128);
    h.extend_from_slice(&(total as u32).to_be_bytes());
    h.extend_from_slice(&[0; 4]); // preferred CMM
    h.extend_from_slice(&[2, 0x10, 0, 0]); // version 2.1
    h.extend_from_slice(b"mntr");
    h.extend_from_slice(b"RGB ");
    h.extend_from_slice(b"XYZ ");
    for v in [2024u16, 1, 1, 0, 0, 0] {
        h.extend_from_slice(&v.to_be_bytes());
    }
    h.extend_from_slice(b"acsp");
    h.extend_from_slice(&[0; 4]); // platform
    h.extend_from_slice(&[0; 4]); // flags
    h.extend_from_slice(&[0; 4]); // manufacturer
    h.extend_from_slice(&[0; 4]); // model
    h.extend_from_slice(&[0; 8]); // attributes
    h.extend_from_slice(&[0; 4]); // rendering intent: perceptual
    for v in [0.9642, 1.0, 0.8249] {
        h.extend_from_slice(&fixed(v)); // the D50 illuminant
    }
    h.extend_from_slice(&[0; 4]); // creator
    h.extend_from_slice(&[0; 16]); // profile id
    h.extend_from_slice(&[0; 28]);
    debug_assert_eq!(h.len(), 128);
    let mut out = h;
    out.extend_from_slice(&table);
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_is_well_formed() {
        let p = srgb_profile();
        assert_eq!(
            u32::from_be_bytes(p[0..4].try_into().expect("size")) as usize,
            p.len()
        );
        assert_eq!(&p[36..40], b"acsp");
        assert_eq!(&p[12..16], b"mntr");
        assert_eq!(p.len() % 4, 0);
        // Every tag lies inside the file.
        let n = u32::from_be_bytes(p[128..132].try_into().expect("count")) as usize;
        assert_eq!(n, 9);
        for i in 0..n {
            let e = 132 + 12 * i;
            let off = u32::from_be_bytes(p[e + 4..e + 8].try_into().expect("off")) as usize;
            let len = u32::from_be_bytes(p[e + 8..e + 12].try_into().expect("len")) as usize;
            assert!(off + len <= p.len());
        }
    }
}
