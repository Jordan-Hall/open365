//! Floating objects on the grid: pictures, shapes and charts of the sheet's drawing part.
//!
//! The drawing part is read through `open365-xlsx` (`sheet_drawing_objects`), placed by the same
//! pixel geometry as the cells, turned into a scene by `open365-drawing` (shapes and pictures)
//! or drawn by `open365-chart` (charts), and painted over the cells. Nothing here edits the file.

use std::collections::HashMap;
use std::sync::Arc;

use egui::epaint::{PathShape, PathStroke};
use egui::{
    Color32, ColorImage, Context, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, TextureHandle,
    TextureOptions, pos2, vec2,
};
use open365_chart::{
    ApproxMeasure, ChartSpace, DisplayItem, PathCmd, RenderOptions, Rgba, TextAnchor,
    WorkbookCells, render,
};
use open365_drawing::geom::Builtin;
use open365_drawing::scene::{Poly, Scene, SceneItem, TextItem, build_scene, place_in_rect};
use open365_drawing::text::{Align, Anchor};
use open365_drawing::{Affine, Kind};
use open365_xlsx::{GridMetrics, SheetObject, Workbook};

/// Points of a worksheet are 1/72 inch; an egui point is 1/96 inch at a scale factor of one.
const EGUI_PER_PT: f32 = 96.0 / 72.0;

/// One object ready to paint.
struct Item {
    object: SheetObject,
    /// Rectangle in egui points from the sheet origin (the corner of cell A1).
    rect: Rect,
    /// Rotation and flips are applied by the scene; charts and pictures use these.
    rotation_deg: f64,
    scene: Scene,
    chart: Option<Arc<ChartSpace>>,
    /// Image bytes by relationship id.
    images: HashMap<String, Arc<[u8]>>,
}

#[derive(Default)]
struct Key {
    layout: usize,
    sheet: usize,
    data: (usize, usize),
}

/// The objects of the current sheet, rebuilt when the sheet, its geometry or its drawing part
/// changes; decoded pictures are kept by part.
#[derive(Default)]
pub(crate) struct DrawingCache {
    key: Option<Key>,
    items: Vec<Item>,
    textures: HashMap<(usize, String), Option<TextureHandle>>,
    /// What could not be drawn faithfully, for the last build (an unknown preset, an image
    /// format without a decoder, text in shapes).
    pub diagnostics: Vec<String>,
    /// What the last frame painted: object name and screen rectangle.
    pub drawn: Vec<(String, Rect)>,
}

fn data_stamp(wb: &Workbook, sheet: usize) -> (usize, usize) {
    // The drawing parts are never edited in place, so the address and size of the first part's
    // bytes identify the workbook's drawing content.
    wb.sheet_drawing_parts(sheet)
        .first()
        .and_then(|part| wb.package().part(part.as_str()))
        .map_or((0, 0), |p| (p.data().as_ptr() as usize, p.data().len()))
}

impl std::fmt::Debug for DrawingCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrawingCache")
            .field("objects", &self.items.len())
            .finish_non_exhaustive()
    }
}

impl DrawingCache {
    /// Rebuilds the item list when `layout` (the address of the sheet layout), the sheet or the
    /// drawing content changed.
    pub fn prepare(
        &mut self,
        wb: &Workbook,
        sheet: usize,
        layout: usize,
        metrics: GridMetrics,
        dpi: u32,
        ppp: f32,
    ) {
        let data = data_stamp(wb, sheet);
        if let Some(k) = &self.key
            && k.layout == layout
            && k.sheet == sheet
            && k.data == data
        {
            return;
        }
        if self.key.as_ref().is_none_or(|k| k.data != data) {
            self.textures.clear();
        }
        self.key = Some(Key {
            layout,
            sheet,
            data,
        });
        self.items.clear();
        self.diagnostics.clear();
        let Some(ws) = wb.sheet(sheet) else { return };
        let theme = wb.drawing_theme();
        // Sheet points -> egui points at this scale factor: the layout is in device pixels at
        // `dpi = 96 * ppp`, one device pixel is `72 / dpi` points.
        let to_egui = f64::from(dpi as f32 / 72.0 / ppp);
        for object in wb.sheet_drawing_objects(sheet) {
            if object.object.hidden {
                continue;
            }
            let r = object.anchor.rect_px_pt(ws, metrics);
            let rect = Rect::from_min_size(
                pos2((r.x * to_egui) as f32, (r.y * to_egui) as f32),
                vec2((r.width * to_egui) as f32, (r.height * to_egui) as f32),
            );
            let mut scene = Scene::default();
            let mut chart = None;
            match &object.object.kind {
                Kind::Shape(_) | Kind::Picture(_) | Kind::Group(_) => {
                    let placed = place_in_rect(&object.object, 0.0, 0.0, r.width, r.height);
                    // Scene coordinates are in sheet points from the object's own corner; the
                    // painter maps them to the screen.
                    scene = build_scene(&placed, &Builtin, &theme, Affine::IDENTITY);
                }
                Kind::Chart { rid } => {
                    chart = object
                        .target(rid)
                        .and_then(|p| wb.package().part(p.as_str()))
                        .and_then(|p| ChartSpace::parse(p.data()).ok())
                        .map(Arc::new);
                    if chart.is_none() {
                        scene.diagnostics.push(format!(
                            "object {}: chart part unreadable",
                            object.object.id
                        ));
                    }
                }
                Kind::Unknown { uri } => scene.diagnostics.push(format!(
                    "object {}: `{uri}` is kept in the file but not drawn",
                    object.object.id
                )),
            }
            let mut images = HashMap::new();
            for it in &scene.items {
                if let SceneItem::Image(img) = it
                    && let Some(rid) = &img.embed
                    && let Some(bytes) = object
                        .target(rid)
                        .and_then(|p| wb.package().part(p.as_str()))
                {
                    images.insert(rid.clone(), Arc::from(bytes.data()));
                }
            }
            self.diagnostics.append(&mut scene.diagnostics);
            let rotation_deg = object.object.xfrm.rot as f64 / 60_000.0;
            self.items.push(Item {
                object,
                rect,
                rotation_deg,
                scene,
                chart,
                images,
            });
        }
        self.diagnostics.sort();
        self.diagnostics.dedup();
    }

