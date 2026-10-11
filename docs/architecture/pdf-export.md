# PDF export (`open365-pdf`)

Layer 4. Implements M3 of `docs/roadmap/first-milestones.md`; this is also the print path of ADR-004
(print = the same layout, written as PDF).

## API

```rust
export_pdf(doc: &Document, layout: &Layout, fonts: &FontLibrary, meta: &PdfMetadata)
    -> Result<PdfOutput { bytes, warnings }, PdfError>
```

`fonts` must be the library the layout was made with. CLI: `open365 docx export-pdf <in> -o <out>
[--title --author --subject --keywords --language --system-fonts]`. Nothing is written for author,
dates or language unless the caller passes it.

## What is written

PDF 1.7, classic xref table, own writer on `flate2` (already in the dependency graph; MIT/Apache).

| Feature | How |
|---|---|
| Fonts | Type0 / Identity-H. TrueType: glyf subset (glyph ids unchanged, unused outlines emptied, composite closure, `loca` rebuilt, checksums fixed). CFF/OpenType: embedded whole as FontFile3. Fonts with the OS/2 `fsType` restricted-licence bit are refused; the no-subsetting bit embeds the whole font. |
| Text copy | ToUnicode CMap per font. Reverse `cmap` first; ligature glyphs the cmap does not describe ("ft", "tti") are recovered by lining a paragraph's glyphs up with its text. |
| Placement | One `BT .. Tm .. TJ .. ET` per run segment (split where face or vertical offset changes). TJ adjustments are computed from the cursor the PDF will have, so rounding never accumulates. |
| Decoration | Underline and strike from the font's `post`/`OS/2` metrics. |
| Tables | Cell shading (`w:shd`, including `pctNN` mixes) and borders (cell, then table style, then table; solid, double, dashed/dotted) as artifacts. Cells are found from the layout's cell boxes through the text inside them. |
| Images | JPEG: header validated, DCTDecode pass-through (CMYK Adobe inverted handled). Others: decoded with `image` under limits, Flate RGB plus an `SMask` when any alpha is not 255. |
| Pages | MediaBox from each page's section. |
| Tagging | `MarkInfo`, `StructTreeRoot`, `ParentTree`, `/Tabs /S`. Elements: Document, P, H1-H6 (style name `heading N`, through `basedOn`), L/LI/LBody (consecutive list paragraphs), Table/TR/TH/TD (header rows are TH with Scope, spans as attributes), Figure with `/Alt` from the picture description. Footnote lines become P elements at the end; headers, footers, shading, borders and repeated table header rows are artifacts. |
| Metadata | Info dictionary and XMP from `PdfMetadata`; `DisplayDocTitle` only with a title. |

The file is deterministic (the `/ID` is a digest of metadata and layout size, no clock).

## Hostile input (pictures)

Image parts are untrusted. Limits: 64 MiB per part, 20 000 px per side, 25 Mpx per image, 256 MiB
decoder allocation. Sizes are read from the header and checked before any pixel buffer exists.
JPEG with a non-8-bit, arithmetic-coded, lossless or odd-component frame is refused. A failing
picture produces a warning and is left out; it never fails the export. Fonts come from the
caller's library, not from the document.

## Validation

- `tests/roundtrip.rs` re-reads every layout fixture that the bundled fonts can lay out (41) with
  a test-only parser (xref offsets are verified, streams inflated, content operators
  interpreted): glyph ids, face and x/y of every glyph equal the Layout to 0.01 pt (worst error
  0.00005 pt over 287 045 glyphs), the ToUnicode text of every paragraph equals the document text
  (up to list labels and note marks), the structure tree claims every marked-content id, sections
  give page sizes, fonts re-parse with skrifa.
- `tests/word_baselines.rs` pairs lines with Word's reference PDF as `word_parity` does (story,
  first offset, same page, 0.5 pt): the PDF baselines match Word on 5 226 of 5 454 paired lines,
  identical to the layout engine's own count, so export adds no error.
- `tests/tables.rs`, unit tests in `image.rs` (alpha SMask, JPEG pass-through, oversize and
  garbage input).
- No external validator (veraPDF, PAC) has been run: the tagged structure is not claimed to be
  PDF/UA conformant.

## Known gaps

- No PDF/UA or PDF/A claim; no `pdfuaid` XMP; Lbl is not separated from LBody (the list label is
  inside the LBody marked content); Table `Summary`, `Headers` ids and TH scope Row are absent.
- Nested tables: an outer cell that contains an inner table takes its borders from the least
  nested paragraph found inside it; borders of nested cells are right, merged-cell edge cases
  are untested.
- Text reading order follows the layout (lines), not Word's story order for floats; notes are
  appended to the Document element.
- Glyphs with no cmap entry and no alignable paragraph text (fields, hidden text, symbols) get no
  ToUnicode entry.
- CFF fonts are embedded whole. Variable fonts are embedded at their default instance only.
- No hyperlinks or bookmarks (no `Link` annotations or outline), no pictures in headers, footers
  or notes (the layout does not report them), no EMF/WMF/SVG pictures (warning), no tiling
  patterns for stripe shading (fill colour only), no text rendering modes for outline/shadow.
- Colour is DeviceRGB, no output intent, no encryption.
- Layout gives no per-glyph source text; a cluster map from `open365-text` would replace the
  alignment heuristic and enable `ActualText`.
