//! Floating objects of the sheet (pictures, shapes, charts) on the Sheets preview grid.
//!
//! The objects come from the sheet's drawing part (`Workbook::sheet_drawing_objects`), are placed
//! with the same pixel geometry as the cells, and are drawn into the grid's draw list by
//! [`paint`]. Shapes and pictures go through `open365-drawing` (display list, scene); charts
//! through `open365-chart`. Nothing here edits the file.
//!
//! Hook (in `doc::paint_sheet`, after the cells and before the selection):
//! `crate::sheet_ext::drawing::paint(ui, list, &g, model);`

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use open365_chart::{
    ApproxMeasure, ChartSpace, DisplayItem, PathCmd as ChartCmd, RenderOptions, Rgba as ChartRgba,
    TextAnchor, WorkbookCells, render,
};
use open365_drawing::geom::{Builtin, P};
use open365_drawing::scene::{Poly, Scene, SceneItem, build_scene, place_in_rect};
use open365_drawing::{Affine, Kind};
use open365_gfx::{Cap, Color, DrawList, ImageId, Join, Path, Rect};
use open365_ui_core::Ui;
use open365_xlsx::{SheetObject, Workbook};

use crate::sheet::{Geo, SheetModel};

/// Largest side of a decoded picture kept in the atlas.
const ATLAS_SIDE: u32 = 1024;
/// Logical pixels per point at 100 % zoom.
const PX_PER_PT: f64 = 96.0 / 72.0;

struct Item {
    /// Left, top, width, height in logical pixels from the corner of A1.
    rect: (f64, f64, f64, f64),
    scene: Scene,
    chart: Option<ChartSpace>,
    images: HashMap<String, Arc<[u8]>>,
}

#[derive(Default)]
struct Cache {
    key: Option<(u64, usize, usize)>,
    generation: u32,
    items: Vec<Item>,
    ids: HashMap<(usize, String), Option<ImageId>>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

fn build(wb: &Workbook, sheet: usize, metrics: open365_xlsx::GridMetrics) -> Vec<Item> {
    let Some(ws) = wb.sheet(sheet) else {
        return Vec::new();
    };
    let theme = wb.drawing_theme();
    // The preview grid is laid out in logical pixels at 96 dpi.
    let k = PX_PER_PT;
    let mut out = Vec::new();
    for obj in wb.sheet_drawing_objects(sheet) {
        if obj.object.hidden {
            continue;
        }
        let r = obj.anchor.rect_px_pt(ws, metrics);
        let rect = (r.x * k, r.y * k, r.width * k, r.height * k);
        let mut scene = Scene::default();
        let mut chart = None;
        match &obj.object.kind {
            Kind::Shape(_) | Kind::Picture(_) | Kind::Group(_) => {
                let placed = place_in_rect(&obj.object, 0.0, 0.0, r.width, r.height);
                scene = build_scene(&placed, &Builtin, &theme, Affine::IDENTITY);
            }
            Kind::Chart { rid } => {
                chart = obj
                    .target(rid)
                    .and_then(|p| wb.package().part(p.as_str()))
                    .and_then(|p| ChartSpace::parse(p.data()).ok());
            }
            Kind::Unknown { .. } => {}
        }
        out.push(Item {
            rect,
            images: images_of(wb, &obj, &scene),
            scene,
            chart,
        });
    }
    out
}

fn images_of(wb: &Workbook, obj: &SheetObject, scene: &Scene) -> HashMap<String, Arc<[u8]>> {
    let mut images = HashMap::new();
    for it in &scene.items {
        if let SceneItem::Image(img) = it
            && let Some(rid) = &img.embed
            && let Some(part) = obj.target(rid).and_then(|p| wb.package().part(p.as_str()))
        {
            images.insert(rid.clone(), Arc::from(part.data()));
        }
    }
    images
}

fn colour(c: open365_drawing::paint::Rgba) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a)
}

