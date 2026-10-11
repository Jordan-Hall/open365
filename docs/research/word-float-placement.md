# Word floating picture and shape placement

Status: measured 2026-10-11 against Microsoft Word 16.0.20430 (Windows, Print Layout), Letter page, 72 pt margins, Calibri 11 pt,
13.44 pt line pitch, 6 pt space after. Clean-room: every fixture is generic text plus solid-colour pictures and autoshapes made by
`tools/office/make-float-probes.ps1`; reference data is Word's own output from `tools/office/word-layout-oracle.ps1`
(each document gets a `floats` array: the object-model placement, the anchor position, and the final page rectangle taken from the
page's shape rectangle and refined to the exact value from the 600 dpi PDF picture or fill; `around` lists the body lines nearby).
Fixtures: `fixtures/docx/layout-float-*.docx` (20), references `fixtures/reference/layout/layout-float-*.json`.
Location note: the brief named `fixtures/layout/`, but every parity test reads `fixtures/docx/<name>.docx` for each reference JSON, so
the probes live there.

## Specification

ECMA-376 Part 1 (5th ed.), 20.4 "DrawingML - WordprocessingML Drawing": `wp:anchor` is 20.4.2.3 (attributes `distT/distB/distL/distR`,
`simplePos`, `relativeHeight`, `behindDoc`, `locked`, `layoutInCell`, `allowOverlap`; children `simplePos`, `positionH`, `positionV`,
`extent`, `effectExtent`, one wrap element, `docPr`). Related elements: `positionH`/`positionV` (20.4.2.10/20.4.2.11), `align` and
`posOffset` (20.4.2.1/20.4.2.12), `effectExtent` (20.4.2.6), `wrapNone`, `wrapSquare`, `wrapTight`, `wrapThrough`,
`wrapTopAndBottom` (20.4.2.15-20.4.2.19, `wrapText` = ST_WrapText 20.4.3.7). Simple types `ST_RelFromH`/`ST_RelFromV`
(20.4.3.4/20.4.3.5) and `ST_AlignH`/`ST_AlignV` (20.4.3.1/20.4.3.2). Sub-clause numbers other than 20.4.2.3 are from memory: verify
against the spec text before citing them elsewhere. The spec says nothing about how a collision, a push-down or a polygon is resolved;
everything under "Rules" is measured.

COM to XML mapping used by the probes (confirmed by reading the saved XML): `WrapFormat.Type` square 0 `wrapSquare`, tight 1
`wrapTight`, through 2 `wrapThrough`, 3 `wrapNone` with `behindDoc=0`, top-and-bottom 4 `wrapTopAndBottom`, 5 `wrapNone` with
`behindDoc=1`. `Side` 1/2/3 = `wrapText` left/right/largest. Word's default wrap distances are 0 top/bottom and 114300 EMU (9 pt)
left/right. A new picture has `layoutInCell=1`, `allowOverlap=1`.

## Rules

Evidence tables give the input (as stored) and Word's measured result in points from the page top-left. Confidence: H = every sample
agrees, M = agrees but few or indirect samples, L = inferred.

### 1. Position resolution (H)

| positionH relativeFrom | x of the float's left edge | sample (offset 20) |
|---|---|---|
| page | offset | 20 |
| margin | left margin + offset | 92 |
| column | left edge of the column holding the anchor + offset | 92; in column 2 (starts 324): 344 |
| character | left edge of the anchor character + offset | 92 (anchor at line start) |
| leftMargin | page left + offset (the area 0..72) | 20 |
| rightMargin | right margin edge (540) + offset | 560 |
| insideMargin / outsideMargin | on an odd page: inside = leftMargin, outside = rightMargin | 20 / 560 |

| positionV relativeFrom | y of the float's top edge | sample (offset 30) |
|---|---|---|
| page | offset | 30 |
| margin | top margin + offset | 102 |
| paragraph | top of the anchor paragraph's first line (after space before) + offset | para top 297.42: 327.42 |
| line | top of the line holding the anchor + offset | line top 131.71 offset 10: 141.71 |
| topMargin | offset (area 0..72) | 30 |
| bottomMargin | 720 + offset | 750 |

Fixtures: `layout-float-posh`, `-posv`, `-posv-line`, `-poscol`.

Alignment (`wp:align`), all H: left/right/center within the reference box, extents = `extent` plus `effectExtent` on the right and
bottom.

| input | result |
|---|---|
| page, center, w42 | x 285 (= (612-42)/2) |
| margin, center, w45 | 283.5 |
| rightMargin, center, w50 | 551 (= 540+(72-50)/2) |
| leftMargin, center, w51 | 10.5 |
| page, bottom, h24 | y 768; bottomMargin center 744; margin center 384; margin bottom 696 |
| column, right, w63 (column 2) | 477 (= 540-63) |

Right/bottom alignment subtracts the `effectExtent` r/b. Word wrote `effectExtent r=6350` (0.5 pt) on exactly the pictures with
x = expected - 0.5: w43 (568.5), w46 (493.5), w49 (281.25 centre, half), w61 (226.5), w64, w100 in some probes (439.5); pictures with
`r=0` land exactly (w110: 430.0). So `x = refRight - w - effectExtent.r` (H for the 7 samples checked in `layout-float-align` and
`-poscol`). Why Word writes a 0.5 pt effect extent for some widths is open; the engine must read `effectExtent` rather than assume 0.

Offsets are applied to the reference box origin whether or not the result stays on the page: an offset below the bottom margin is
kept (in-front float at paragraph-relative +80 sat at y 708.8, bottom at 768.8, inside the page). Fully off-page clamping is open
(`layout-float-anchor-bottom` has a +200 offset float that Word moved to y 660 instead of 828.8, see below).

### 2. Wrap rectangle and line displacement (H for square)

The exclusion zone of a square float is `[y - distT, y + h + distB)` vertically and `[x - distL, x + w + distR)` horizontally. A body
line is displaced if its box (top to top+13.44 for 11 pt Calibri) intersects the vertical zone, even by 0.19 pt; lines outside are
untouched.

| distT/B/L/R | float | first/last wrapped line (box) | right segment starts at |
|---|---|---|---|
| 0 | y 72..132, x 222..322 | 72..139.2 (last line top 125.8 < 132) | 322 |
| 6 | y 251.13..312.13 | 251.1..318.3 (zone end 318.13) | 328 |
| 12 | y 430.27..492.27 | 410.98..510.8 (zone start 418.27) | 334 |
| 24 | y 636.25..699.25 | 603.5..703.6 (zone start 612.25) | 346 |
| B20 only | y 251.13..312.13 | wrapped through the line 324.3..337.8 (zone end 332.13); next line 337.8 is full width | 322 |
| L30 only | x 222..322 | left segments end about 17 pt earlier (last char at 172.8-187.8 against 205) | 322 |
| R30 only | y 622.83..685.83 | as above | 352 (first ink 352.7) |

Horizontal: a wrapped line has a left segment ending at or before `x - distL` and a right segment starting at `x + w + distR`
(first ink = start + the glyph's left bearing, 0.68 pt for "A"/"c"). A segment is dropped when the paragraph ends.

Side (`wrapText`): `left` text only left of the float, `right` only right (start 341.2 for float at 222..332 = 332+9+bearing),
`largest` only the wider side (float 172..292: text only right). `bothSides` with a float flush to the margin: single segment on
the open side (left-aligned float: all lines start at 181.5). H (3 samples each).

Anchor position does not matter to the zone: the zone applies to every line of every paragraph in the vertical range, including the
last line of the paragraph before the anchor (`layout-float-dist` float 4, `layout-float-wrap-topbottom` float 2). H.

Float y is computed from the anchor paragraph's top in the layout without the float's own push-down (see 3).

### 3. Top and bottom (H)

Lines intersecting `[y - distT, y + h + distB)` move whole to `y + h + distB` (quantised to the 0.24 pt grid); the span stays empty.

| input | result |
|---|---|
| float at paragraph top (y 131.71), h60, dist 0 | first line of anchor paragraph at 192 (= 191.71) |
| float h50 anchored in para 5, y 357.42, distT10 distB20 | last line of para 4 (box 338..351.4 meets the zone 347.4..427.4) moves to 427.2; para 5 first line then 447; float stays at 357.42 |
| float (100,30) offset | y = para top + 30 = 642.55, lines pushed below its bottom |

So the float is placed from the paragraph top before the push, not from the pushed paragraph. Left/right distances are unused.

### 4. Behind, in front, none (H)

`wrapNone` with `behindDoc` 0 or 1: no line is displaced (0 of 22 + 33 + 44 + 33 lines had a second segment or shifted ink).
Position rules as in 1.

### 5. Tight and through

Rectangular pictures behave exactly like square (rows in `layout-float-wrap-tight`: x 72, y 72, text from 180 = 172+9 less nothing
extra) (M, 2 samples; Word's default wrap polygon of a rectangular picture is its rectangle). Through behaves the same as tight for
these (M). For a triangle autoshape (x 192..332, y 397..487) the right-hand text starts at 284.0, 294.1, 304.2, 314.3 on four
consecutive lines (+10.08 per 13.44 pt line, slope 0.75) and at 328.9 where the line box reaches y 471; a `x_outline(line bottom) + 9`
estimate is within about 2 pt in the first rows and 0.5 pt later. So tight wrapping follows the outline per line with the distance
added, stepped; exact polygon rules for ellipses and the wrap polygon of arbitrary artwork are open (L). Right-aligned ellipse data
(left segments end 403.8, 385.8, 390.6, 379.2 ... ) is recorded in the JSON but not modelled.

### 6. Collisions and allowOverlap (M)

`allowOverlap=1`: floats overlap as placed (pair at x 172/212, y 589.97/609.97, both kept).
`allowOverlap=0` (compat 14 and 15 identical): the float anchored earlier in the document (lower anchor offset, not the
lower index or z-order) keeps its position; the later one is moved horizontally so its left edge equals the right edge of the other
(no extra distance): square/square 172 -> 322 (= 212+110); left-aligned pair -> 182 (= 72+110); top-and-bottom pair 122 -> 282
(= 172+110); square after top-and-bottom 172 -> 332 (= 212+120). Vertical positions are unchanged. What happens when there is no
room to the right (or the left) is open. Evidence is 4 pairs, all with room on the right.

### 7. Anchor paragraph on the next page (H)

An anchor paragraph that does not fit the room left on page 1 starts page 2 (widow/orphan control) and its floats go with it:
paragraph-relative floats use the new paragraph top (72 -> y 72), page-relative offsets are relative to page 2 (100 -> y 100,
column/margin offsets likewise). A `pageBreakBefore` anchor behaves the same (float y 72/92 on page 2). A float anchored on page 1
whose paragraph stays there is unaffected (`layout-float-pagebreak-anchor`, `-anchor-nextpage`). Floats pushed beyond the bottom
margin: a paragraph-relative float anchored at page 1 y 628.8 with +120 (top-and-bottom) was placed at y 660 and +200 (square) at
y 660, i.e. clamped so that its bottom (720) meets the bottom margin; the in-front float +80 was not clamped (y 708.8). Clamping
applies to wrapping floats only (M, single sample each).

### 8. layoutInCell (H for compat 15, M for 14)

| compat | layoutInCell | positionH/V | result (float in a table cell, offset 10) |
|---|---|---|---|
| 15 | on | page | relative to the cell box: x 82.5 (cell left 72.5 + 10), y 271.64 (cell top 151.64 + 120) |
| 15 | on | margin / paragraph | margin = the cell's text area: 287.65 (text left 277.65 + 10); paragraph = cell paragraph top |
| 15 | off | page | identical to on: the flag is ignored (x 82.5, y 512.14 = row top + 280) |
| 14 | on | page | cell box (cell left 66.85, i.e. 5.4 pt of padding left of the text): x 76.85 |
| 14 | off | page / margin | page / page margin: x 10, y 280 (page); x 82 (margin 72 + 10) |
| 14 | off | right align | right edge 540 (page margin) instead of the cell's 261.2 |

Right-aligned in cell with layoutInCell on: right edge = cell text right (266.85 and 466.85 in 15; 261.2 in 14).

### 9. Anchor character x and line (L)

For a line-relative or character-relative float whose anchor character is the first character of a wrapped line, Word placed
the float at the end of the previous line (x 534 = end of line 2, y = line 2 top + 10, although the anchor offset is the first
character of line 3). One sample; the cause (the trailing space) is a hypothesis.

## What the engine needs

1. Resolve placement per section 1: separate reference boxes for page, margin, column (the anchor's column), character, line,
   paragraph, margin areas, inside/outside by page parity; alignment against that box with `effectExtent` included.
2. Wrap zone per section 2 with `wrapText` sides, `distT/B/L/R` and line-box intersection; a segment ends at `x - distL`, the next
   starts at `x + w + distR`; segments are separate lines in Word's model (and in the reference `lines`).
3. Top-and-bottom push-down per section 3, including lines of paragraphs before the anchor; the float's y comes from the paragraph
   top before the push.
4. Collision resolution with `allowOverlap=0` (earlier anchor wins, later float moves right of it), iterated for three or more
   floats (unmeasured).
5. Floats follow their anchor to the next page; page/margin offsets resolve on the page of the anchor.
6. Table cells: layoutInCell by compatibility mode (15: always in cell).
7. Tight/through polygons for autoshapes and pictures with a wrap polygon (`wp:wrapPolygon`), unmodelled.
8. Parity floors: the 20 new references score 1/78 to 44/44 against the current engine (`parity-floor.toml`); the weak ones
   (`layout-float-dist*`, `-poscol`, `-posv-line`, `-collide*`, `-wrap-tight`, `-incell*`) mark the work.

## Open

- Why `effectExtent r/b` is 6350 for only some sizes; whether bottom alignment is affected the same way.
- Tight/through for non-rectangular shapes beyond the triangle samples; wrap polygons from `wrapPolygon`.
- Collision with no room, three or more floats, collisions across wrap types other than the four pairs measured.
- Off-page and bottom clamping rules beyond the two samples.
- Line/character-relative anchors at a line break (section 9).
- Floats in headers/footers, multiple sections, mirrored margins (inside/outside only checked on page 1), RTL.
- The incell probes' text rows are not paired in the reference ("table row matches no paragraph" warnings) because a float in a
  cell changes the cell text rows; the `floats` data is complete.

## Tooling notes

- `word-layout-oracle.ps1` gained `-Floats` (always on for `layout-float-*`); existing output is unchanged for other documents.
  The picture regex now accepts the CRLF content streams the current Word build writes (`cm\r?\n/ImageN Do`); before this, pictures
  were silently not recorded and became "cells".
- For documents with floats, wrapped line segments are paired to PDF rows by baseline and left edge instead of leaving baselines null.
- Run: `pwsh -NoProfile -File tools/office/make-float-probes.ps1`, then
  `pwsh -NoProfile -Command '& ./tools/office/word-layout-oracle.ps1 -Path (Get-ChildItem fixtures/docx/layout-float-*.docx).FullName'`
  (about 4 minutes for the 20 probes).
