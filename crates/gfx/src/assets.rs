//! CPU-side resources behind a draw list: registered fonts, rasterised glyphs, SVG icons and
//! images, all packed into atlases that the renderer later uploads. Nothing here needs a GPU.

use std::collections::HashMap;
use std::sync::Arc;

use resvg::{tiny_skia, usvg};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, SmoothMode, Target};
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::atlas::{Atlas, AtlasSlot};
use crate::error::GfxError;
use crate::raster::{Flattener, RasterMode, rasterize};

/// Registered font, returned by [`Assets::add_font`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontId(pub(crate) u32);

/// Registered SVG icon, returned by [`Assets::add_icon`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IconId(u32);

/// Registered bitmap, returned by [`Assets::add_image`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageId(u32);

/// Which outline hinting is applied before rasterising.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Hinting {
    /// Unhinted outlines.
    None,
    /// FreeType-style light auto-hinting (vertical snapping only).
    Light,
    /// The font's own instructions when it has them, the auto-hinter otherwise. In smooth mode
    /// the TrueType interpreter only moves points vertically, as ClearType does.
    #[default]
    Normal,
}

/// Glyph rasterisation choices. Part of the cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextOptions {
    pub hinting: Hinting,
    /// Horizontal subpixel positions per pixel (3 or 4 are typical; 1 disables).
    pub subpixel_bins: u8,
    /// Rasterise for a ClearType-style RGB-stripe LCD instead of grayscale.
    pub lcd: bool,
}

impl Default for TextOptions {
    fn default() -> Self {
        Self {
            hinting: Hinting::Normal,
            subpixel_bins: 4,
            lcd: false,
        }
    }
}

/// One positioned glyph: baseline origin in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphPos {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// A run of glyphs from one font at one size and colour.
#[derive(Debug, Clone, Copy)]
pub struct GlyphRun<'a> {
    pub font: FontId,
    /// Size in device pixels per em.
    pub size_px: f32,
    pub glyphs: &'a [GlyphPos],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: FontId,
    glyph: u32,
    /// Size in 1/64 px.
    size_q: u32,
    bin: u8,
    options: TextOptions,
}

/// A cached bitmap: its atlas slot and offset from the glyph origin (y down: row 0 is
/// `origin.y - top`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub slot: AtlasSlot,
    pub left: i32,
    pub top: i32,
}

pub(crate) struct FontEntry {
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) index: u32,
}

impl std::fmt::Debug for FontEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontEntry")
            .field("bytes", &self.bytes.len())
            .field("index", &self.index)
            .finish()
    }
}

struct ImageEntry {
    slot: AtlasSlot,
}

/// Fonts, glyph/icon/image atlases and their caches.
pub struct Assets {
    pub(crate) fonts: Vec<FontEntry>,
    hinters: HashMap<(FontId, u32, Hinting, bool), Option<HintingInstance>>,
    glyphs: HashMap<GlyphKey, Option<Placed>>,
    icons: Vec<usvg::Tree>,
    icon_masks: HashMap<(IconId, u32, u32), Option<Placed>>,
    pub(crate) path_masks: HashMap<u64, Option<Placed>>,
    pub(crate) fallbacks: HashMap<FontId, Vec<FontId>>,
    images: Vec<ImageEntry>,
    pub(crate) mask: Atlas,
    lcd: Atlas,
    color: Atlas,
    text: TextOptions,
    generation: u32,
    pub(crate) overflowed: bool,
}

impl std::fmt::Debug for Assets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Assets")
            .field("fonts", &self.fonts.len())
            .field("glyphs", &self.glyphs.len())
            .field("icons", &self.icons.len())
            .field("images", &self.images.len())
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

/// Reset the glyph atlases at frame start when they are fuller than this.
const RESET_THRESHOLD: f32 = 0.8;

impl Assets {
    /// `atlas_size` is the side of each square atlas texture (2048 is a safe default).
    pub fn new(atlas_size: u32) -> Self {
        Self {
            fonts: Vec::new(),
            hinters: HashMap::new(),
            glyphs: HashMap::new(),
            icons: Vec::new(),
            icon_masks: HashMap::new(),
            path_masks: HashMap::new(),
            fallbacks: HashMap::new(),
            images: Vec::new(),
            mask: Atlas::new(atlas_size, atlas_size, 1),
            lcd: Atlas::new(atlas_size, atlas_size, 4),
            color: Atlas::new(atlas_size, atlas_size, 4),
            text: TextOptions::default(),
            generation: 0,
            overflowed: false,
        }
    }

    pub fn atlas_size(&self) -> u32 {
        self.mask.width()
    }

    pub fn text_options(&self) -> TextOptions {
        self.text
    }