fn chart_colour(c: ChartRgba) -> Color {
    Color::rgba(c.0, c.1, c.2, c.3)
}

/// Draws the objects of the active sheet over the cells. Objects in the frozen panes are drawn
/// in the pane they sit in, objects across a split in each pane they cross.
pub fn paint(ui: &mut Ui, list: &mut DrawList, g: &Geo, model: &SheetModel) {
    let Some(wb) = model.target.xlsx() else {
        return;
    };
    let geo = model.geometry();
    let metrics = geo.metrics();
    let key = (
        model.target.revision(),
        model.sheet,
        std::ptr::from_ref(wb) as usize,
    );
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if cache.key != Some(key) {
            cache.items = build(wb, model.sheet, metrics);
            cache.ids.clear();
            cache.key = Some(key);
        }
        if cache.generation != ui.assets.generation() {
            cache.ids.clear();
            cache.generation = ui.assets.generation();
        }
        if cache.items.is_empty() {
            return;
        }
        let (fr, fc) = g.frozen();
        let dev = |logical: f64| (logical * f64::from(g.scale)).round() as f32;
        let col0 = geo.col_x(1) as f64;
        let row0 = geo.row_y(1) as f64;
        let (fw, fh) = (g.frozen_w(), g.frozen_h());
        // (clip, x frozen, y frozen)
        let (bx, by) = (g.x0 + g.row_header_w, g.y0 + g.col_header_h);
        let mut panes = vec![(
            Rect::from_edges(g.scroll_x0(), g.scroll_y0(), g.x1, g.y1),
            false,
            false,
        )];
        if fc > 0 {
            panes.push((
                Rect::from_edges(bx, g.scroll_y0(), bx + fw, g.y1),
                true,
                false,
            ));
        }
        if fr > 0 {
            panes.push((
                Rect::from_edges(g.scroll_x0(), by, g.x1, by + fh),
                false,
                true,
            ));
        }
        if fc > 0 && fr > 0 {
            panes.push((Rect::from_edges(bx, by, bx + fw, by + fh), true, true));
        }
        let scroll_x = geo.col_x(g.top_left.1.max(fc + 1)) as f64;
        let scroll_y = geo.row_y(g.top_left.0.max(fr + 1)) as f64;
        for (clip, fx, fy) in panes {
            if clip.x1 <= clip.x0 || clip.y1 <= clip.y0 {
                continue;
            }
            let ox = if fx {
                bx - dev(col0)
            } else {
                g.scroll_x0() - dev(scroll_x)
            };
            let oy = if fy {
                by - dev(row0)
            } else {
                g.scroll_y0() - dev(scroll_y)
            };
            list.push_clip(clip);
            for i in 0..cache.items.len() {
                let (x, y, w, h) = cache.items[i].rect;
                let (l, t) = (ox + dev(x), oy + dev(y));
                let screen = Rect::from_edges(l, t, l + dev(w).max(0.0), t + dev(h).max(0.0));
                if screen.x1 < clip.x0
                    || screen.x0 > clip.x1
                    || screen.y1 < clip.y0
                    || screen.y0 > clip.y1
                {
                    continue;
                }
                paint_item(ui, list, wb, &mut cache, i, screen, g.scale);
            }
            list.pop_clip();
        }
    });
}

