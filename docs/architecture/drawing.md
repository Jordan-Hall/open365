# Shared drawing layer (`open365-drawing`)

Status: M0 done except the preset table (being rebuilt clean-room from Word measurements, below; 187 of 187 authored; 184 match Word on every probe page, 3 have known unmatched pages). Plan decided by the lead on
2026-10-12.

One crate, pure data, no I/O, used by Writer and Sheets for pictures, shapes, text boxes and groups.

## Modules

| Module | Role |
|---|---|
| `model` | `DrawingObject { id, name, descr, hidden, xfrm, kind, raw }`; `Kind` = `Shape`, `Picture`, `Group`, `Chart`, `Unknown`. `Xfrm` in EMU (off, ext, rot in 60000ths of a degree, flips). `RawXml` = verbatim source. |
| `geom` | `ShapeDef` (adjust values, guides, paths, text rect); guide-formula evaluator (`*/ +- +/ ?: abs at2 cat2 cos max min mod pin sat2 sin sqrt tan val`, built-in guides `w h l t r b hc vc wd2..wd32 hd2..hd8 ss ls ssd2..ssd32 cd2 cd4 cd8 3cd4 3cd8 5cd8 7cd8`); path commands `moveTo lnTo arcTo quadBezTo cubicBezTo close` (arcs become cubics), fill modes, stroke flag. `PresetSource` trait, `Builtin`, `MapPresets`. |
| `geom::presets` | The preset table, authored from Word measurements (clean-room method below). 187 of 187 entries (`presets_b01` .. `b08`, one file per batch, plus the pilot five). |
| `paint` | `Color` with modifiers (resolved against a `Theme` trait only when drawn), `Fill`, `Line`, `Effect`, `StyleRef`. |
| `text` | `TextBody<S>`: `BodyProps` (insets, wrap, anchor, vert, autofit) plus a host-generic story (`RawXml` by default). |
| `render` | `display_list(obj, presets, parent: Affine)` gives `Prim::{Path, Image, TextFrame}` in points with group transforms, rotation and flips applied. Unknown presets draw as `rect` and add a diagnostic. |
| `hit` | `bounding_box` only (rotated box under a parent transform). Hit tests and handles come later. |
| `parse` | Read-only: `parse_fragment` reads `a:graphic`, `xdr:graphicFrame`, `wps:wsp`, `xdr:sp`, `xdr:cxnSp`, `pic:pic`, `wpg:wgp` / `xdr:grpSp` into a `DrawingObject`, keeping the fragment as `RawXml`. Not wired into docx or xlsx yet. |

Elements are matched by local name; the host validates namespaces. DOCTYPE is rejected and nesting is capped.

## Decisions (from the plan)

- Anchors stay host-specific: docx `WordAnchor` (z-key `relativeHeight`), xlsx `SheetAnchor` (TwoCell, OneCell, Absolute; shift follows `editAs`).
- VML is read-only for rendering; VML is written only for notes.
- Round trip: writes patch edited fields into the raw tree; untouched objects are written byte-identical; `extLst` and unknown `graphicData` are kept.
- Every edit is a typed command and one undo step, exposed in the mcp registry.
- Default sizes and styles come only from oracle measurement or Office-written files, never guessed.
- Insert > Icons uses the bundled Tabler set as SVG with a PNG fallback.
- The chart crate moves onto `paint`/`Prim` after M3. WordArt warp renders flat in v1. SVG pictures render through resvg and get a PNG fallback as Office does. A drawing object is one CRDT register in v1. SmartArt and ink are preserve-only. Word's Draw Text Box default wrap is measured in the oracle.

## Preset geometry: clean-room method (owner decision, 2026-10-12)

The 187 preset shapes are not generated from Ecma's `presetShapeDefinitions.xml`: the licence on that file is
unclear (see the 2026-10-12 check in the decision log), so the owner chose to rebuild them from the ECMA-376 prose
plus measurement of Word's own output. `geom::presets::table()` fills up one measured preset at a time.

**Exclusion list. These must never be opened, read, searched, diffed against or used as a source** (by people or
agents, for presets or for checking them):

