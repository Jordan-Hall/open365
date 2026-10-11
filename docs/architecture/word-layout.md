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
| A cell that a vertical merge covers keeps its own top and bottom border at the edges of its row, and the first cell of a merge also puts its bottom border at the end of its first row: Word leaves that border's gap between two rows of the other columns (a 0.5 pt `insideH` between rows 6 and 7 of a masthead table where only the merged column had no explicit bottom border). Corpus evidence only (+54 lines over `ad265748` and one more template, no regression), no probe fixture yet. | corpus |
| Word snaps the clip rectangle of a cell, not its padding: `CellBox::pad_top` and `pad_bottom` are the distances from the 0.24 pt-snapped box to the clip rectangle `snap(y + pad_top)` .. `snap(y + height - pad_bottom)`. A 2 pt padding is no multiple of 0.24 pt. | `layout-trh-asym` 11 to 17 of 66, `layout-trh-pad` 100 of 100, `layout-table-basic` 138 of 138 |
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
`layout-img-tb` to the picture tolerance. The rules of `docs/research/word-float-placement.md` are implemented in `float.rs`:

- Each wrapping float excludes `[x - distL, x + w + distR)` over `[y - distT, y + h + distB)`. A line band loses those spans
  (`wrapText` left, right, both, largest); every free span that holds the next word becomes its own line on the same row.
  When no span fits, the row goes below the floats. Top-and-bottom floats push whole lines below them.
- Tight and through wraps follow the `wp:wrapPolygon` read from the raw anchor: the extent is the union of the polygon over
  the line band (`layout-float-wrap-tight`, 68 of 68).
- A float in the next paragraph is placed from that paragraph's top as laid out without it (kept in `anchor_top`), and its
  zone pushes the last lines of the paragraph above. Only when all lines still fit on the page.
- `allowOverlap="0"`: a later float moves right until its left edge meets the right edge of the earlier one it covers.
- Character-relative x is the left edge of the character before the anchor; line-relative y is the top of that line. Such
  floats are placed after their line and never move it.
- Right and bottom alignment subtract the `effectExtent` on that side.
- Shapes in `mc:AlternateContent` without text are kept as anchors (one character, wrapping); header/footer ones are not
  emitted as images.

Open: clamping at the
bottom margin (`layout-float-anchor-bottom`, `-anchor-nextpage`), the lookahead after an incremental restart.

## Section columns (`w:cols`; `props.rs`, `flow.rs`, `float.rs`)

Probes: `tools/office/make-cols-probes.ps1` (`fixtures/docx/layout-cols-*`). The oracle reads multi-column pages from the PDF in
stream order (Word writes a page column by column; lines of two columns share baselines).

| Rule | Evidence |
|---|---|
| Equal columns are `(text width - (n-1) x space) / n` wide at `i x (width + space)`; unequal ones take each `w:col` width and its own `w:space`. Text fills a column to the bottom margin, then the next column from the same top, then the next page. A paragraph laid out for one width is broken again for a column of another width. | `layout-cols-two/-three/-four/-line/-unequal/-unequal-two` (all lines) |
| A column break (`w:br type=column`) is a character on a line of its own at the end of the column; the text after it starts the next column (a page in a one-column section). | `layout-cols-break` |
| Section types: continuous sections start below the previous one on the same page (same page size), next-page, odd-page and even-page sections start a page (a blank page is inserted for the wrong parity), a next-column section starts in the next column. Space after the last paragraph and space before the first one still combine as the larger. | `layout-cols-continuous/-nextpage/-newcol` |
| A continuous break balances the columns of the section it ends, on the section's last page only: each column aims at `content height / n` and takes a line that starts before that share is used up (10 lines and two gaps over two columns: 6 + 4); the height grows until the rest fits in the other columns; widow, orphan and keep rules apply. The next section starts below the tallest column plus the space after its last paragraph (none when the column ends inside a paragraph). The last section of the document and sections before a next-page break are not balanced. | `layout-cols-balance/-balance-pages/-continuous/-mid` |
| `Column`-relative float positions use the column (`float-poscol`, 235 of 236). Floats wrap per column. | `layout-float-poscol` |

