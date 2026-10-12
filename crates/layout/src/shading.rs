//! Paragraph shading (`w:shd` in `w:pPr`): the band Word fills behind each line.
//!
//! Measured on Word's PDF (`fixtures/reference/pdf/probes/p-pshd`, `layout-pbdr-ctx`): the band
//! spans the text area plus 1.44 pt on each side (the same gap the border rules stand off by),
//! inside the border rules of a bordered paragraph, from the top of the first line to the bottom
//! of the last (a border box: from the inner edge of the top rule to the inner edge of the bottom
//! rule). Between two paragraphs with the same band edges the whole gap is filled: the upper
//! band reaches down by its space after, the lower one up by the rest. Next to an unshaded
//! paragraph, or a shaded one with other edges, the space stays unfilled.
//! [`shades_of`] derives the final rectangles after pagination, like the border rules.

use crate::Line;
use crate::rules::{box_edges, same_box, snap};

/// The gap the band reaches outside the text area, as the border rules do.
const SIDE_GAP: f64 = 1.44;
const EPS: f64 = 1e-6;

/// The fill behind one line.
#[derive(Debug, Clone, PartialEq)]
pub struct LineShading {
    /// Fill colour (a pattern is mixed into a solid colour).
    pub fill: [u8; 3],
    /// `[x0, y0, x1, y1]` of the band in page coordinates (y down), set once the page is
    /// laid out.
    pub rect: [f64; 4],
    pub(crate) left: f64,
    pub(crate) right: f64,
    /// The paragraph's space after, which the band covers when the next paragraph is shaded alike.
    pub(crate) after: f64,
    /// The line is the first or last line of its paragraph.
    pub first: bool,
    pub last: bool,
}

fn plain_x(s: &LineShading) -> (f64, f64) {
    (snap(s.left - SIDE_GAP), snap(s.right + SIDE_GAP))
}

/// Sets the band rectangle of every shaded line of one story on one page.
pub(crate) fn shades_of(lines: &mut [Line]) {
    let n = lines.len();
    let mut i = 0;
    while i < n {
        if lines[i].border.is_none() {
            let (top, bottom) = (lines[i].top, lines[i].top + lines[i].height);
            if let Some(s) = lines[i].shading.as_mut() {
                let (x0, x1) = plain_x(s);
                s.rect = [x0, top, x1, bottom];
            }
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < n && same_box(&lines[j - 1], &lines[j]) {
            j += 1;
        }
        let edges = box_edges(&lines[i..j]);
        for (k, line) in lines.iter_mut().enumerate().take(j).skip(i) {
            let (mut top, mut bottom) = (line.top, line.top + line.height);
            if let Some(e) = edges {
                if k == i {
                    top = e.top_inner.unwrap_or(top);
                }
                if k == j - 1 {
                    bottom = e.bottom_inner.unwrap_or(bottom);
                }
            }
            if let Some(s) = line.shading.as_mut() {
                let (x0, x1) = edges.map_or_else(|| plain_x(s), |e| (e.left_inner, e.right_inner));
                s.rect = [x0, top, x1, bottom];
            }
        }
        i = j;
    }
    // Join neighbours: lines of one paragraph abut; between paragraphs with the same edges the
    // gap is shared.
    for k in 1..n {
        let (head, tail) = lines.split_at_mut(k);
        let (a, b) = (&mut head[k - 1], &mut tail[0]);
        let same_para = a.paragraph == b.paragraph && a.story == b.story;
        let (Some(sa), Some(sb)) = (a.shading.as_mut(), b.shading.as_mut()) else {
            continue;
        };
        if (sa.rect[0] - sb.rect[0]).abs() > EPS || (sa.rect[2] - sb.rect[2]).abs() > EPS {
            continue;
        }
        if same_para {
            let mid = sb.rect[1];
            if mid > sa.rect[3] + EPS && mid - sa.rect[3] < 4.0 {
                sa.rect[3] = mid;
            }
        } else if sa.last && sb.first {
            let gap = sb.rect[1] - sa.rect[3];
            if gap > EPS {
                let down = sa.after.min(gap);
                sa.rect[3] += down;
                sb.rect[1] -= gap - down;
            }
        }
    }
}
