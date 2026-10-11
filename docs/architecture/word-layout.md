# Word layout rules measured so far

Every rule below was measured against Word 16 with `tools/office/word-layout-oracle.ps1` and is
scored by `crates/layout/tests/word_parity.rs` (floors in `fixtures/reference/layout/parity-floor.toml`).
Fixtures are made by `tools/office/make-layout-fixtures.ps1`. The code is in `crates/layout/src`.

Units are points. Baselines come from Word's 600 dpi PDF and sit on a 0.24 pt grid.

## Line metrics (`metrics.rs`)

| Rule | Evidence |
|---|---|
| Ascent and descent are the OS/2 `usWin*` values. GDI external leading, `max(0, hheaGap - ((winA + winD) - (hheaA + hheaD)))`, is added above the ascent. Arial and Times New Roman gain 67 and 87 units; Calibri, Cambria, Courier New and Segoe UI gain none. | `layout-img-face` (system fonts) |
| An inline picture stands on the baseline and has no descent. A line holding only a picture and the paragraph mark has no descent. | `layout-img-inline`, `layout-img-line` |
| The run that holds a picture sizes the line like a text run: the ascent becomes `max(text ascent, picture height, run font height - text descent)`, the descent stays the text's. A 12x8 picture in a 24 pt Calibri run among 11 pt text gives 29.3 - 2.97 = 26.33 above the baseline. | `layout-img-line`, `layout-img-inline` (last line) |
| Multiple spacing m >= 1 adds `(m - 1) x tallest font height` (text or picture run) below the baseline. | `layout-img-spacing` |
| Multiple spacing m < 1 cuts the text ascent by `(1 - m) x text ascent` and scales the descent by m. A picture taller than the text keeps its height. | `layout-img-spacing` |
| "At least" puts the extra above the baseline and ignores the picture's run font (the picture height still counts). | `layout-img-spacing` |
| "Exactly" puts the baseline at 80 % of the line height, whatever the font, size or picture. | `layout-img-spacing`, probes in 5 faces and 3 sizes |
| A paragraph mark sizes its line only when nothing else is on it. | `layout-marks` |

## Tables (`table.rs`, `flow.rs`)

| Rule | Evidence |
|---|---|
| Cell edges lie on the 0.24 pt grid (rounded to nearest, halves up); the cell is drawn between the borders, the grid line in the middle of each border. | `layout-table-*` |
| The inset of a cell is half the wider border that meets at its grid line **in its own row**, not in the whole table. | `layout-table-flow` (header row) |
| A table style's cell borders `left` and `right` name the region's outer edges; the edges between cells use `insideV`. A header row with `insideV` nil has no vertical borders between its cells. | `layout-table-flow` |
| The empty paragraph after a nested table at the end of a cell takes no height. | `layout-table-flow` |

## Pictures (`float.rs`, `flow.rs`)

Inline and floating placements match `layout-img-inline`, `layout-img-none`, `layout-img-square` and
`layout-img-tb` to the picture tolerance. Wrap distances and anchor positions have no dedicated probe yet.

## Not matched yet

- Cell padding (`tblCellMar` top and bottom): Word's clip rectangle is the cell inset vertically by the padding
  (`layout-table-basic`, 4 cells); the engine reports the full cell and counts the padding twice with space after.
- Auto-fit tables wider than the text area (`layout-table-width`, 4 lines and cells).
- The zero-height line of the empty paragraph after a nested table (`layout-table-flow`, 1 line).
- Contextual spacing (`layout-indents-spacing`, 7 lines) and the Caladea/Cambria vertical metric difference in bundled mode.
- The cell tolerance in `word_parity.rs` (0.05 pt) remains: the PDF scale constants leave about 0.02 pt of rounding.