Footnotes in columns: done in Round 3 (below). Not done: the `w:sep` line is not drawn,
tables in unequal columns use the first column's width, documents with columns are always paginated from scratch (no incremental restart).

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

Body text wraps around square and top-and-bottom boxes through the same float code as pictures (`layout-txbx-wrap`, 65 of 66). The box text is not clipped in the PDF or the desktop view.

## Table row heights and section-break paragraphs (probes `layout-trh-*`, `layout-sect-*`; `tools/office/make-table-probes.ps1`)

| Rule | Evidence |
|---|---|
| An "at least" row is its height plus the top and bottom cell padding (`tblCellMar`, `tcMar`); an auto row is content plus padding. Word's clip rectangle of a cell is the cell inset by the padding, so the parity test compares `y + pad_top` and `height - pad_top - pad_bottom`. | `layout-trh-pad` (86 of 100), corpus +900 lines |
| An exact row is its height plus the **bottom** padding; the clip rectangle runs from the top padding to the exact height (`CellBox::pad_bottom` is the bottom padding as for any row). When the exact height does not exceed the top padding the clip is empty: Word emits no rectangle and draws no text, but the row still takes height + bottom padding (12/2 pt padding: exact 4 -> 6 pt, 8 -> 10, 12 -> 14, 12.5 -> 14.5 with a 0.5 pt clip). | `layout-trh-pad`, `layout-trh-asym` (66 of 66), `layout-trh-small` (210 of 210) |
| The vertical padding is the row's: every cell takes the largest top and bottom padding any cell of its row states (content position, vertical alignment, clip). | corpus (card templates) |
| A row taller than a page whose content fits is clamped to the page height, not split. | `layout-trh-tall` (36 of 36) |
| A paragraph that holds only a section break (continuous, next, even or odd page) takes no height and none of its spacing, never moves to another page, and the spacing around it passes through; Word's line of the paragraph before it ends after the break character. | `layout-sect-empty` (12 of 12), `layout-sect-tail` (242 of 242), `layout-sect-space` |

Open: space before at the top of a page after a next-page section break is ignored in `layout-sect-space` (Word, 3 pt before) but kept in the corpus
(`15d030a7`, 6 pt), so the engine keeps it (`hard_start`); the cause is unknown.

## Hyperlinks (`w:hyperlink`; `para.rs`, probes `layout-anc-hyperlink*`; `tools/office/make-anchor-probes.ps1`)

| Rule | Evidence |
|---|---|
| Word reads a `w:hyperlink` as a field: begin, code, separator, result, end. The hidden code is ` HYPERLINK "url"` (url from the relationship target) + ` \l "anchor"` + ` 	 "frame"` + ` \o "tooltip"` for the attributes present, + ` \h` when `w:history` is not true. Story offsets count 1 + code + 1 before the text and 1 after. | `layout-anc-hyperlink` (10 of 10), `-nohist`, `-hist0`; corpus 4376 to 4731 lines |

Header, footer, footnote and endnote parts have relationship ids of their own, so the URL (and with it the hidden field code length) is read from the part's own
relationships (`incremental.rs` `related_links`, `Furniture::links`, `NoteStore`): `layout-anc-hyperlink-hf` (12 of 12, all line ends).

Row-level content controls (`w:sdt` around `w:tc`, 13 in the corpus, `ad265748` has 4) count a start and an end character in Word: `flatten.rs` `row()` puts
`SDT_START` on the first paragraph of the first wrapped cell and `SDT_END` on the last paragraph of the last one (`crates/docx/tests/flatten_rows.rs`; corpus +26 lines).

