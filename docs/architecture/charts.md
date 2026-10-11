# Charts (crates/chart)

Status: first slice. Clean-room: rules come from Excel's observable answers via COM, never from Microsoft code.

## Layers
- `crates/xlsx/src/drawing.rs`: read-only `Workbook::sheet_charts`, anchors (two-cell, one-cell, absolute), `Anchor::rect_pt`.
- `open365-chart::ChartSpace`: typed view over the lossless XML tree of `c:chartSpace`; `to_bytes` writes the tree back, so unknown elements survive. Typed: bar/column (clustered, stacked, 100%), line, area, pie, doughnut, scatter; series refs and caches, titles, legend, axes, data labels, manual layout.
- `resolve` + `CellSource`: values from the workbook's stored cells (`WorkbookCells`) or any source (calc engine later); falls back to the cache.
- `render` gives a `DisplayList` (rects, paths, text runs with font and size) at the anchor size. Text is measured through `TextMeasure`; tests use bundled Carlito (Calibri-compatible).

## Oracle (tools/office/chart-oracle.ps1)
Own Excel instance, AutomationSecurity=3, quit in `finally`. Modes: `-Fixtures`, `-Layout`, `-Axes [-Seed n]`, `-Sweep`.
Fixtures force the chart font to Calibri: the 2023 theme default (Aptos Narrow) cannot be bundled.

## Axis scaling (crate::axis)
Matched exactly (1e-9 relative) on the 306-case generated corpus plus 40 scatter X axes, and on the 780-case plot-size sweep. Rules are in the module docs. Limits: the interval budget was fitted to these data; log axes, date axes and partially fixed axes are not verified; 10 pt text only was measured (other sizes scale linearly, unverified). All-negative data move category labels into the plot; the budget then uses the length without them.

## Layout
`tests/layout_fixtures.rs` compares plot inside/outer, legend and title rectangles at +-1 pt: 10 fitted fixtures 140/140, 3 later fixtures (`chart-holdout-*`) 44/44 after one refit of legend constants (first, un-refitted pass: 31/44, so those constants are in-sample now). Not covered: manual layouts, overlay legends, multi-line titles, rotated labels, data-label collisions, long category labels (no wrapping/rotation), combo charts, secondary axes, 3-D and other chart types.
Coordinates: Excel reports rectangles in a chart area 5 pt smaller than the object; the display list uses those coordinates 1:1 (assumption, not pixel-verified).

## Next
Wire into the grid and PDF; theme colours; log/date axes; hold-out axis corpus (`-Axes -Seed 7`).