fn paint_item(
    ui: &mut Ui,
    list: &mut DrawList,
    wb: &Workbook,
    cache: &mut Cache,
    index: usize,
    screen: Rect,
    scale: f32,
) {
    if let Some(space) = &cache.items[index].chart {
        paint_chart(ui, list, wb, space, screen, scale);
        return;
    }
    let scene = cache.items[index].scene.clone();
    // Scene points are from the object's corner; one point is 96/72 logical pixels.
    let k = scale * PX_PER_PT as f32;
    let at = |p: P| [screen.x0 + p.x as f32 * k, screen.y0 + p.y as f32 * k];
    for item in &scene.items {
        match item {
            SceneItem::Poly(p) => paint_poly(ui, list, p, &at, k),
            SceneItem::Image(img) => {
                let rid = img.embed.clone().unwrap_or_default();
                let id = image_id(ui, cache, index, &rid, img.src_rect);
                let a = at(img.transform.apply(P::new(0.0, 0.0)));
                let b = at(img.transform.apply(P::new(img.width_pt, img.height_pt)));
                let dest = Rect::from_edges(
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[0].max(b[0]),
                    a[1].max(b[1]),
                );
                match id {
                    Some(id) => list.image(&ui.assets, id, dest, Color::rgb(255, 255, 255)),
                    None => list.rect_outline(dest, 1.0, Color::rgb(128, 128, 128)),
                }
            }
        }
    }
}

fn image_id(
    ui: &mut Ui,
    cache: &mut Cache,
    index: usize,
    rid: &str,
    crop: Option<open365_drawing::model::SrcRect>,
) -> Option<ImageId> {
    let key = (index, rid.to_owned());
    if let Some(id) = cache.ids.get(&key) {
        return *id;
    }
    let id = cache.items[index]
        .images
        .get(rid)
        .and_then(|bytes| decode(bytes, crop))
        .and_then(|(w, h, px)| ui.assets.add_image(w, h, &px).ok());
    cache.ids.insert(key, id);
    id
}

/// Decodes PNG, JPEG, GIF and BMP from untrusted bytes, cropped to `crop` and scaled down.
fn decode(
    bytes: &[u8],
    crop: Option<open365_drawing::model::SrcRect>,
) -> Option<(u32, u32, Vec<u8>)> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut rgba = reader.decode().ok()?.to_rgba8();
    if let Some(c) = crop {
        let (w, h) = (f64::from(rgba.width()), f64::from(rgba.height()));
        let frac = |v: i64| (v as f64 / 100_000.0).clamp(0.0, 1.0);
        let x = (w * frac(c.l)) as u32;
        let y = (h * frac(c.t)) as u32;
        let cw = ((w * (1.0 - frac(c.l) - frac(c.r))) as u32).max(1);
        let ch = ((h * (1.0 - frac(c.t) - frac(c.b))) as u32).max(1);
        if x + cw <= rgba.width() && y + ch <= rgba.height() {
            rgba = image::imageops::crop_imm(&rgba, x, y, cw, ch).to_image();
        }
    }
    let longest = rgba.width().max(rgba.height());
    if longest > ATLAS_SIDE {
        let k = f64::from(ATLAS_SIDE) / f64::from(longest);
        rgba = image::imageops::resize(
            &rgba,
            ((f64::from(rgba.width()) * k) as u32).max(1),
            ((f64::from(rgba.height()) * k) as u32).max(1),
            image::imageops::FilterType::Triangle,
        );
    }
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

fn path_of(points: &[P], closed: bool, at: &dyn Fn(P) -> [f32; 2]) -> Path {
    let mut path = Path::new();
    for (i, p) in points.iter().enumerate() {
        let [x, y] = at(*p);
        if i == 0 {
            path.move_to(x, y);
        } else {
            path.line_to(x, y);
        }
    }
    if closed {
        path.close();
    }
    path
}

fn paint_poly(ui: &mut Ui, list: &mut DrawList, poly: &Poly, at: &dyn Fn(P) -> [f32; 2], k: f32) {
    for sub in &poly.subpaths {
        if let Some(f) = poly.fill
            && sub.closed
        {
            list.fill_path(&mut ui.assets, &path_of(&sub.points, true, at), colour(f));
        }
        if let Some(s) = &poly.stroke {
            // Dashes are drawn solid for now.
            let width = (s.width_pt as f32 * k).max(1.0);
            list.stroke_path(
                &mut ui.assets,
                &path_of(&sub.points, sub.closed, at),
                width,
                Join::Miter,
                Cap::Butt,
                colour(s.color),
            );
        }
    }
}