Open (anchors): in `7fc3ccc4` (table 0, merged first cell) Word numbers the paragraph-mark line after an inline `wpg` group at @287
(x after the group, the baseline of the group) and the empty paragraph below it at @314, after the title paragraph, so the lines of a
merged cell are not in document order in the reference (possibly the oracle's pairing of empty paragraphs, not Word); not changed.
`ad265748`: the empty paragraphs of vertically merged, centre-aligned cells (@1243, @1410, @1426) are paired with other y values in the reference; not understood.

Floating tables (`w:tblpPr`; `table.rs` `TablePos`, `flow.rs` `place_float_table`, probes `layout-tblp-*` made by `tools/office/make-frame-probes.ps1`) and frames are implemented.

| Rule | Evidence |
|---|---|
| A table with `w:tblpPr` is out of the flow: the cursor does not move and the next paragraph (its anchor, skipping further floating tables) starts where it would have without the table. Text wraps like around a square float over `[left - leftFromText, right + rightFromText]` (`BothSides`) and `[top - topFromText, bottom + bottomFromText]`; the outer left edge of the table (half the left border outside the first grid line) is exactly the anchor, there is no cell-margin shift. | `layout-tblp-basic` 35/35, `-page` 60/60, `-yspec` 80/80 |
| Word writes a position one twip too large (`tblpX`/`tblpY` 0 as 1, 60 pt as 1201, 100 pt as 2001): an odd value loses that twip. A 0.05 pt shift moves cell edges across the 0.24 pt snap. | `layout-tblp-basic`, `-margin` |
| Omitted attributes: `horzAnchor` = text (the column), `vertAnchor` = **margin** (the other probes write `vertAnchor="text"` explicitly). `vertAnchor` text is the anchor paragraph's top, margin the top margin (or the margin box for `tblpYSpec`), page the page top; `tblpXSpec`/`tblpYSpec` left/top, center and right/bottom align the outer box in the base. | `layout-tblp-margin` 40/41, `-yspec` |
| A table whose outer width plus both distances leaves no room beside it in the column stays in the flow (the anchor paragraph goes below it either way; the corpus' full-width layout tables score 16 lines in the flow and 9 floating, `7e53ee0d`). | corpus |
| A table that covers an earlier floating table moves right until its text area meets the earlier one's (its left edge = earlier zone right + its own left distance), when either of the two has `tblOverlap="never"`: the later table's `overlap` setting does not release it. | `layout-tblp-never`, `-allow` (50/50 each) |
| A floating table anchored in the next paragraph reaches back over the lines of the paragraph before it (negative `tblpY`, margin or page anchors): the lines are broken again around it with the same lookahead as pictures (`ahead_floats`). | `layout-tblp-never` |
| The oracle reports one more clip rectangle per table: the outer box grown by the left and right text distances only (the top and bottom distances are not in it), `Page::table_wrap_zones`. Right aligned with `rightFromText` 0 it is 0.5 pt wider than that (`layout-tblp-right`, `layout-tblp-margin`: unexplained, 2 rectangles). | `layout-tblp-basic/-right2` |

A floating table that runs past the bottom margin breaks between rows (`layout-tblp-bottom` 97/97, `place_float_table`, `emit_float_rows`): the rows that fit stay (rows 1 to 5, 70.10 pt), the rest start the next page at the top margin (the part again begins with the table's top border, so it is 70.10 pt as well), and the anchor paragraph moves to that page with them, where its text wraps beside the second part (L22 and L23 start at 82.58 and are 281.64 from the left; L24 is back at full width). Not measured: a table of which no row fits, a row that could split itself, header rows repeated in the second part, and a vertically merged cell across the break (the table then stays whole). Frames and tables that are not as wide as the anchor's column in a section with columns are untested.

Frames (`w:framePr`, drop caps; `frame.rs`, `build.rs` `frame_paragraph`, `flow.rs` `place_frame_run`; probes `layout-frame-basic/-page/-dropcap/-nobdr/-bdr/-gap`, 38/38, 39/41, 17/17, 41/43, 40/42, 79/84). A frame paragraph, or consecutive paragraphs with equal `framePr` (a drop cap stands alone), is a float that leaves the flow like a floating table; frames with `yAlign="inline"` stay in the flow.

| Rule | Evidence |
|---|---|
| The text is laid out at `w:w`; without it at its natural width (a wrapped paragraph fills the column, a drop cap is as wide as its letter, 23.05 pt for a 55 pt "L"). | `-nobdr`, `-dropcap` |
| Position: x, y and the keywords as for tables (odd twip values lose one twip). The text box is placed horizontally (`xAlign` right puts its right edge on the margin), the clip rectangle vertically (its top is the anchor paragraph's top for `vAnchor` text, plus y). | `-basic`, `-page` |
| The clip rectangle is the text box grown by `hSpace` + (for a side with a `w:pBdr` element, whatever its style, `none` included: `w:space` + border width + 1.5 pt) left and right, and by `w:space` + border width above and below (no border element: 0; `vSpace` is not in it). The first line sits that far below the top. `hRule="exact"` fixes the clip height. | `-basic`, `-nobdr`, `-bdr`, `-dropcap` |
| Text wraps over the clip rectangle horizontally and the clip rectangle grown by `vSpace` vertically; the frame of the next paragraph reaches back over the lines of the paragraph above (same lookahead as pictures). | `-bdr` (last line of the paragraph above moves right), `-page` |
| Text is never put in a gap beside a float narrower than 18 pt (2-letter word: used at 18.5 pt, not at 15.5; "Harbour" needs its 37 pt). Applied to every float. | `-gap` |

Open (frames): clip rectangle edges differ by one grid step (0.24 pt) in some probes (`-gap` 5, `-nobdr` 1, `-bdr` 1, `-page` 1): Word's frame top/bottom come out about 0.02 to 0.2 pt lower than the engine's unrounded line heights give (a line height of 13.43 instead of 13.4277 fits the frame cases, but the same rounding breaks the table probes, so it is not applied); one frame line of `-bdr` is 0.56 pt off the screen-quantised reference. `dropCap="margin"` hangs the cap left of the column (unverified), style-defined `framePr` is not read, frames in table cells are ignored (`4e493df5`), wide frames that leave no room beside them and frame overlap are untested.

Floats in table cells (`table.rs` `layout_cell`, `FlowImage`; `float.rs` `Frame::cell`; probes `layout-float-incell`, `-incell-compat14` from `tools/office/make-float-probes.ps1`, 40/40 and 39/39; every cell has its own text so the oracle keys each line to its own offset, and the anchor sits after the cell text because the oracle's cell offsets come from `Range.Text`, which has no character for a floating anchor).

| Rule | Evidence |
|---|---|
| In a cell the cell stands in for the page: `margin` (and `column`) horizontal base is the cell's text area (right alignment puts the picture's right edge on the text right), `page` horizontal base is the cell box (text left less the cell margin), every vertical base (`page`, `margin`, `paragraph`) is the cell content top, `paragraph` is the anchor paragraph's top. Compatibility mode 15 ignores `layoutInCell`. | `layout-float-incell` 40/40 |
| A square float narrows the cell's lines beside it and rewraps them through `dynamic_lines`; the row grows only through the extra lines (row 3 of the probe 59.52 pt with the new texts). Later paragraphs of the cell see floats that still reach down. | `layout-float-incell` |
| Mode 14 or lower with `layoutInCell="0"`: the drawing is placed against the page (margin box x, `paragraph` y counted from the outer edge of the cell's top border, i.e. 0.5 pt above the in-cell base for a 0.5 pt border, also when right aligned) and does not wrap the cell text (`ObjectBox::page_anchored`, placed when the cell is drawn). | `layout-float-incell-compat14` 39/39 |
| Mode 14 or lower puts the left grid line of a left-aligned table one cell margin left of the indent and does not move it by half the border (text origin 72.0, cell boxes 66.96 / 266.88 on the 0.24 grid); mode 15 puts the border's outer edge on the indent. `Context::compat` carries `w:compatibilityMode` (a document without one counts as 12; the 17 corpus templates without it score the same). | `layout-float-incell-compat14` cells |

Not done: floats whose anchor is inside a nested table or a text box in a cell are laid out like any cell paragraph without row-height feedback, wrapping around out-of-cell drawings in mode 14, character- and line-relative placement in cells. The oracle (`Walk-Table`) adds each shape's `Anchor.Start` to `marks`, so a cell paragraph whose anchor precedes its text gets the right line offsets (`layout-float-incell-before`, 38 of 38; before the fix the anchor characters were missing from `Range.Text` offsets). Regenerating the 17 corpus templates with floating shapes in tables changed no non-empty cell line offset; other differences between the old and new references (empty-paragraph pairing, picture rectangles of `463abd5c`) are oracle drift, not this fix, so the old references were kept.

## Empty header and footer stories (`furniture.rs`; probes `layout-hdr-*` from `tools/office/make-header-probes.ps1`)

| Rule | Evidence |
|---|---|
| A section with no header or footer reference at all reserves nothing: the body starts at the top margin even when the header distance is larger (`layout-hdr-none`, `-near`, `-style`, `-normal`, `-big`, all lines). | probes (121/121, 121/121, 121/121, 61/61, 121/121) |
| A section that names two or more footer parts and no header part (`0a90ca9d`: default and first footers; `d52c20af`: even and default) keeps an empty paragraph of the `Header` style as its header: the body starts at `header distance + height of that paragraph` when that is lower than the top margin (10.8 pt for the 8 pt Header style of `0a90ca9d`, so 21.6 + 10.8 = 32.4; `d52c20af` 14.4 + 12.6 = 27). The same holds for a footer story when only headers are named. `Word.Headers(1).Exists` is true for these. | corpus +141 lines (`0a90ca9d` 3 to 84, `d52c20af` 2 to 62), no template lost |

Open: the rule is a fit, not a mechanism. `titlePg` alone is not the trigger (`65cf689e`, `0c8d491f`, `25409d45`, `b0e9c2cf`, `114ed224` lost all lines when it was). A section that inherits its stories and sets `titlePg` later (`4e493df5`, +59 lines when treated as implied) is not covered; a probe that writes a docx with exactly one footer part, two footer parts and `titlePg` is still to be made (COM writes both parts, so the file has to be patched).

## In-line x positions (per-character parity; `crates/layout/tests/char_x.rs`)

`char_x.rs` reads Word's own PDF (`fixtures/reference/pdf/<name>.pdf`, `tests/common/wordpdf.rs`: glyph origin = `Tm` origin + `/W` widths
+ `TJ` adjustments, characters through `ToUnicode`) and pairs every non-blank glyph of a page with the engine's by longest common subsequence in
reading order (case-insensitive: `w:caps` glyphs are capitals in the PDF). A glyph agrees when x is within 0.5 pt and the baseline within 3 pt.
Floors: `fixtures/reference/layout/charx-floor.toml` (bundled and `[system]`), `CHARX_DETAIL=<fixture>` lists bad rows, `CHARX_ROW=<page>:<y>` one row.
Before / after (system fonts, 443,196 Word glyphs): 93.6 % -> 94.7 %; layout-justify 82.8 % -> 87.2 %, layout-align 95.4 % -> 96.7 %, layout-tabs 63.5 % -> 65.2 %.

| Rule | Evidence |
|---|---|
| A justified line is stretched to `avail - 0.02 pt x spaces` (lines with at least 10 spaces): Word's lines end up to 0.6 pt short and the shortfall grows with the number of spaces. Empirical (0.01: 86.5 %, 0.02: 87.2 %, 0.035: 85.3 % on layout-justify); a rule `slack / (n + c)` fits worse. | `layout-justify`, `-align` |
| A tab is a break opportunity before it, not after it: `Twelve` + tab + `Thirteen` at the margin puts the tab at the start of the next line, which then advances to the next stop (108 pt). | `layout-tabs` (16 -> 17 of 18 lines; corpus unchanged at 5385 of 7998) |

Round 2 (justified breaks, tab leaders):

| Rule | Evidence |
|---|---|
| A justified line may squeeze its spaces by 0.24 x space width (was 0.2): per-candidate fit over 2,424 lines of `layout-justify`: 64 -> 48 wrong accept/reject decisions; the optimum is flat at 0.235-0.245. Word's own space is 2.4 pt (10 px of 0.24 pt) against the engine's 2.487, so 0.2 x 2.4 plus that offset is about 0.24. | `layout-justify` 87.2 % -> 92.4 %; other justified fixtures unchanged |
| Tab leaders are glyph runs of the tab's font on a grid of one pitch from the text-area left edge; pitch = advance of the leader character rounded to 0.24 pt ('.' 2.88, '-' 3.36, '_' 5.52 at Calibri 11), from the first grid point at or after the tab start to the last that ends by the tab target. Glyphs carry `ClusterKind::Other` with the character in `text_start` (`Line::leader_char`). | `layout-tabs` 65.2 % -> 72.8 % |

Open: the remaining 48 wrong decisions are mostly lines with ligatures and the `sound` / `of` case (Word rejects `of` at a shrink it accepts elsewhere); Word advances are whole 300 dpi pixels (round(units x ppem / upem), ppem = round(size x 300 / 72); 95 % of pairs match) but rounding every advance in the engine lowered parity (layout-align 96.7 % -> 83.8 %), so kerning/pair rounding is still not understood. The leader pitch in Word's PDF is 2.88 pt (the reader saw 2.78 until Round 3 fixed `Tc`).

Findings that did not become rules (so nobody retries the same thing):

- Word's glyph advances and spaces are whole multiples of 0.24 pt (two 600 dpi pixels): in the PDF `l`/`i` advance 22 px (design 21.1), `t` 30 (30.7), `h`/`o`/`u` 48, `e` 46,
  `g` 42 or 44, a natural space 20 px (design 20.7), and every justified space is 30/32/34/40 px with a few +-2 px outliers on a line. Rounding each advance to 0.24 pt in the
  engine (nearest, ppem = size x 600/72) made things worse (layout-wrap 90 % -> 72 %, layout-justify 82.8 % -> 80.6 %): spaces lose 0.7 px each and the same glyph gets 42 or 44 px
  depending on context, so the rounding is on positions or has a kerning/hinting part that is not understood. Distributing the justified slack in whole 0.24 pt units
  (equal, floor + remainder first, evenly spread) was worse too (57 %, 53 %, 79 %).
- Word's PDF writer drops `TJ` adjustments under about 3/1000 em, so a glyph's PDF position can differ from Word's layout by a fraction of a pixel; that noise bounds the metric.
- Line breaks in justified text: Word accepts lines the engine breaks early (229 lines in layout-justify, space compression of 0.2-0.36 of the space width) and rejects some the
  engine keeps (31 lines, e.g. `...fill with the sound` + `of`). 13,500 of the 18,900 failing glyphs of layout-justify are in the 194 rows whose break differs, so the break rule, not the spacing,
  is now the largest in-line loss. `SPACE_SHRINK` = 0.2 is not a threshold on shrink per space (overflow at the end of the word did not separate the two sets).
- Tab leaders are drawn by Word as glyph rows ('.' every 2.78 pt, '-' every 3.37 pt, starting a fraction of a point after the text) and are not drawn by the engine: 326 of 892 glyphs of
  layout-tabs are leader glyphs with no engine counterpart.
- Looked at in Round 3: list labels, mixed sizes and footnotes in columns (below). Not looked at: text boxes.

Round 3 (reader, list labels, footnotes in columns; system fonts, 443,196 Word glyphs): 94.7 % -> 96.3 % (bundled 93.6 % -> 93.8 %).

| Rule | Evidence |
|---|---|
| `wordpdf.rs` honours `Tc`, `Tw` and `Tz` (graphics state, saved and restored by `q` / `Q`). Word writes `Tc` (e.g. -0.0773) on list labels and on `w:spacing` runs; ignoring it put every later glyph of the `TJ` off. This is a reader fix, not an engine change: it raised `fields` 142 -> 352 of 353, `layout-tabs` 649 -> 872, `layout-caps` 2066 -> 2076, `layout-caps-sizes` 198 -> 206 (system). | all 29 PDFs that contain `Tc` |
| The one lowered floor, caused by the reader fix: bundled `layout-caps-sizes` 122 -> 119 of 208 (the earlier 122 counted glyphs that agreed only because `Tc` was ignored; the system-font figure rose). | `charx-floor.toml` |
| The space after a label (`w:suff` = space) is set in Arial at the label's size, whatever the label font: the PDF draws it in ArialMT and the text starts 3.12 pt after the label at 11 pt (Calibri's space would give 2.4 pt). | `layout-lists-suff` 1959 -> 1990 of 2015 |
| A list label is not kerned (`v.`, `r.`, `o.` keep their unkerned gap; kerning moved `.` by 0.5 to 1.2 pt). | `layout-lists-formats` 3808 -> 3816 of 3862 |
| Label x positions for left, centre and right alignment, the tab / space / nothing gaps and the hanging indent were already right (`layout-lists-style` 90.9 %, `-suff` 98.8 %, `-multilevel` 100 %); what remains there is 0.5 to 0.9 pt drift after the first 30 glyphs of a line (the kerning/rounding noise below), not label placement. | `CHARX_DETAIL` rows |
| Footnotes of a section with equal columns are set at the column width and balanced over the columns: whole notes in order, split so the tallest column is as short as it can be (6 one-line notes in 2 columns: 3 + 3). The note area (`NoteStore::area`) is that tallest column plus the separator, so the body gets the room back. Unequal columns still use the full width. | `layout-cols-footnote` 1446 -> 2212 of 2224 glyphs, lines 38 -> 62 of 63 |

Mixed sizes (`layout-mixed-sizes`, 96.9 %): the failing glyphs are the same sub-pixel drift (0.5 to 0.7 pt late in the line), baselines and the x of the size change agree; no rule to add.

Open after Round 3: Word's glyph advances are whole pixels but rounding in the engine still lowers parity (kerning-pair rounding not investigated this round); the 5385 -> 5396 of 7998 corpus lines are mostly cell rectangles, text boxes and `ca4e5536`-style titles (a Title paragraph inside a block `w:sdt` is left-aligned by the engine where Word centres it, x 72 against 217.9; not diagnosed). The ~120 dropped blank rows are an oracle-side pairing problem (they need Word's `Range.Information`); they cannot be recovered from the committed references.

## Superscript and subscript runs (`w:vertAlign`; `para.rs` `style_from`, `GlyphRun::vert_align`)

Measured from Word's PDF: the MS-template references with `th`/`st` superscripts (`114ed224`, `1907b7c8`, `33449d14`, `b97c34b7`, `c0651bb6`,
`e41daea3`) and a probe (`vert/probe.docx`, Arial, Calibri and Times New Roman at 10, 12 and 20 pt, exported through Word's PDF export; subscripts have no
fixture). Word snaps sizes and baselines to its 0.24 pt grid.

| Base size | Super size | Super raise | Sub size | Sub drop |
|---|---|---|---|---|
| 10 pt (PDF 10.08) | 6.48 | 3.60 | 6.48 | 0.96 |
| 12 pt | 7.92 | 4.08 | 7.92 | 0.96 |
| 20 pt (PDF 19.92) | 12.96 | 6.48 | 12.96 | 1.44 |

Rule used: size 0.65 of the run's size, superscript raised by 1/3 em of the unreduced size, subscript lowered by 1/12 em (all within one grid step of
the data; identical across the three fonts). The raise is `PlacedGlyph::y_offset`; the run keeps the full size for line metrics, so the line height does not
change. Not matched: Segoe UI in `c0651bb6` raises 4.08 pt at 10 pt (a font-specific value, probably the OS/2 superscript metrics), and Avenir Next LT Pro Light
shrinks to 6.0 pt rather than 6.5; both are left on the generic rule. `w:position` is still ignored. Note reference marks keep their own rule (`superscript`).

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