- Ecma's `presetShapeDefinitions.xml`, and anything under `D:\dev\open365\dl-ecma`;
- `crates/pdf/src/shape.rs`;
- any other project's preset tables (LibreOffice, Apache POI, OnlyOffice, docx4j and so on) and web copies of them.

Allowed: the ECMA-376 prose (the shape names in `ST_ShapeType`, the guide-language operators and built-in names),
Word itself, and our own measurements. If an excluded file is seen by accident, stop and report it. Guide names in our
entries are our own; only the adjust names a document may carry (`adj`, `adj1` .. `adjN`, `hf`, `vf`) are fixed by the
file format.

**Method, per batch of about 25 presets** (`fixtures/reference/drawing/presets/manifest.json` lists the 187 names in
batches `b01` .. `b08`, plus a five-preset `pilot`):

1. `pwsh -NoProfile -File tools/office/make-preset-probes.ps1 -Batch b03` (local only; needs Word). Stage 1 opens a
   one-shape-per-page document and reads `Shape.Adjustments` (count and default values) into `b03.handles.json`. Stage 2
   writes `b03.docx` with one preset per page, no fill, a 0.75 pt line, no text, anchored at (72 pt, 72 pt): four sizes
   at default adjusts (144x144, 288x96, 96x288 and 216x144 pt), then on 216x144 all adjusts set to 0, 30000, 100000,
   -50000 and 400000 (out of range), then one page per adjust set to 0 with the rest at Word's default. The script
   exports `b03.pdf` through `make-pdf-refs.ps1`, records every Word PID it starts in `b03.run.json`, and never touches
   another WINWORD process. Word refuses a whole file for one invalid page (an adjust name the preset lacks, a partial
   `avLst`); the script then opens each page alone, finds the adjust names (`adj`, `hf`, `vf` presets are remembered in
   `adjnames.json`), drops what Word still refuses (listed in `probes.json` and `run.json`) and exports again.
   Measured run times: pilot (5 presets, 39 pages) 20 s; `b01` (24 presets, 214 pages) 21 s, or 124 s when the
   adjust names have to be found.
2. `cargo test -p open365-drawing --test preset_probes -- --ignored write_word_paths` writes `b03.paths.json`: Word's
   stroked paths (moveto, lineto, curveto from the PDF content stream; `re` is expanded), in EMU relative to the shape
   box with y down. Author from those numbers, not from the PDF.
3. Write the entry in `geom/presets.rs`, as a function with a header comment recording what was measured (adjusts,
   defaults, clamp limits seen with out-of-range values, how the outline changes with size). Add it to `table()`.
4. `cargo test -p open365-drawing --test preset_probes -- --nocapture` renders our path at each probe's size and
   adjusts and compares it with Word's. The deviation is the symmetric Hausdorff distance between the flattened
   outlines (independent of start point, curve splitting and subpath order); a page passes at 0.25 pt or less. The report
   gives, per preset, `pass`, `FAIL` (with the worst page kind and the failing pages) or `missing` (not in the table,
   never a failure), then the coverage of the probed presets and of all 187. A preset also fails when its adjust count
   or defaults differ from Word's object model.
5. A preset that cannot reach 0.25 pt is a finding: record it in the header comment and raise it, do not loosen the
   tolerance. Page kinds that show no change (`scan-adj3` equal to the defaults) tell which adjust a clamp belongs to.

What the probes cannot see: text rectangles (no text), connection sites, handle positions (`Shape.Adjustments` gives
counts and defaults only), and paths that are not stroked (`stroke="0"` paths such as the shading parts of `cube`; use
`-Fill` and extend the comparison when a batch needs them). The tests also check that the harness can fail.

Pilot result (2026-10-12), all within 0.25 pt on every page: `rect`, `roundRect` (radius `ss * adj / 100000`, adj
clamped 0..50000), `ellipse`, `triangle` (apex `w * adj / 100000`, clamped 0..100000) and `rightArrow` (shaft
`h * adj1 / 100000` clamped 0..100000; head `ss * adj2 / 100000` clamped 0..`100000 * w / ss`).

