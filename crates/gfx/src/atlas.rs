//! CPU-side texture atlases. Pixels live in a CPU mirror so draw lists can be built, and tests
//! can inspect results, without a GPU; the renderer uploads the dirty region before drawing.

use crate::geom::PixelRect;

/// Gap kept around every packed item so filtered sampling never bleeds a neighbour in.
const PADDING: u32 = 1;

#[derive(Debug, Clone)]
struct Shelf {
    y: u32,
    height: u32,
    cursor: u32,
}

/// Shelf (row) packer: items go left to right on the shortest shelf that fits them; a new
/// shelf opens when none does. Simple, deterministic and good for glyphs of similar height.
#[derive(Debug, Clone)]
pub struct ShelfPacker {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    next_y: u32,
    used_area: u64,
}

impl ShelfPacker {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            shelves: Vec::new(),
            next_y: 0,
            used_area: 0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Fraction of the atlas area handed out so far (0 to 1), padding included.
    pub fn utilisation(&self) -> f32 {
        self.used_area as f32 / (u64::from(self.width) * u64::from(self.height)) as f32
    }

    pub fn clear(&mut self) {
        self.shelves.clear();
        self.next_y = 0;
        self.used_area = 0;
    }

    /// Reserves a `w` by `h` region (plus padding) and returns its top-left corner.
    pub fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let pw = w.checked_add(PADDING)?;
        let ph = h.checked_add(PADDING)?;
        if pw > self.width || ph > self.height {
            return None;
        }
        // Best fit: the shelf with the least wasted height that still has room.
        let mut best: Option<usize> = None;
        for (i, s) in self.shelves.iter().enumerate() {
            // A shelf more than twice as tall as the item would waste too much; open a new one.
            if s.height >= ph
                && s.height <= ph * 2
                && self.width - s.cursor >= pw
                && best.is_none_or(|b| s.height < self.shelves[b].height)
            {
                best = Some(i);
            }
        }
        if let Some(i) = best {
            let s = &mut self.shelves[i];
            let pos = (s.cursor, s.y);
            s.cursor += pw;
            self.used_area += u64::from(pw) * u64::from(s.height);
            return Some(pos);
        }
        if self.next_y + ph > self.height {
            return None;
        }
        let y = self.next_y;
        self.shelves.push(Shelf {
            y,
            height: ph,
            cursor: pw,
        });
        self.next_y += ph;
        self.used_area += u64::from(pw) * u64::from(ph);
        Some((0, y))
    }
}

/// Where an item sits inside an atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AtlasSlot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A packed pixel store with `channels` bytes per pixel and dirty-region tracking.
#[derive(Debug, Clone)]
pub struct Atlas {
    packer: ShelfPacker,
    channels: u32,
    pixels: Vec<u8>,
    dirty: PixelRect,
}

impl Atlas {
    pub fn new(width: u32, height: u32, channels: u32) -> Self {
        Self {
            packer: ShelfPacker::new(width, height),
            channels,
            pixels: vec![0; (width * height * channels) as usize],
            dirty: PixelRect::default(),
        }
    }

    pub fn width(&self) -> u32 {
        self.packer.width()
    }

