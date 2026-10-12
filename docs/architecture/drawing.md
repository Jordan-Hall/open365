# Shared drawing layer (`open365-drawing`)

Status: M0 done except the preset table (blocked on a licence decision, below). Plan decided by the lead on
2026-10-12.

One crate, pure data, no I/O, used by Writer and Sheets for pictures, shapes, text boxes and groups.

## Modules

| Module | Role |
|---|---|
| `model` | `DrawingObject { id, name, descr, hidden, xfrm, kind, raw }`; `Kind` = `Shape`, `Picture`, `Group`, `Chart`, `Unknown`. `Xfrm` in EMU (off, ext, rot in 60000ths of a degree, flips). `RawXml` = verbatim source. |
| `geom` | `ShapeDef` (adjust values, guides, paths, text rect); guide-formula evaluator (`*/ +- +/ ?: abs at2 cat2 cos max min mod pin sat2 sin sqrt tan val`, built-in guides `w h l t r b hc vc wd2..wd32 hd2..hd8 ss ls ssd2..ssd32 cd2 cd4 cd8 3cd4 3cd8 5cd8 7cd8`); path commands `moveTo lnTo arcTo quadBezTo cubicBezTo close` (arcs become cubics), fill modes, stroke flag. `PresetSource` trait, `Builtin`, `MapPresets`. |
| `geom::presets` | Slot for the generated preset table. Empty today. |
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

## Open: preset geometry data

The 187 preset shapes were to be generated from ECMA-376 Part 1 `presetShapeDefinitions.xml`. Checked on
2026-10-12: neither the 4th (2012) nor the 5th (2016) edition, nor the annex zip, carries a copyright notice that
grants rights to reuse the file (the 4th edition PDF says only "(c) Ecma International 2012"; the 5th has none).
Ecma's policy says a copyright notice permits copying a standard but not extracting portions of it. The v3 text
policy (December 2025) allows implementing by copy and paste, but that notice is not on these files. The owner
decides; until then nothing is embedded or generated from that file, and `geom::presets::table()` is empty.
Everything else works with hand-written or custom geometry (`a:custGeom`), and tests use geometry written from the
operator descriptions. When the table lands, the parity tests against `crates/pdf/src/shape.rs` (and finiteness,
closure and bounds checks over all presets) are added with it.

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