    /// Changes how glyphs are rasterised. Cached glyphs of other settings stay valid (the
    /// options are part of the key) but only matter again if the options change back.
    pub fn set_text_options(&mut self, options: TextOptions) {
        self.text = TextOptions {
            subpixel_bins: options.subpixel_bins.clamp(1, 8),
            ..options
        };
    }

    /// Incremented every time the glyph and icon atlases are cleared.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// True when an item did not fit since the last call. Its draw was skipped; build the frame
    /// again (the atlases were cleared) or enlarge the atlas.
    pub fn take_overflow(&mut self) -> bool {
        std::mem::take(&mut self.overflowed)
    }

    /// The grayscale coverage atlas (glyphs and icons), for inspection.
    pub fn mask(&self) -> &Atlas {
        &self.mask
    }

    /// The LCD coverage atlas (RGBA texels: R, G, B coverage and their mean).
    pub fn lcd(&self) -> &Atlas {
        &self.lcd
    }

    /// The image atlas (sRGB RGBA).
    pub fn color(&self) -> &Atlas {
        &self.color
    }

    pub(crate) fn mask_atlas(&mut self) -> &mut Atlas {
        &mut self.mask
    }

    pub(crate) fn lcd_atlas(&mut self) -> &mut Atlas {
        &mut self.lcd
    }

    pub(crate) fn color_atlas(&mut self) -> &mut Atlas {
        &mut self.color
    }

    /// Call once before building a frame: clears glyph atlases that are nearly full so a
    /// frame never has to evict mid-way.
    pub fn begin_frame(&mut self) {
        if self.mask.utilisation() > RESET_THRESHOLD || self.lcd.utilisation() > RESET_THRESHOLD {
            self.reset_glyph_caches();
        }
    }

    pub(crate) fn reset_glyph_caches(&mut self) {
        self.glyphs.clear();
        self.icon_masks.clear();
        self.path_masks.clear();
        self.mask.clear();
        self.lcd.clear();
        self.generation += 1;
    }

    pub fn add_font(&mut self, bytes: Arc<[u8]>, index: u32) -> Result<FontId, GfxError> {
        let font = FontRef::from_index(&bytes, index).map_err(|e| GfxError::Font(e.to_string()))?;
        if font.outline_glyphs().iter().next().is_none() {
            return Err(GfxError::Font("font has no outlines".into()));
        }
        let id = FontId(self.fonts.len() as u32);
        self.fonts.push(FontEntry { bytes, index });
        Ok(id)
    }

    /// Parses SVG bytes. The icon is rasterised on demand at the size it is drawn, as a coverage
    /// mask, so it takes its colour from the draw call (Tabler and Lucide use `currentColor`).
    pub fn add_icon(&mut self, svg: &[u8]) -> Result<IconId, GfxError> {
        let tree = usvg::Tree::from_data(svg, &usvg::Options::default())
            .map_err(|e| GfxError::Svg(e.to_string()))?;
        let id = IconId(self.icons.len() as u32);
        self.icons.push(tree);
        Ok(id)
    }

    /// Registers straight-alpha sRGB RGBA pixels. Must fit one atlas.
    pub fn add_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> Result<ImageId, GfxError> {
        if width == 0 || height == 0 || rgba.len() != (width as usize) * (height as usize) * 4 {
            return Err(GfxError::Image(format!(
                "{width}x{height} image with {} bytes",
                rgba.len()
            )));
        }
        let slot = self
            .color
            .insert(width, height, rgba)
            .ok_or(GfxError::AtlasFull { width, height })?;
        let id = ImageId(self.images.len() as u32);
        self.images.push(ImageEntry { slot });
        Ok(id)
    }

    /// Atlas slot and size of a registered image.
    pub fn image(&self, id: ImageId) -> Option<AtlasSlot> {
        self.images.get(id.0 as usize).map(|i| i.slot)
    }

    fn hinter(
        &mut self,
        font: FontId,
        size_q: u32,
        options: TextOptions,
    ) -> Option<&HintingInstance> {
        if options.hinting == Hinting::None {
            return None;
        }
        let key = (font, size_q, options.hinting, options.lcd);
        if !self.hinters.contains_key(&key) {
            let inst = self.build_hinter(font, size_q, options);
            self.hinters.insert(key, inst);
        }
        self.hinters.get(&key).and_then(Option::as_ref)
    }

    fn build_hinter(
        &self,
        font: FontId,
        size_q: u32,
        options: TextOptions,
    ) -> Option<HintingInstance> {
        let entry = self.fonts.get(font.0 as usize)?;
        let font_ref = FontRef::from_index(&entry.bytes, entry.index).ok()?;
        let outlines = font_ref.outline_glyphs();
        let (engine, mode) = match options.hinting {
            Hinting::None => return None,
            Hinting::Light => (Engine::Auto(None), SmoothMode::Light),
            Hinting::Normal => (
                Engine::AutoFallback,
                if options.lcd {
                    SmoothMode::Lcd
                } else {
                    SmoothMode::Normal
                },
            ),
        };
        let target = Target::Smooth {
            mode,
            symmetric_rendering: true,
            preserve_linear_metrics: true,
        };
        HintingInstance::new(
            &outlines,
            Size::new(size_q as f32 / 64.0),
            LocationRef::default(),
            HintingOptions { engine, target },
        )
        .ok()
    }