Oracle pairing of empty cell paragraphs (`word-layout-oracle.ps1`, `Pair-BlankRows`; done). A blank paragraph prints a row with no text, so
the old rule (the next unconsumed blank paragraph of any table) mis-paired rows whenever a drawn row belonged to no main-story table
paragraph (tables in text boxes, a picture paragraph) or the drawing order differed from the document order. Word draws the content
of one cell inside one clip rectangle in paragraph order, so now: the cell of a clip comes from its text rows; a blank row takes the first
unconsumed blank paragraph of that cell after the paragraph of the row before it and before the paragraph of the next text row; a clip with
no text takes the cell of the nearest blank paragraph by `Range.Information` 3, 5, 6 (not beyond 12 pt, else the row is dropped with a
warning instead of being paired with an unrelated paragraph). `Range.Information(6)` cannot be the key on its own: for the later
paragraphs of a tall cell Word returns the same wrong y (`65cf689e`: four paragraphs 70 to 400 pt apart all report 357.6, and
`Information(10)` stops at line 12). A paragraph whose only content is a floating shape's anchor (Range.Text `/`) or an inline object counts
as blank (`65cf689e` @1109: its row is the paragraph mark right of a 160 pt picture). Text rows prefer the cell of their clip, and a blank row
still tells which table is being drawn. `O365_ORACLE_DEBUG=1` prints the blank rows, their group and the cell's paragraphs for each unpaired row.