    /// Paints the objects that intersect `painter`'s clip. `origin` is the screen position of the
    /// corner of cell A1 for this region (frozen shift and scroll applied).
    pub fn paint(&mut self, painter: &Painter, wb: &Workbook, origin: Pos2) {
        let clip = painter.clip_rect();
        for i in 0..self.items.len() {
            let screen = self.items[i].rect.translate(origin.to_vec2());
            if !screen.intersects(clip) {
                continue;
            }
            self.drawn
                .push((self.items[i].object.object.name.clone(), screen));
            // Charts re-render each frame so edits to their cells show at once.
            let item = &self.items[i];
            match &item.object.object.kind {
                Kind::Chart { .. } => {
                    if let Some(space) = &item.chart {
                        paint_chart(painter, wb, space, screen);
                    }
                }
                _ => {
                    let scene_origin = screen.min;
                    let (rot, rect) = (item.rotation_deg, screen);
                    let scene = item.scene.clone();
                    self.paint_scene(painter, &scene, i, scene_origin, rot, rect);
                }
            }
        }
    }

    fn paint_scene(
        &mut self,
        painter: &Painter,
        scene: &Scene,
        index: usize,
        origin: Pos2,
        _rot: f64,
        _rect: Rect,
    ) {
        for item in &scene.items {
            match item {
                SceneItem::Poly(p) => paint_poly(painter, p, origin),
                SceneItem::Text(t) => paint_text(painter, t, origin),
                SceneItem::Image(img) => {
                    let embed = img.embed.clone().unwrap_or_default();
                    let tex = self.texture(painter.ctx(), index, &embed);
                    if let Some(tex) = tex {
                        paint_image(painter, &tex, img, origin);
                    } else {
                        // An image the decoder cannot read: a frame so the object is visible.
                        let a = img.transform;
                        let tl =
                            to_screen(origin, a.apply(open365_drawing::geom::P::new(0.0, 0.0)));
                        let br = to_screen(
                            origin,
                            a.apply(open365_drawing::geom::P::new(img.width_pt, img.height_pt)),
                        );
                        painter.rect_stroke(
                            Rect::from_two_pos(tl, br),
                            0.0,
                            Stroke::new(1.0, Color32::GRAY),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            }
        }
    }

    fn texture(&mut self, ctx: &Context, index: usize, embed: &str) -> Option<TextureHandle> {
        let key = (index, embed.to_owned());
        if let Some(t) = self.textures.get(&key) {
            return t.clone();
        }
        let tex = self.items[index]
            .images
            .get(embed)
            .and_then(|b| load_texture(ctx, &format!("sheet-picture-{index}"), b));
        if tex.is_none() {
            self.diagnostics
                .push(format!("picture {index}: image format not supported"));
        }
        self.textures.insert(key, tex.clone());
        tex
    }
}

fn to_screen(origin: Pos2, p: open365_drawing::geom::P) -> Pos2 {
    pos2(
        origin.x + p.x as f32 * EGUI_PER_PT,
        origin.y + p.y as f32 * EGUI_PER_PT,
    )
}

fn colour(c: open365_drawing::paint::Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// The text of a shape or text box in the text rectangle: wrapped, aligned, anchored, clipped to the
/// rectangle. Upright whatever the shape's rotation; one regular face.
fn paint_text(painter: &Painter, t: &TextItem, origin: Pos2) {
    let tl = to_screen(origin, open365_drawing::geom::P::new(t.rect.0, t.rect.1));
    let br = to_screen(origin, open365_drawing::geom::P::new(t.rect.2, t.rect.3));
    let area = Rect::from_two_pos(tl, br);
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return;
    }
    let galleys: Vec<_> = t
        .lines
        .iter()
        .map(|l| {
            painter.layout(
                l.text.clone(),
                FontId::proportional((l.size_pt as f32 * EGUI_PER_PT).max(1.0)),
                colour(l.color),
                if t.wrap { area.width() } else { f32::INFINITY },
            )
        })
        .collect();
    let total: f32 = galleys.iter().map(|g| g.size().y).sum();
    let mut y = match t.anchor {
        Anchor::Middle => area.top() + (area.height() - total) / 2.0,
        Anchor::Bottom => area.bottom() - total,
        _ => area.top(),
    };
    let clipped = painter.with_clip_rect(area.intersect(painter.clip_rect()));
    for (g, l) in galleys.into_iter().zip(&t.lines) {
        let x = match l.align {
            Align::Center => area.left() + (area.width() - g.size().x) / 2.0,
            Align::Right => area.right() - g.size().x,
            _ => area.left(),
        };
        let h = g.size().y;
        clipped.galley(pos2(x, y), g, colour(l.color));
        y += h;
    }
}

fn paint_poly(painter: &Painter, poly: &Poly, origin: Pos2) {
    let fill = poly.fill.map(colour);
    for sub in &poly.subpaths {
        let pts: Vec<Pos2> = sub.points.iter().map(|p| to_screen(origin, *p)).collect();
        if let Some(f) = fill
            && sub.closed
        {
            painter.add(Shape::Path(PathShape {
                points: pts.clone(),
                closed: true,
                fill: f,
                stroke: PathStroke::NONE,
            }));
        }
        if let Some(s) = &poly.stroke {
            let width = (s.width_pt as f32 * EGUI_PER_PT).max(1.0);
            let stroke = Stroke::new(width, colour(s.color));
            let mut line = pts;
            if sub.closed
                && let Some(first) = line.first().copied()
            {
                line.push(first);
            }
            if let [on, off, ..] = s.dash.as_slice() {
                painter.extend(Shape::dashed_line(
                    &line,
                    stroke,
                    *on as f32 * EGUI_PER_PT,
                    *off as f32 * EGUI_PER_PT,
                ));
            } else {
                painter.add(Shape::line(line, stroke));
            }
        }
    }
}

fn paint_image(
    painter: &Painter,
    tex: &TextureHandle,
    img: &open365_drawing::render::ImagePrim,
    origin: Pos2,
) {
    use open365_drawing::geom::P;
    let corner = |x: f64, y: f64| to_screen(origin, img.transform.apply(P::new(x, y)));
    let (w, h) = (img.width_pt, img.height_pt);
    let crop = img.src_rect.unwrap_or_default();
    let (u0, v0) = (crop.l as f32 / 100_000.0, crop.t as f32 / 100_000.0);
    let (u1, v1) = (
        1.0 - crop.r as f32 / 100_000.0,
        1.0 - crop.b as f32 / 100_000.0,
    );
    let mut mesh = Mesh::with_texture(tex.id());
    let verts = [
        (corner(0.0, 0.0), pos2(u0, v0)),
        (corner(w, 0.0), pos2(u1, v0)),
        (corner(w, h), pos2(u1, v1)),
        (corner(0.0, h), pos2(u0, v1)),
    ];
    for (pos, uv) in verts {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(Shape::mesh(mesh));
}

// --- charts -------------------------------------------------------------------------------------

fn chart_colour(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3)
}

fn chart_stroke(s: &open365_chart::Stroke) -> Stroke {
    Stroke::new(
        (s.width as f32 * EGUI_PER_PT).max(0.5),
        chart_colour(s.color),
    )
}

fn paint_chart(painter: &Painter, wb: &Workbook, space: &ChartSpace, screen: Rect) {
    let k = EGUI_PER_PT;
    let width_pt = f64::from(screen.width() / k);
    let height_pt = f64::from(screen.height() / k);
    let cells = WorkbookCells(wb);
    let opts = RenderOptions::new(&cells, &ApproxMeasure);
    let list = render(space, width_pt, height_pt, &opts);
    let at = |x: f64, y: f64| pos2(screen.min.x + x as f32 * k, screen.min.y + y as f32 * k);
    let rect_of =
        |r: &open365_chart::Rect| Rect::from_min_max(at(r.x, r.y), at(r.right(), r.bottom()));
    // Chart items are in the chart area, which the display list insets itself; clip to the object.
    let mut clips: Vec<Painter> =
        vec![painter.with_clip_rect(screen.intersect(painter.clip_rect()))];
    for item in &list.items {
        let p = clips.last().cloned().unwrap_or_else(|| painter.clone());
        match item {
            DisplayItem::Rect { rect, fill, stroke } => {
                let r = rect_of(rect);
                if let Some(f) = fill {
                    p.rect_filled(r, 0.0, chart_colour(*f));
                }
                if let Some(s) = stroke {
                    p.rect_stroke(r, 0.0, chart_stroke(s), egui::StrokeKind::Middle);
                }
            }
            DisplayItem::Path { cmds, fill, stroke } => {
                paint_chart_path(&p, cmds, *fill, stroke.as_ref(), &at);
            }
            DisplayItem::PushClip(r) => {
                let c = rect_of(r).intersect(p.clip_rect());
                clips.push(p.with_clip_rect(c));
            }
            DisplayItem::PopClip => {
                if clips.len() > 1 {
                    clips.pop();
                }
            }
            DisplayItem::Text(run) => {
                let font = FontId::proportional((run.font.size_pt as f32 * k).max(1.0));
                let galley = p.layout_no_wrap(run.text.clone(), font, chart_colour(run.color));
                let w = galley.size().x;
                let left = match run.anchor {
                    TextAnchor::Start => run.x,
                    TextAnchor::Middle => run.x - f64::from(w / k) / 2.0,
                    TextAnchor::End => run.x - f64::from(w / k),
                };
                let top = at(left, run.y);
                if run.rotation_deg == 0.0 {
                    p.galley(top, galley, chart_colour(run.color));
                } else {
                    let centre = at(run.x, run.y + run.height / 2.0);
                    let angle = -(run.rotation_deg as f32).to_radians();
                    let rel = top - centre;
                    let (s, c) = angle.sin_cos();
                    let pos = centre + vec2(rel.x * c - rel.y * s, rel.x * s + rel.y * c);
                    p.add(
                        egui::epaint::TextShape::new(pos, galley, chart_colour(run.color))
                            .with_angle(angle),
                    );
                }
            }
        }
    }
}

fn paint_chart_path(
    p: &Painter,
    cmds: &[PathCmd],
    fill: Option<Rgba>,
    stroke: Option<&open365_chart::Stroke>,
    at: &dyn Fn(f64, f64) -> Pos2,
) {
    // Flatten into subpaths.
    let mut subs: Vec<(Vec<Pos2>, bool)> = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();
    for c in cmds {
        match *c {
            PathCmd::MoveTo(x, y) => {
                if cur.len() > 1 {
                    subs.push((std::mem::take(&mut cur), false));
                }
                cur.clear();
                cur.push(at(x, y));
            }
            PathCmd::LineTo(x, y) => cur.push(at(x, y)),
            PathCmd::CubicTo(x1, y1, x2, y2, x, y) => {
                let from = cur.last().copied().unwrap_or_else(|| at(x1, y1));
                let (c1, c2, to) = (at(x1, y1), at(x2, y2), at(x, y));
                let n = 16;
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    cur.push(pos2(
                        u * u * u * from.x
                            + 3.0 * u * u * t * c1.x
                            + 3.0 * u * t * t * c2.x
                            + t * t * t * to.x,
                        u * u * u * from.y
                            + 3.0 * u * u * t * c1.y
                            + 3.0 * u * t * t * c2.y
                            + t * t * t * to.y,
                    ));
                }
            }
            PathCmd::Close => {
                if cur.len() > 1 {
                    subs.push((std::mem::take(&mut cur), true));
                }
                cur.clear();
            }
        }
    }
    if cur.len() > 1 {
        subs.push((cur, false));
    }
    for (pts, closed) in subs {
        if let Some(f) = fill
            && f.3 > 0
        {
            // Chart fills are convex (pie wedges, area strips, bars); tessellate as one polygon.
            p.add(Shape::Path(PathShape {
                points: pts.clone(),
                closed: true,
                fill: chart_colour(f),
                stroke: PathStroke::NONE,
            }));
        }
        if let Some(s) = stroke {
            let mut line = pts;
            if closed && let Some(first) = line.first().copied() {
                line.push(first);
            }
            let st = chart_stroke(s);
            if let [on, off, ..] = s.dash.as_slice() {
                p.extend(Shape::dashed_line(
                    &line,
                    st,
                    *on as f32 * EGUI_PER_PT,
                    *off as f32 * EGUI_PER_PT,
                ));
            } else {
                p.add(Shape::line(line, st));
            }
        }
    }
}

/// Decodes an image part into a texture (PNG, JPEG, GIF and BMP).
fn load_texture(ctx: &Context, name: &str, bytes: &[u8]) -> Option<TextureHandle> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(name, color, TextureOptions::LINEAR))
}