Batch result (2026-10-12), all 187 presets authored and compared at 0.25 pt: 184 match Word on every probe page; 3 match
all but the known pages below. The comparison ignores dots (subpaths under 0.05 pt, or only zero-length segments) and
subpaths wholly off the page on both sides, because Word's PDF drops those inconsistently; angle adjust defaults are
compared modulo 360 degrees (Word reports 16200000 as -90). The arc converter draws a flat ellipse (one radius zero)
as Word does, using the visual angle as the parameter (measured on `blockArc` and `donut`), and sweeps a full 360
degrees.

Known unmatched pages (allow-list `KNOWN_UNMATCHED` in `tests/preset_probes.rs`; the test fails if one starts to pass).
Word's outline there comes from degenerate arithmetic (collapsed inner ellipses, zero-size heads) that the guides do not
reproduce:

- `circularArrow`: `adj-all-400000` (1 of 14 pages).
- `leftCircularArrow`: `adj-all-0`, `adj-all-30000`, `adj-all-100000`, `adj-all--50000`, `adj-all-400000`, `scan-adj5`
  (6 of 14).
- `leftRightCircularArrow`: `adj-all-0`, `adj-all-100000`, `adj-all--50000`, `adj-all-400000`, `scan-adj5` (5 of 14).

Formula choices the prose leaves open, to confirm in the oracle: division by zero gives 0, `at2` is normalised to
`0..21 600 000`, tint and shade mix in sRGB.

## Milestones

- **M0** (done, minus presets): crate, model, evaluator, display list, parsers.
- **M1** (done, hook pending in the preview) Sheets render of existing drawings. See below.
- **M2** Sheets Insert Picture, anchor modes, row/column shift.
- **M3** Sheets shapes and text boxes (hit tests, handles).
- **M4** Writer Insert Picture defaults.
- **M5** Writer shapes, wrap modes, z-order.
- **M6** Groups.
- **M7** VML read-only and effects.

## M1: Sheets render (2026-10-12)

- `open365-xlsx` reads a sheet's drawing part: `Workbook::sheet_drawing_objects(sheet)` returns
  `SheetObject { anchor: SheetAnchor, object: DrawingObject, drawing_part, z, target(rid) }` in z order, for
  `twoCellAnchor` (with `editAs`), `oneCellAnchor` and `absoluteAnchor`, looking through `mc:AlternateContent`.
  Objects go through `open365_drawing::parse_fragment`; the raw fragment is kept. `Workbook::drawing_theme()` gives
  the colour scheme and fill and line styles of the workbook theme part.
- Placement: `Anchor::rect_px_pt(ws, metrics)` positions an anchor with the grid's pixel geometry (columns and rows
  quantised to whole pixels at `metrics.dpi`, hidden and resized rows and columns included). `rect_pt` keeps stored
  row points (chart tests). The anchor holds the unrotated frame (checked with a 30 degree rectangle), so a shape's
  `xfrm` position and size are ignored on sheets; only rotation and flips are kept (`scene::place_in_rect`).
- `drawing::scene` resolves a display list: `fillRef`/`lnRef` against the theme's format scheme, colours, line
  widths, preset dashes, curves flattened. Gradients draw as their average colour, patterns as the foreground,
  picture fills are dropped, all with a diagnostic; text inside shapes is not drawn yet (M3).
- Painters: `ui-egui/src/grid_drawing.rs` (hooked into `paint_region`, between cells and selection, once per frozen
  pane) and `apps/preview/src/sheet_ext/drawing.rs` (`paint(ui, list, &g, model)`; the call in `doc::paint_sheet`
  belongs to the build steward). Charts use `open365-chart` render in the anchor rectangle; vertical (rotated)
  chart text and dashed lines are not drawn in the preview yet.
- Oracle: `tools/office/make-drawing-probes.ps1` (fixtures `drawing-basic`, `-editas`, `-hidden`, `-anchors`) and
  `tools/office/drawing-oracle.ps1` (`fixtures/reference/xlsx/drawing-*/drawing.json`). `tests/drawing_oracle.rs`
  compares every object rectangle with `Shape.Left/Top/Width/Height`: deviation 0.000 pt at the run's 120 dpi
  (limit 1 px). The oracle does not yet cover other dpi values or sheets with frozen panes.
- Not yet: moving objects when rows or columns change in a session (`editAs` shift is M2), selection and handles
  (M3), group and connector probes.
