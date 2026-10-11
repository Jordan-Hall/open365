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
| A face whose `OS/2` `fsSelection` bit 7 (USE_TYPO_METRICS) is set takes its line from the typographic metrics: ascent `sTypoAscender + sTypoLineGap` (gap above), descent `-sTypoDescender`. Gabriola 24 pt gives 33.2 above and 7.6 below the baseline; Aptos, Cascadia Mono and Bahnschrift pitches agree (Word's PDF draws those three as pictures, so only screen tops and picture rectangles exist for them). | `layout-face-typo`, `layout-face-multiple`, Aptos templates in the corpus |
| Multiple spacing 1.3 and 2.0 at 10 and 24 pt in Gabriola, Calibri, Cambria and Arial follows the rule above. | `layout-face-multiple` (61 of 66 lines) |

## Tables (`table.rs`, `flow.rs`)

| Rule | Evidence |
|---|---|
| Cell edges lie on the 0.24 pt grid (rounded to nearest, halves up); the cell is drawn between the borders, the grid line in the middle of each border. | `layout-table-*` |
| The inset of a cell is half the wider border that meets at its grid line **in its own row**, not in the whole table. | `layout-table-flow` (header row) |
| A table style's cell borders `left` and `right` name the region's outer edges; the edges between cells use `insideV`. A header row with `insideV` nil has no vertical borders between its cells. | `layout-table-flow` |
| The empty paragraph after a nested table at the end of a cell takes no height. | `layout-table-flow` |
| A composite cell border (`thickThinSmallGap`, `double`, ...) takes its full extent (`border_extent`) in the half-border inset of the cell, not only its `w:sz`: the first row of a masthead table with a 3 pt `thickThinSmallGap` top border sits 1.5 pt lower in Word than with `w:sz`. Corpus evidence only (+41 lines in one template, 1385 -> 1426 of 8118, no fixture changed); no probe fixture yet. | corpus |

## Caps and small caps (`para.rs`)

| Rule | Evidence |
|---|---|
| `w:caps` shapes the capitals of the text at the run's size; clusters map back to the original characters. | `layout-caps` |
| `w:smallCaps` shows lower-case letters and spaces as capitals of `round(0.8 x size, nearest 0.5 pt)` (8 -> 6.5, 10 -> 8, 11 -> 9, 12 -> 9.5, 14 -> 11, 18 -> 14.5, 24 -> 19, 36 -> 29, and 13 other sizes). Capitals, digits and punctuation keep the full size. The line is sized for the full font. | `layout-caps`, `layout-caps-sizes`, `layout-caps-chars` |

## Paragraph borders (`para.rs`, `flow.rs`)

| Rule | Evidence |
|---|---|
| A top or bottom border adds its extent plus `w:space` to the paragraph: above the first line (after the space before), below the last line (before the space after). Left and right borders hang outside the text and change neither position nor wrapping. | `layout-pbdr-edges` |
| Extent of a rule: single, dotted and dashed styles the width; double 3x; triple 5x; thin-thick and thick-thin small gap width + 1.5 pt; thin-thick-thin small gap width + 3 pt; medium gap 2x; large gap width + 2.4 pt (thin-thick-thin 2x + 3 pt). Approximate for the large gaps (the layout-pbdr-styles drift is under 1.2 pt per paragraph). | `layout-pbdr-styles` |
| Consecutive paragraphs with identical `pBdr` and identical left and right indents form one box: the top edge once, the bottom edge once, nothing between unless `w:between` is set, then `width + 2 x space` is added to the usual gap (the larger of space after and before). Alignment, width, distance or indent differences end the group. | `layout-pbdr-groups` |
| Rule geometry (`rules.rs`, `Page::rules`, oracle `-Rules`, scored by `tests/rules_parity.rs`). Every edge lies on the 0.24 pt grid (nearest, halves up) in the PDF frame (x / 1.0004, y / 1.0003). A bar is `floor(width / 0.24)` units thick, at least one. The inner edge of a left rule is `text left - space - 1.44`, the inner edge of a right rule `text right + space + 1.44` (a missing side counts as space 0); top and bottom rules span from the outer left edge to the outer right edge. A side rule runs the whole box, top rule to bottom rule. Top rule starts at `first line top - space - extent`, bottom rule at `last line bottom + space`; a between rule sits `space + extent` above the next paragraph's first line. | PDF rectangles of `layout-pbdr-edges`, `-styles`, `-groups`, `-widths` |
| Composite styles are bars (top to bottom, the same for top and bottom borders; units of 0.24 pt): double and triple: `w` bars with `w` gaps; small gap: thick `w`, thin and gap 0.75 pt; medium gap: thin and gap `w / 2`; large gap: thick `max(w, 1.5)`, gap `w`, thin 0.75 pt. `thinThick*` draws thick first, `thickThin*` thin first. Word raises `w:sz` of the small-gap styles to 2.25 pt in the file. | `layout-pbdr-styles` |
| Paragraph borders also apply inside table cells (`layout_cell`), headers and footers (`furniture::place`) with the same grouping rule. | `layout-pbdr-ctx` (lines 33/33) |

## Multiple spacing at a page end

A line under multiple spacing above 1 stays on the page when its text (ascent and descent) fits,
even if the extra spacing below it does not (`layout-face-multiple`, Arial 10 pt, multiple 2.0).

## Pictures (`float.rs`, `flow.rs`)

Inline and floating placements match `layout-img-inline`, `layout-img-none`, `layout-img-square` and
`layout-img-tb` to the picture tolerance. Wrap distances and anchor positions have no dedicated probe yet.

## Content controls (`w:sdt`; `para.rs`, `incremental.rs`, `crates/docx/src/flatten.rs`)

| Rule | Evidence |
|---|---|
| A content control, inline or block, lays out like plain content: its `w:sdtContent` is part of the line, a placeholder (`showingPlcHdr`) is ordinary text in the control. The typed model has `Block::Sdt` (block-level controls) and an `sdt`/`sdtContent` `Group` (inline ones), both round-tripping `w:sdtPr` raw, so `Body::paragraphs`, `Document::paragraph` and `Paragraph::text` cover control content and `Line::paragraph` is a `Document::paragraph` index; `flatten_content_controls` unwraps controls for layout (only paragraphs of row-level controls stay unaddressable). | `layout-sdt` (19 of 19) |
| The start and the end of each control count as one story character each (`Range.Start`, line ranges, the next paragraph's start): `Alpha beta ...` with one inline control is 36 + 2 characters; a block control around one paragraph adds 2; nested and adjacent controls add 2 each. A line that begins at the paragraph start of a control begins on the start character. | `layout-sdt` |
| In table cells the oracle reads text from the PDF and used `paragraph start + text index` for a line, which ignores the control characters; the oracle now adds them (`marks` in `Walk-Table`). | `layout-sdt`, corpus |

## Text boxes (`wps:txbx`; `crates/layout/src/textbox.rs`, `Page::frames`)

| Rule | Evidence |
|---|---|
| A text box is Word story 5, one per box, offsets from 0; `StoryRanges(5)` lists them in anchor order (document order of the anchoring paragraphs). The oracle gives each box line a `frame` (index in that list) and pairs the PDF rows of the box's clip rectangle with the box's lines in order (a glyph the ToUnicode map lacks decodes as a space, so a row may lack its first or last character). `FrameBox::id` is the same index. | `layout-txbx-basic` (22 of 22) |
| The anchor counts as one character of the body paragraph (`Body paragraph one` is 38 + 6 characters for six boxes). | `layout-txbx-basic` |
| The `w:txbxContent` is laid out as its own story at shape width less `lIns` and `rIns` (defaults 7.2 pt; 3.6 pt for `tIns`/`bIns`); the content area starts `lIns`, `tIns` inside the shape. `anchor` `t`/`ctr`/`b` places the stack at the top, centred, or at the bottom of the shape less `tIns` and `bIns`. | `layout-txbx-basic` |
| `wrap="none"`: lines never wrap; the clip rectangle is `lIns + widest line + rIns` wide. | `layout-txbx-basic` (Zeta) |
| The clip rectangle Word draws is the shape inset by half the outline width (`a:ln/@w`) all round and by `tIns`/`bIns` vertically; `spAutoFit` does not resize the shape on open (Epsilon keeps its 30 pt clip while its third line falls outside). | `layout-txbx-basic` clip rectangles |

Not matched: body text wrapping around square-wrapped boxes (`layout-txbx-wrap`, 21 of 66: the lines beside a box are placed below it, and the baselines of that page are unpaired in the reference); the box text is not clipped in the PDF or the desktop view.

## Not matched yet

- Cell padding (`tblCellMar` top and bottom): in `layout-table-basic` Word's clip rectangle is the cell inset
  vertically by the padding (4 cells; they still differ by 0.10 pt in x), but in the template corpus the same
  inset breaks more cells than it fixes, so `CellBox::pad_top` and `pad_bottom` are reported and the parity test
  compares the full box.
- Paragraph border rules: 6 of 50 rectangles of `layout-pbdr-edges` are 0.24 pt off (the grid rounding of the top rule
  after a 4 pt space and of some bottom rules: Word rounds from the snapped baseline of the previous or last line, not from
  the exact bottom), groups, cells, indents and page breaks (`layout-pbdr-groups`, `-ctx`, `-pages`) are only partly
  matched (see `rules-floor.toml`); `w:bar`, hanging-indent left edges, the vertical order of thin and thick bars in
  side rules and the box across a page break are unverified. Text boxes (`wps`, `wpg`, VML) are not started (probes `layout-txbx-*` have references only).
- Auto-fit tables wider than the text area (`layout-table-width`, 4 lines and cells).
- The zero-height line of the empty paragraph after a nested table (`layout-table-flow`, 1 line).
- Contextual spacing (`layout-indents-spacing`, 7 lines) and the Caladea/Cambria vertical metric difference in bundled mode.
- The cell tolerance in `word_parity.rs` (0.05 pt) remains: the PDF scale constants leave about 0.02 pt of rounding.

## Microsoft template corpus (`crates/layout/tests/ms_template_parity.rs`)

Local references for the 74 public .dotx templates (gitignored; see `docs/testing/ms-template-corpus.md`) are scored in
system-font mode; counts live in `fixtures/reference/ms-templates-layout-floor.toml`. Lines, cells and pictures matched:
799 of 8118 (4 templates at 100 %) before, 979 of 8118 (5 at 100 %) after the USE_TYPO_METRICS rule and the PostScript-name
family match for "Aptos Display". Largest causes still open, by lines affected:

- Paragraph borders (`w:pBdr`, about 30 templates): the border width plus `w:space` is added above or below the paragraph
  (a Heading 1 with a top border, space 11, sits 11.33 pt lower in Word). `open365-docx` does not resolve `pBdr`.
- `w:caps` and `w:smallCaps` (most templates): the layout measures lower-case glyphs, so widths and line breaks differ.
  `RunLayoutProps` has no caps flag.
- Text boxes, drawing groups, floating shapes and content-control placeholders (`wps`, `wpg`, `txbxContent`, `sdt`): their
  text is not in the main-story line list the way Word reports it; most of the worst templates are flyers and brochures.
- Columns, frames (`framePr`), floating tables (`tblpPr`) and page borders.
- The last line of a page when multiple spacing adds space below the baseline: Word keeps the line when the text part fits
  (`layout-face-multiple`, Arial 10 pt, multiple 2.0, 5 lines); the engine moves it to the next page.
- The `tblCellMar` padding and contextual spacing residue listed above is unchanged.