Evidence for the regenerated references. The committed references predated several oracle features (cell rectangles without picture clips,
`images`, baselines for some pages), so a control run was made: the HEAD oracle on the same Word build scores 5111 of 8121 lines, 14 templates
at 100 %. The new pairing scores 5385 of 7998, 17 templates (new at 100 %: `67792a9e`, `741ee369`, `9f1fce92`, `a2a7720e`, `b0e9c2cf`); the
pairing alone is +274 matched and -123 reference lines against the control. Reviewed by hand: `65cf689e` ref @100 moved from y 538.66 (a row of
a table in a text box) to 274.61 (the engine's 274.56; Word's own position 262.8 plus the 11.8 pt from line top to baseline), 164/502 -> 231/498;
`741ee369` 200/232 -> 232/232; `7fc3ccc4` 433/546 -> 534/554; `463abd5c` 94/130 -> 120/130. Reference lines that disappeared are rows no
main-story paragraph accounts for. Floors that fell against the old floor file (`15d030a7`, `494a603b`, `4b9c352f`, `4c5d5d40`, `5ff67a28`,
`8583a4ca`, `938d0510`, `aae2ed66`, `ae0aba0c`, `d52c20af`, `ee25c76b`, `ee8d0c24`) fall mostly to the older references' drift (picture and
cell rectangles now in the reference, 20 of the 21 lines of `ae0aba0c`) or to removed mis-paired rows; the file is re-baselined to the new run.
Probes: of the 25 probes with table rows, 22 regenerate identically; `layout-table-flow` moves one blank row from @278 to @288 (the paragraph
after the nested table); `layout-table-width` differs only by 4 cells at the page edge (oracle drift, not taken); `layout-table-pages` timed
out at 240 s and was not regenerated.

Still open:

- `ae0aba0c`: the reference pairs a page's PDF rows with the listed lines in sorted order, and one row (y 8.88, an empty header paragraph Word
  prints but does not list) shifts every later pairing by one. The engine's 45.12, 664.32 and 750.48 are the plausible values. A monotone
  alignment that checks `Range.Information(6)` against each row would fix it. `aae2ed66` is 82/84 now (the box line and the last body
  paragraph, both `U+001F`, were swapped in the old reference).
- `ad265748`: the page-1 sidebar sits 3.9 pt above Word's (a merged-cell row height, not pairing). `7fc3ccc4` is 534/554 with cell rectangles
  0.02 to 0.3 pt off. `4ebce9eb`, `54d00e5f`: the remaining failures are cell and frame rectangles, not order.
- Blank rows of tables inside text boxes are dropped (the oracle does not list those paragraphs); a repeated header row of blank cells is paired
  by position only.
