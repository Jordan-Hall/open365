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
| Text copy | ToUnicode CMap per font from the layout's glyph clusters (`Line::cluster_text`): a ligature glyph ("ft", "tti") or a glyph the font's `cmap` does not describe gets the characters it was shaped from; other glyphs use the reverse `cmap`. A line where some glyph stands for other characters than the CMap gives it (accent merged into its base, NBSP drawn with the space glyph) is wrapped in a `Span` with `/ActualText`. List labels with private-use symbol characters keep the reverse-`cmap` mapping. |
| Placement | One `BT .. Tm .. TJ .. ET` per run segment (split where face or vertical offset changes). TJ adjustments are computed from the cursor the PDF will have, so rounding never accumulates. |
| Decoration | Underline and strike from the font's `post`/`OS/2` metrics. |
| Tables | `CellBox` carries the resolved fill (`w:shd`, `pctNN` mixes mixed) and the four borders (cell, then table style, then table; solid, double, dashed, dotted). The PDF draws them as artifacts straight from the layout; it does not read the DOCX for them. Table, row and column ids are in `CellBox` too. |
| Images | Body pictures, and the inline and floating pictures of headers, footers and footnotes (`Page::header_images`, `footer_images`, `note_images`). A picture's `embed` id is resolved through the relationships of the part the layout names (`ImageSource`), so a header picture is found through the header part. Header, footer and note pictures are artifacts. JPEG: header validated, DCTDecode pass-through (CMYK Adobe inverted handled). Others: decoded with `image` under limits, Flate RGB plus an `SMask` when any alpha is not 255. |
| Pages | MediaBox from each page's section. |
| Tagging | `MarkInfo`, `StructTreeRoot`, `ParentTree`, `/Tabs /S`. Elements: Document, P, H1-H6 and L/LI/LBody (consecutive list paragraphs), both from the layout's `ParaRole` of the paragraph (heading level from the style name `heading N` through `basedOn`; a numbered heading is a list item as before), Table/TR/TH/TD (header rows are TH with Scope, spans as attributes), Figure with `/Alt` from the picture description. Footnote lines become P elements at the end; headers, footers, shading, borders and repeated table header rows are artifacts. |
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
- A glyph that stands for no characters (the note separator rule) gets no ToUnicode entry.
- CFF fonts are embedded whole. Variable fonts are embedded at their default instance only.
- No hyperlinks or bookmarks (no `Link` annotations or outline); pictures of headers, footers and
  notes are artifacts without alternative text in the structure tree; floating pictures there do
  not displace text; no EMF/WMF/SVG pictures (warning), no tiling
  patterns for stripe shading (fill colour only), no text rendering modes for outline/shadow.
- Colour is DeviceRGB, no output intent, no encryption.
- `ActualText` is per line, not per glyph run, and compares against the CMap only; it is not
  validated with an external checker.

## Layout API the export and the desktop use

`open365-layout` reports, so that no consumer has to guess:

- `PlacedGlyph::cluster` (`Cluster`): story range (relative to `Line::start`) and byte range in
  `Line::text` / `Line::label` of the characters a glyph was shaped from; a ligature is one glyph
  with a multi-character cluster, the later glyphs of a cluster carry an empty text range.
  `Line::cluster_story` and `Line::cluster_text` resolve them.
- `CellBox`: `table` (document-order id, stable under incremental update), `row`, `col`, spans,
  `header_row`, `fill` and `borders` (`CellBorders` of `BorderLine`).
- `Line::role` (`ParaRole`: plain, heading level, list item).
- `Page::header_images`, `footer_images`, `note_images` with `ImagePlacement::source`.

The desktop view uses the cluster map for caret, selection and accessibility character positions
(tab characters have no glyph and sit at the pen; a ligature is shared out by UTF-16 units), draws
cell shading and borders from `CellBox`, and publishes heading and list-item roles.

## Desktop: Export PDF and Print

`File > Export PDF...` (rfd save dialog) and `File > Print...` (Ctrl+P) render the open Word
document through `DocView::export_pdf`, which lays the document out if an edit is pending and uses
the library the screen layout used (installed fonts first), so the PDF has the pages the user sees.

| Part | Native? |
|---|---|
| Save dialog | Native (rfd: Win32/GTK/portal). |
| PDF production | Ours (this crate); fonts whose licence forbids embedding fail the export with a message in the status bar. |
| Print | Not native. The PDF goes to a temp file `open365-print-<pid>-<name>.pdf` and the OS print path runs on it: Windows `Start-Process -Verb Print` through PowerShell (the shell `print` verb of the default PDF handler, falling back to opening the file when the handler has no print verb); elsewhere `lp`. The path is passed in an environment variable, never in script text. |
| Print dialog, printer choice, page range, duplex | Whatever the PDF handler offers (none for the Windows `print` verb, which prints to the default printer). We have no print dialog or print preview of our own. |
| Temp file | Left in the temp directory (the handler reads it after we return). |

Untested: actual printing (a test asserts the command shape only), macOS, and Linux without CUPS.
