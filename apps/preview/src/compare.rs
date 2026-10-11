//! Pixel comparison of a preview frame against an Office capture, per chrome region.
//!
//! The capture is made by `tools/office/capture-chrome.ps1` (PrintWindow of Office's own window,
//! 1600 x 1000 at 125 %) and stays in a scratch folder. The regions are the rectangles recorded
//! in `fixtures/reference/ui/*-ribbon.json` (window relative, physical pixels at 125 %), so a
//! frame and a capture of the same size line up one to one. A region's score is the root mean
//! square of the per-channel difference in 8-bit levels over every pixel in it.

use crate::AppKind;

/// A named rectangle in device pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Region {
    fn new(name: &str, x: u32, y: u32, w: u32, h: u32) -> Self {
        Self {
            name: name.to_owned(),
            x,
            y,
            w,
            h,
        }
    }
}

/// An RGBA frame (alpha ignored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }
}

/// Title height, tab row y, group row y, group names, group (x, width) spans.
type Layout = (
    u32,
    u32,
    u32,
    &'static [&'static str],
    &'static [(u32, u32)],
);

/// Window-relative regions of the Home tab at 1600 x 1000, 125 % (Excel: Clipboard, Font,
/// Alignment, Number, Styles, Cells, Editing, Add-ins; Word: Clipboard, Font, Paragraph, Styles,
/// Editing, Voice, Editor, Add-ins).
pub fn regions(kind: AppKind) -> Vec<Region> {
    let (title_h, tabs_y, group_y, names, spans): Layout = match kind {
        AppKind::Sheets => (
            61,
            61,
            101,
            &[
                "Clipboard",
                "Font",
                "Alignment",
                "Number",
                "Styles",
                "Cells",
                "Editing",
                "Add-ins",
            ],
            &[
                (24, 134),
                (158, 292),
                (450, 253),
                (703, 200),
                (903, 230),
                (1133, 136),
                (1269, 197),
                (1466, 78),
            ],
        ),
        AppKind::Writer => (
            69,
            69,
            109,
            &[
                "Clipboard",
                "Font",
                "Paragraph",
                "Styles",
                "Editing",
                "Voice",
                "Editor",
                "Add-ins",
            ],
            &[
                (24, 104),
                (128, 386),
                (514, 239),
                (753, 463),
                (1216, 70),
                (1286, 74),
                (1360, 72),
                (1432, 78),
            ],
        ),
    };
    let mut out = vec![
        Region::new("title row", 0, 0, 1600, title_h),
        Region::new("tabs", 0, tabs_y, 1600, 38),
    ];
    for (n, (x, w)) in names.iter().zip(spans) {
        out.push(Region::new(&format!("group {n}"), *x, group_y, *w, 120));
    }
    if kind == AppKind::Sheets {
        out.push(Region::new("formula bar", 9, 222, 1582, 96));
    }
    out.push(Region::new("status bar", 9, 964, 1582, 27));
    out
}

/// Root mean square difference, in levels, of `a` and `b` over `r` (all three channels).
pub fn region_rms(a: &Frame, b: &Frame, r: &Region) -> f64 {
    let w =
        r.w.min(a.width.saturating_sub(r.x))
            .min(b.width.saturating_sub(r.x));
    let h =
        r.h.min(a.height.saturating_sub(r.y))
            .min(b.height.saturating_sub(r.y));
    if w == 0 || h == 0 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for y in r.y..r.y + h {
        for x in r.x..r.x + w {
            let (p, q) = (a.at(x, y), b.at(x, y));
            for c in 0..3 {
                let d = f64::from(p[c]) - f64::from(q[c]);
                sum += d * d;
            }
        }
    }
    (sum / f64::from(w * h * 3)).sqrt()
}

/// One scored region.
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    pub name: String,
    pub rms: f64,
}

/// Scores every region of `kind`, plus `whole window` over the full frame.
pub fn compare(kind: AppKind, frame: &Frame, capture: &Frame) -> Vec<Score> {
    let mut out: Vec<Score> = regions(kind)
        .iter()
        .map(|r| Score {
            name: r.name.clone(),
            rms: region_rms(frame, capture, r),
        })
        .collect();
    let all = Region::new("whole window", 0, 0, frame.width, frame.height);
    out.push(Score {
        name: all.name.clone(),
        rms: region_rms(frame, capture, &all),
    });
    out
}

/// A text table of scores.
pub fn format_scores(scores: &[Score]) -> String {
    let mut s = String::new();
    for sc in scores {
        s.push_str(&format!("{:<18} {:7.2}\n", sc.name, sc.rms));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, v: u8) -> Frame {
        Frame {
            width: w,
            height: h,
            rgba: vec![v; (w * h * 4) as usize],
        }
    }

    #[test]
    fn rms_of_identical_frames_is_zero_and_of_a_constant_offset_is_that_offset() {
        let a = flat(1600, 1000, 40);
        let b = flat(1600, 1000, 40);
        let r = Region::new("r", 10, 10, 100, 50);
        assert_eq!(region_rms(&a, &b, &r), 0.0);
        let c = flat(1600, 1000, 50);
        assert!((region_rms(&a, &c, &r) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn rms_only_counts_pixels_inside_the_region() {
        let a = flat(100, 100, 0);
        let mut b = flat(100, 100, 0);
        // One pixel differs by 30 in every channel: RMS over a 10 x 10 region = 30 / 10.
        let i = ((5 * 100 + 5) * 4) as usize;
        b.rgba[i..i + 3].fill(30);
        let r = Region::new("r", 0, 0, 10, 10);
        assert!((region_rms(&a, &b, &r) - 3.0).abs() < 1e-9);
        let outside = Region::new("o", 50, 50, 10, 10);
        assert_eq!(region_rms(&a, &b, &outside), 0.0);
    }

    #[test]
    fn regions_cover_title_tabs_every_group_formula_bar_and_status_bar() {
        let s = regions(AppKind::Sheets);
        assert_eq!(s.len(), 2 + 8 + 2);
        assert!(s.iter().any(|r| r.name == "formula bar"));
        let w = regions(AppKind::Writer);
        assert_eq!(w.len(), 2 + 8 + 1);
        assert!(w.iter().all(|r| r.x + r.w <= 1600 && r.y + r.h <= 1000));
    }
}
