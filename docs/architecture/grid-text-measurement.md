# Grid text measurement and column auto-fit

Status: measured against Excel (build 20430) at 120 dpi, 2026-10-11. Oracle:
`tools/office/grid-advances-oracle.ps1` -> `fixtures/reference/xlsx/grid-advances/advances.json`;
end to end: `tools/office/grid-autofit-oracle.ps1` -> `fixtures/reference/xlsx/grid-autofit/`.

## Findings

- Glyph advances: Excel uses whole-pixel hinted advances at the GDI ppem (`round(pt * dpi / 72)`).
  `HintedFace` (skrifa, TrueType interpreter) reproduces all 95 printable ASCII advances for
  Calibri 9/11/14/20 and Cambria 11/14 (unit test `hinted_advances_reproduce_excel_auto_fit`).
  The unhinted advances were never the problem.
- The error that grew with text length was not text measurement. `AutoFit` returns
  `t + floor((t - 83) / 36)` pixels for `t >= 83` px of text plus padding (mdw 9 px), independent of
  font, glyphs and length (steps measured at every integer `t` up to 560). `excel_fit_px`.
- Padding is per font, not per workbook: `2 * ceil(mdw / 4) + 3` px with `mdw` the widest digit of the
  cell's own hinted font (7, 9, 9, 13 px for Calibri 9/11/14/20; 9, 11 for Cambria 11/14).
- Italic overhang is `round(ppem / 3)`; a 90-degree rotated cell is one line height plus 1 px.

## Open

- The scaling step positions shift by font (first step at 79-80 px for Calibri 9, 83 for Calibri 11, 81 for
  Calibri 14, 84-86 for Calibri 20, 83-85 for Cambria 14); only 83 is used, so other fonts can be 1 px off
  near a step. The cause is unknown.
- The fit scaling is only measured with Normal = Calibri 11 at 120 dpi (mdw 9); other digit widths
  are left unscaled until measured at other display scales.
- Excel's 255-character cap counts the padding (2302 px); stored widths here stop at 255.
- Wrapped text containing a line break is 5 px narrower than Excel.