    /// Flattened outline of a glyph in pixels (y up), hinted per `options`.
    fn outline_lines(
        &mut self,
        font: FontId,
        glyph: u32,
        size_q: u32,
        options: TextOptions,
    ) -> Option<Vec<[f32; 4]>> {
        // Build the hinter first (needs &mut self), then draw with shared borrows.
        let hinted = self.hinter(font, size_q, options).is_some();
        let entry = self.fonts.get(font.0 as usize)?;
        let font_ref = FontRef::from_index(&entry.bytes, entry.index).ok()?;
        let outline = font_ref.outline_glyphs().get(GlyphId::new(glyph))?;
        let mut pen = Flattener::default();
        let size = Size::new(size_q as f32 / 64.0);
        if hinted {
            let key = (font, size_q, options.hinting, options.lcd);
            let inst = self.hinters.get(&key)?.as_ref()?;
            outline
                .draw(DrawSettings::hinted(inst, false), &mut pen)
                .ok()?;
        } else {
            outline
                .draw(
                    DrawSettings::unhinted(size, LocationRef::default()),
                    &mut pen,
                )
                .ok()?;
        }
        Some(pen.finish())
    }

    /// Rasterises (or fetches) one glyph at subpixel `bin` and returns where it sits.
    pub fn glyph(&mut self, font: FontId, glyph: u32, size_px: f32, bin: u8) -> Option<Placed> {
        let options = self.text;
        let size_q = (size_px * 64.0).round().max(1.0) as u32;
        let key = GlyphKey {
            font,
            glyph,
            size_q,
            bin,
            options,
        };
        if let Some(hit) = self.glyphs.get(&key) {
            return *hit;
        }
        let placed = self.rasterise_glyph(&key);
        self.glyphs.insert(key, placed);
        placed
    }

    fn rasterise_glyph(&mut self, key: &GlyphKey) -> Option<Placed> {
        let lines = self.outline_lines(key.font, key.glyph, key.size_q, key.options)?;
        let offset = f32::from(key.bin) / f32::from(key.options.subpixel_bins.max(1));
        let mode = if key.options.lcd {
            RasterMode::Lcd
        } else {
            RasterMode::Gray
        };
        let bitmap = rasterize(&lines, offset, mode)?;
        let slot = if key.options.lcd {
            // RGB coverage into RGBA texels; alpha carries the mean for debugging.
            let mut rgba = Vec::with_capacity(bitmap.data.len() / 3 * 4);
            for px in bitmap.data.chunks_exact(3) {
                let mean = ((u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2])) / 3) as u8;
                rgba.extend_from_slice(&[px[0], px[1], px[2], mean]);
            }
            self.lcd.insert(bitmap.width, bitmap.height, &rgba)
        } else {
            self.mask.insert(bitmap.width, bitmap.height, &bitmap.data)
        };
        match slot {
            Some(slot) => Some(Placed {
                slot,
                left: bitmap.left,
                top: bitmap.top,
            }),
            None => {
                self.overflowed = true;
                self.reset_glyph_caches();
                None
            }
        }
    }

    /// Rasterises (or fetches) icon `id` as a coverage mask of `width` by `height` pixels.
    pub fn icon_mask(&mut self, id: IconId, width: u32, height: u32) -> Option<Placed> {
        if let Some(hit) = self.icon_masks.get(&(id, width, height)) {
            return *hit;
        }
        let placed = self.rasterise_icon(id, width, height);
        self.icon_masks.insert((id, width, height), placed);
        placed
    }

    fn rasterise_icon(&mut self, id: IconId, width: u32, height: u32) -> Option<Placed> {
        let tree = self.icons.get(id.0 as usize)?;
        let size = tree.size();
        if width == 0 || height == 0 || size.width() <= 0.0 || size.height() <= 0.0 {
            return None;
        }
        let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
        let transform = tiny_skia::Transform::from_scale(
            width as f32 / size.width(),
            height as f32 / size.height(),
        );
        resvg::render(tree, transform, &mut pixmap.as_mut());
        let coverage: Vec<u8> = pixmap.data().chunks_exact(4).map(|p| p[3]).collect();
        match self.mask.insert(width, height, &coverage) {
            Some(slot) => Some(Placed {
                slot,
                left: 0,
                top: 0,
            }),
            None => {
                self.overflowed = true;
                self.reset_glyph_caches();
                None
            }
        }
    }
}