fn paint_chart(
    ui: &mut Ui,
    list: &mut DrawList,
    wb: &Workbook,
    space: &ChartSpace,
    screen: Rect,
    scale: f32,
) {
    let k = scale * PX_PER_PT as f32;
    let (w_pt, h_pt) = (
        f64::from(screen.width() / k),
        f64::from(screen.height() / k),
    );
    let cells = WorkbookCells(wb);
    let opts = RenderOptions::new(&cells, &ApproxMeasure);
    let chart = render(space, w_pt, h_pt, &opts);
    let at = |x: f64, y: f64| [screen.x0 + x as f32 * k, screen.y0 + y as f32 * k];
    let rect_of = |r: &open365_chart::Rect| {
        let (a, b) = (at(r.x, r.y), at(r.right(), r.bottom()));
        Rect::from_edges(a[0], a[1], b[0], b[1])
    };
    list.push_clip(screen);
    let mut depth = 1;
    for item in &chart.items {
        match item {
            DisplayItem::Rect { rect, fill, stroke } => {
                let r = rect_of(rect);
                if let Some(f) = fill {
                    list.fill_rect(r, chart_colour(*f));
                }
                if let Some(s) = stroke {
                    list.rect_outline(r, (s.width as f32 * k).max(1.0), chart_colour(s.color));
                }
            }
            DisplayItem::Path { cmds, fill, stroke } => {
                let mut path = Path::new();
                for c in cmds {
                    match *c {
                        ChartCmd::MoveTo(x, y) => {
                            let p = at(x, y);
                            path.move_to(p[0], p[1]);
                        }
                        ChartCmd::LineTo(x, y) => {
                            let p = at(x, y);
                            path.line_to(p[0], p[1]);
                        }
                        ChartCmd::CubicTo(a, b, c2, d, e, f) => {
                            path.cubic_to(at(a, b), at(c2, d), at(e, f));
                        }
                        ChartCmd::Close => {
                            path.close();
                        }
                    }
                }
                if let Some(f) = fill
                    && f.3 > 0
                {
                    list.fill_path(&mut ui.assets, &path, chart_colour(*f));
                }
                if let Some(s) = stroke {
                    list.stroke_path(
                        &mut ui.assets,
                        &path,
                        (s.width as f32 * k).max(1.0),
                        Join::Round,
                        Cap::Butt,
                        chart_colour(s.color),
                    );
                }
            }
            DisplayItem::PushClip(r) => {
                list.push_clip(rect_of(r));
                depth += 1;
            }
            DisplayItem::PopClip => {
                if depth > 1 {
                    list.pop_clip();
                    depth -= 1;
                }
            }
            DisplayItem::Text(run) => {
                // Rotated text (vertical axis titles) is not drawn yet.
                if run.rotation_deg != 0.0 {
                    continue;
                }
                let px = (run.font.size_pt as f32 * k).max(1.0);
                let lay = ui.assets.layout_text(
                    ui.font,
                    &run.text,
                    px,
                    0.0,
                    0.0,
                    open365_gfx::TextLayoutOptions::default(),
                );
                let left = match run.anchor {
                    TextAnchor::Start => at(run.x, 0.0)[0],
                    TextAnchor::Middle => at(run.x, 0.0)[0] - lay.width / 2.0,
                    TextAnchor::End => at(run.x, 0.0)[0] - lay.width,
                };
                let top = at(0.0, run.y)[1];
                let bottom = at(0.0, run.y + run.height)[1];
                let base = match ui.assets.font_metrics(ui.font, px) {
                    Some(m) => {
                        (top + (bottom - top - (m.ascent + m.descent)) / 2.0 + m.ascent).round()
                    }
                    None => bottom,
                };
                crate::doc::text(ui, list, &run.text, left, base, px, chart_colour(run.color));
            }
        }
    }
    while depth > 0 {
        list.pop_clip();
        depth -= 1;
    }
}