    pub fn height(&self) -> u32 {
        self.packer.height()
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn utilisation(&self) -> f32 {
        self.packer.utilisation()
    }

    /// Takes the region changed since the last call (empty when nothing changed).
    pub fn take_dirty(&mut self) -> PixelRect {
        std::mem::take(&mut self.dirty)
    }

    /// Forgets every item and zeroes the pixels; the whole atlas becomes dirty.
    pub fn clear(&mut self) {
        self.packer.clear();
        self.pixels.fill(0);
        self.dirty = PixelRect::new(0, 0, self.width(), self.height());
    }

    /// Packs a `w` by `h` image (`channels` bytes per pixel, tightly packed rows).
    pub fn insert(&mut self, w: u32, h: u32, data: &[u8]) -> Option<AtlasSlot> {
        debug_assert_eq!(data.len(), (w * h * self.channels) as usize);
        let (x, y) = self.packer.allocate(w, h)?;
        let row = (w * self.channels) as usize;
        for r in 0..h {
            let dst = (((y + r) * self.width() + x) * self.channels) as usize;
            let src = r as usize * row;
            self.pixels[dst..dst + row].copy_from_slice(&data[src..src + row]);
        }
        self.dirty = self.dirty.union(&PixelRect::new(x, y, x + w, y + h));
        Some(AtlasSlot {
            x,
            y,
            width: w,
            height: h,
        })
    }

    /// One pixel of the CPU mirror, for tests and debugging.
    pub fn pixel(&self, x: u32, y: u32) -> &[u8] {
        let i = ((y * self.width() + x) * self.channels) as usize;
        &self.pixels[i..i + self.channels as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_on_one_shelf_advance_left_to_right() {
        let mut p = ShelfPacker::new(64, 64);
        assert_eq!(p.allocate(10, 8), Some((0, 0)));
        assert_eq!(p.allocate(10, 8), Some((11, 0)));
        assert_eq!(p.allocate(10, 6), Some((22, 0)));
    }

    #[test]
    fn a_taller_item_opens_a_new_shelf() {
        let mut p = ShelfPacker::new(64, 64);
        assert_eq!(p.allocate(10, 8), Some((0, 0)));
        assert_eq!(p.allocate(10, 20), Some((0, 9)));
    }

    #[test]
    fn full_row_wraps_to_a_new_shelf() {
        let mut p = ShelfPacker::new(32, 64);
        assert_eq!(p.allocate(20, 8), Some((0, 0)));
        assert_eq!(p.allocate(20, 8), Some((0, 9)));
    }

    #[test]
    fn best_fit_prefers_the_closest_shelf_height() {
        let mut p = ShelfPacker::new(100, 100);
        p.allocate(10, 30);
        p.allocate(10, 10);
        // The 10-high shelf is the tighter fit for a 9-high item.
        let (_, y) = p.allocate(10, 9).expect("fits");
        assert_eq!(y, 31);
    }

    #[test]
    fn oversized_and_overflowing_items_are_refused() {
        let mut p = ShelfPacker::new(16, 16);
        assert_eq!(p.allocate(16, 4), None, "padding does not fit");
        assert_eq!(p.allocate(4, 16), None);
        for _ in 0..3 {
            assert!(p.allocate(14, 4).is_some());
        }
        assert_eq!(p.allocate(14, 4), None);
    }

    #[test]
    fn allocations_never_overlap() {
        let mut p = ShelfPacker::new(128, 128);
        let mut placed: Vec<(u32, u32, u32, u32)> = Vec::new();
        for i in 0..200u32 {
            let (w, h) = (3 + (i * 7) % 17, 4 + (i * 11) % 13);
            if let Some((x, y)) = p.allocate(w, h) {
                for &(ox, oy, ow, oh) in &placed {
                    let disjoint = x + w <= ox || ox + ow <= x || y + h <= oy || oy + oh <= y;
                    assert!(disjoint, "({x},{y},{w},{h}) overlaps ({ox},{oy},{ow},{oh})");
                }
                assert!(x + w <= 128 && y + h <= 128);
                placed.push((x, y, w, h));
            }
        }
        assert!(placed.len() > 20);
    }

    #[test]
    fn clear_resets_utilisation() {
        let mut p = ShelfPacker::new(32, 32);
        p.allocate(10, 10);
        assert!(p.utilisation() > 0.0);
        p.clear();
        assert_eq!(p.utilisation(), 0.0);
        assert_eq!(p.allocate(10, 10), Some((0, 0)));
    }

    #[test]
    fn insert_copies_rows_and_marks_the_dirty_region() {
        let mut a = Atlas::new(16, 16, 1);
        let slot = a.insert(2, 2, &[1, 2, 3, 4]).expect("fits");
        assert_eq!((slot.x, slot.y), (0, 0));
        assert_eq!(a.pixel(0, 0), &[1]);
        assert_eq!(a.pixel(1, 1), &[4]);
        assert_eq!(a.take_dirty(), PixelRect::new(0, 0, 2, 2));
        assert!(a.take_dirty().is_empty());
        let slot = a.insert(2, 1, &[9, 9]).expect("fits");
        assert_eq!(
            a.take_dirty(),
            PixelRect::new(slot.x, slot.y, slot.x + 2, slot.y + 1)
        );
    }

    #[test]
    fn clear_zeroes_pixels_and_dirties_everything() {
        let mut a = Atlas::new(8, 8, 4);
        a.insert(1, 1, &[1, 2, 3, 4]).expect("fits");
        a.take_dirty();
        a.clear();
        assert_eq!(a.pixel(0, 0), &[0, 0, 0, 0]);
        assert_eq!(a.take_dirty(), PixelRect::new(0, 0, 8, 8));
    }
}
