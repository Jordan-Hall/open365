# PDF export (`open365-pdf`)

Layer 4. Implements M3 of `docs/roadmap/first-milestones.md`; this is also the print path of ADR-004
(print = the same layout, written as PDF).

## API

```rust
export_pdf(doc: &Document, layout: &Layout, fonts: &FontLibrary, meta: &PdfMetadata)
    -> Result<PdfOutput { bytes, warnings }, PdfError>
```

`export_pdf_with(.., &PdfOptions { conformance, bookmarks, ua }, &cancel)` adds PDF/A-1b/2b, the
bookmarks switch and PDF/UA-1 (`pdfuaid`); `export_pdf` uses the defaults (plain PDF 1.7, bookmarks on).
`fonts` must be the library the layout was made with. CLI: `open365 docx export-pdf <in> -o <out>
[--title --author --subject --keywords --language|--lang --document-properties --pdfa 1b|2b --pdfua
--no-bookmarks --system-fonts --overwrite --allow-destructive --root <dir>]`. Author and dates are
written only when the caller passes them or asks for `--document-properties`
(`PdfMetadata::from_document`; flags override single fields). The export is the `export_pdf`
command and goes through `open365_mcp::policy` (`assess` + `cli_decision`): replacing a file needs
`--overwrite` and `--allow-destructive`; `--root` resolves the output through the fence and the
file is written at the resolved path; one JSON audit line (`applied` or `refused`) goes to stderr.
The CLI renders itself (layout + `export_pdf_with`) because `open365_commands::PdfOptions` has no
field for the PDF options yet; see Known gaps.

## What is written

PDF 1.7, classic xref table, own writer on `flate2` (already in the dependency graph; MIT/Apache).

| Feature | How |
|---|---|
| Fonts | Type0 / Identity-H for every font (so complex-script clusters need no encoding tricks; glyph ids are CIDs). TrueType: glyf subset (glyph ids unchanged, unused outlines emptied, composite closure, `loca` rebuilt, checksums fixed). CFF/OpenType: `cff.rs` writes a bare CFF with the used charstrings only (unused glyphs shrink to `endchar`, ids unchanged; local and global subroutines are kept whole; a CID-keyed font gets an identity charset so CID = glyph id), embedded as `FontFile3 /CIDFontType0C`. Fonts with the OS/2 `fsType` restricted-licence bit are refused; the no-subsetting bit embeds the whole font; CFF2 or an unreadable CFF is embedded whole with a warning. |
| Text copy | ToUnicode CMap per font from the layout's glyph clusters (`Line::cluster_text`): a ligature glyph ("ft", "tti") or a glyph the font's `cmap` does not describe gets the characters it was shaped from; other glyphs use the reverse `cmap`. A line where some glyph stands for other characters than the CMap gives it (accent merged into its base, NBSP drawn with the space glyph) is wrapped in a `Span` with `/ActualText`. List labels with private-use symbol characters keep the reverse-`cmap` mapping. |
| Placement | One `BT .. Tm .. TJ .. ET` per run segment (split where face or vertical offset changes). TJ adjustments are computed from the cursor the PDF will have, so rounding never accumulates. |
| Decoration | Underline and strike from the font's `post`/`OS/2` metrics. |
| Tables | `CellBox` carries the resolved fill (`w:shd`, `pctNN` mixes mixed) and the four borders (cell, then table style, then table; solid, double, dashed, dotted). The PDF draws them as artifacts straight from the layout; it does not read the DOCX for them. Table, row and column ids are in `CellBox` too. |
| Images | Body pictures, and the inline and floating pictures of headers, footers and footnotes (`Page::header_images`, `footer_images`, `note_images`). A picture's `embed` id is resolved through the relationships of the part the layout names (`ImageSource`), so a header picture is found through the header part. Header, footer and note pictures are artifacts. JPEG: header validated, DCTDecode pass-through (CMYK Adobe inverted handled). Others: decoded with `image` under limits, Flate RGB plus an `SMask` when any alpha is not 255. |
| Tagging | `MarkInfo`, `StructTreeRoot`, `ParentTree`, `IDTree`, `/Tabs /S`. Elements: Document, P, H1-H6 (also a numbered heading, whose label is part of the heading's marked content, as in Word; a heading is found by role or by its style `heading N` through `basedOn`), L (`/A ListNumbering`)/LI/Lbl/LBody for other numbered paragraphs, `Table > THead > TR > TH` and `TBody > TR > TH/TD` as Word writes them (the first row, plus `tblHeader` rows, is the head; with the table look `firstColumn` the first cell of each body row is a TH). TH has `/ID` and `/A Scope Column|Row`; TD has `/A Headers [ids]` (the column heads it spans and its row head); spans are `ColSpan`/`RowSpan` and a cell covered by a vertical merge has no element. `Table /Alt` and `/A Summary` come from `w:tblDescription` (Word writes only `/Alt`, from the description, never from the caption). Figure with `/Alt` from the picture description (PDF/UA mode: a placeholder when there is none). A hyperlink is a `Link` element inside its paragraph (marked content of the link text plus an `OBJR` to the annotation). Footnote lines become P elements at the end; headers, footers, shading, borders and repeated table header rows are artifacts. |
| Language | `/Lang` in the catalog: the caller's, else the primary subtag of the document default (`w:docDefaults` `w:lang`; `de-DE` gives `de`, as Word writes the subtag). Text whose effective language (run `w:lang`, character style, paragraph style chain, default) differs from it gets a `Span /Lang` element (evidence: Word's exports of `fixtures/reference/pdf/probes/probe-lang*`). Word's own catalog `/Lang` was `en` for every probe whatever the document language (de-DE, fr-FR), so it is not derived from the document; we write the document's. |
| Metadata | Info dictionary and XMP from `PdfMetadata` (title, author, subject, keywords, creator, created, modified, language); Info and XMP carry the same values. `PdfMetadata::from_document` reads the document's own core properties (Word's "Document properties" option); it is never applied implicitly. `DisplayDocTitle` only with a title. PDF/UA (`PdfOptions::ua`): `pdfuaid:part` 1 in the XMP, `DisplayDocTitle` always, and when no title was given the first heading is used (with a warning). |
| Hyperlinks | `Link` annotations (print flag, no border, `Contents` = link text) over the glyph extent of each line the link covers. External targets are limited to `http`, `https`, `mailto`, `ftp`, `tel` (anything else is dropped with a warning); `w:anchor` becomes an explicit `Dest [page /XYZ x y 0]` at the first line of the bookmark's paragraph. Link ranges come from aligning the model's paragraph text (cached field results, no field codes) with the text the layout drew (`align.rs`: common prefix and suffix plus an LCS of the middle), so a link around a field whose value the layout computed (a PAGE field) is still found; no text search is used. Links in headers, footers and notes are made (the header or footer part is picked per section and kind, as the layout does); their annotations get a `Link` element each (headers and footers are artifacts, so it sits under the Document element and holds only the OBJR). Links in text boxes are not made. |
| Bookmarks | Outline from the layout's heading roles (`PdfOptions::bookmarks`, default on, as Word's "Create bookmarks using Headings"), nested by level, `Dest` at the top of the heading's first line. |
| Pages | MediaBox from each page's section, on Word's 600 dpi grid: an A4 section (595.3 x 841.9 pt) is written 595.2 x 841.92 pt as Word does (evidence: every A4 page of the 132 reference PDFs); other sizes are written as the section says. |
| PDF/A | `PdfOptions::conformance` = `PdfA1b` (header 1.4, pictures with alpha composited onto white, no soft masks) or `PdfA2b` (1.7). Both: XMP with `pdfaid:part`/`conformance`, uncompressed XMP, Info/XMP agreement, `/ID`, sRGB output intent with an ICC profile written by `icc.rs` from the public sRGB definition, all fonts embedded, no encryption/JavaScript/transparency operators. PDF/A-1 also gets a `CIDSet` per font; the `.notdef` glyph is never drawn in PDF/A or PDF/UA mode. veraPDF results: see Validation. |

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
  first offset, same page, 0.5 pt): the PDF baselines match Word on 10 453 of 10 775 paired lines
  (125 fixtures, 2026-10-11), identical to the layout engine's own count, so export adds no error.
- `tests/word_geometry.rs` compares every glyph of our PDF with Word's own export of the same
  fixture (`fixtures/reference/pdf/<name>.pdf`, made by `tools/office/make-pdf-refs.ps1` with
  `ExportAsFixedFormat`: print quality, heading bookmarks, document structure tags, no document
  properties). One content reader (`tests/common/scan.rs`: scans objects, unpacks object streams,
  full text matrices, ToUnicode `bfchar`/`bfrange`, simple and Type0 fonts) reads both files.
  Characters of a page are paired by longest common subsequence (blanks ignored) and a pair
  agrees when x and y are within 0.5 pt. See "Word geometry" below.
- `tests/features.rs`: link annotation + `Link` element + parent tree, bookmark titles and
  destinations, internal jumps, `Lbl`/`LBody`, Info/XMP agreement, core properties, and the PDF/A
  profiles (header, `pdfaid`, output intent ICC, embedded fonts, banned features).
- Unit tests: `cff.rs` (synthetic name-keyed CFF; also checked once by hand against a real CFF
  OpenType font, 3681 to 287 bytes, re-parsed by skrifa), `icc.rs`, `meta.rs`, outline nesting.
- PyMuPDF opened the tagged and PDF/A output of seven fixtures, rendered a page and listed the
  links (an ad hoc check, not kept).
- `tests/tables.rs`, unit tests in `image.rs` (alpha SMask, JPEG pass-through, oversize and
  garbage input).
- `tests/tagging.rs` checks language spans, headings, table structure, links in every story, the
  field-replaced link, `pdfuaid` and the CIDSet on the Word probes (`fixtures/reference/pdf/probes`:
  nine small documents made for this, with Word's own PDFs beside them; regenerate with
  `tools/office/make-pdf-refs.ps1 -DocxDir fixtures/reference/pdf/probes -OutDir fixtures/reference/pdf/probes`).
- veraPDF 1.30.3 (GPL/MPL Java CLI, installed from downloads.verapdf.org into a scratch folder, run
  as an external dev-time tool only, never bundled or vendored), 2026-10-11, on 17 files (the 6
  probes plus plain-paragraphs, table-merged-cells, hyperlink-external, footnotes-endnotes,
  header-footer, layout-hf-basic, layout-lists-multilevel, inline-image, layout-table-basic,
  comments, fields), each exported with `--pdfa 1b`, `--pdfa 2b`, `--pdfua` and `--pdfua --pdfa
  2b`: PDF/A-1b 17 of 17 pass, PDF/A-2b 17 of 17, PDF/UA-1 17 of 17, PDF/UA-1 on the combined
  files 17 of 17. Problems it found and that were fixed: PDF/A-1 `CIDSet` missing (6.3.5),
  `.notdef` glyph drawn (6.2.11.8; UA 7.21.7, 7.21.8), a figure without alternative text (UA 7.3),
  rows of a table with vertical merges counted twice (UA 7.2-42). This is a sample, not a claim of
  full conformance: text boxes, charts and EMF pictures are not covered.

## Word geometry (measured 2026-10-11)

word geometry: 125 fixtures, 438027 Word glyphs, 99.37% paired, 86.23% within 0.5 pt of Word; pages 347 ours / 347 Word, same size 347, exact 185

Word 16.0.20430 on the generated fixtures that the bundled fonts can lay out. "Paired" is the share
of Word's non-blank characters that have a counterpart in our page (LCS); "within 0.5 pt" counts
pairs whose x and y both agree. Pages and MediaBoxes agree on all 347 pages. The PDF writer adds
no error of its own (`roundtrip.rs`: glyphs equal the layout within 0.01 pt), so every miss below
is the layout engine's: the layout owner's list is under "Differences for the layout owner".
The floors in the test (99.3 % paired, 86.2 % within tolerance, 185 exact pages) only move up.

```
comments                           pages  1/1  glyphs     42 paired 100.0% within 0.5 pt 100.0%
custom-xml-part                    pages  1/1  glyphs     31 paired 100.0% within 0.5 pt 100.0%
footnotes-endnotes                 pages  1/1  glyphs     66 paired 100.0% within 0.5 pt 100.0%
header-footer                      pages  2/2  glyphs     72 paired  76.4% within 0.5 pt  70.8%
hidden-text                        pages  1/1  glyphs     24 paired 100.0% within 0.5 pt  54.2%
hyperlink-external                 pages  1/1  glyphs     16 paired 100.0% within 0.5 pt 100.0%
inline-image                       pages  1/1  glyphs     16 paired 100.0% within 0.5 pt 100.0%
layout-align                       pages  1/1  glyphs   1446 paired 100.0% within 0.5 pt  25.4%
layout-anc-hyperlink               pages  1/1  glyphs     84 paired 100.0% within 0.5 pt 100.0%
layout-anc-hyperlink-hf            pages  1/1  glyphs    131 paired  89.3% within 0.5 pt  84.0%
layout-anc-hyperlink-hist0         pages  1/1  glyphs     84 paired 100.0% within 0.5 pt 100.0%
layout-anc-hyperlink-nohist        pages  1/1  glyphs     84 paired 100.0% within 0.5 pt 100.0%
layout-caps                        pages  3/3  glyphs   2136 paired 100.0% within 0.5 pt  14.9%
layout-caps-chars                  pages  3/3  glyphs     74 paired 100.0% within 0.5 pt 100.0%
layout-caps-sizes                  pages  2/2  glyphs    208 paired 100.0% within 0.5 pt  46.6%
layout-cols-balance                pages  1/1  glyphs   1235 paired 100.0% within 0.5 pt  99.5%
layout-cols-balance-pages          pages  2/2  glyphs   3757 paired 100.0% within 0.5 pt  99.7%
layout-cols-break                  pages  3/3  glyphs   2727 paired 100.0% within 0.5 pt 100.0%
layout-cols-continuous             pages  1/1  glyphs   2580 paired 100.0% within 0.5 pt  99.7%
layout-cols-footnote               pages  1/1  glyphs   2212 paired  97.4% within 0.5 pt  65.5%
layout-cols-four                   pages  2/2  glyphs   5060 paired 100.0% within 0.5 pt 100.0%
layout-cols-line                   pages  2/2  glyphs   3746 paired 100.0% within 0.5 pt  99.8%
layout-cols-mid                    pages  3/3  glyphs   9378 paired 100.0% within 0.5 pt  99.6%
layout-cols-newcol                 pages  1/1  glyphs    999 paired 100.0% within 0.5 pt 100.0%
layout-cols-nextpage               pages  3/3  glyphs   2353 paired 100.0% within 0.5 pt  99.5%
layout-cols-pagebefore             pages  4/4  glyphs   9342 paired 100.0% within 0.5 pt 100.0%
layout-cols-pagebreak              pages  3/3  glyphs   2098 paired 100.0% within 0.5 pt 100.0%
layout-cols-spacing                pages  3/3  glyphs   5638 paired 100.0% within 0.5 pt 100.0%
layout-cols-table                  pages  1/1  glyphs   1708 paired 100.0% within 0.5 pt 100.0%
layout-cols-three                  pages  2/2  glyphs   4926 paired 100.0% within 0.5 pt 100.0%
layout-cols-two                    pages  2/2  glyphs   4486 paired 100.0% within 0.5 pt 100.0%
layout-cols-unequal                pages  2/2  glyphs   3826 paired 100.0% within 0.5 pt  90.7%
layout-cols-unequal-two            pages  3/3  glyphs   7202 paired 100.0% within 0.5 pt 100.0%
layout-float-align                 pages  1/1  glyphs   2535 paired 100.0% within 0.5 pt  99.5%
layout-float-anchor-bottom         pages  2/2  glyphs   4808 paired  96.8% within 0.5 pt  61.4%
layout-float-anchor-nextpage       pages  2/2  glyphs   4808 paired  97.3% within 0.5 pt  64.6%
layout-float-collide               pages  2/2  glyphs   3380 paired 100.0% within 0.5 pt  99.5%
layout-float-collide-compat14      pages  1/1  glyphs   2529 paired 100.0% within 0.5 pt  98.5%
layout-float-collide-tb            pages  1/1  glyphs   2535 paired 100.0% within 0.5 pt  99.2%
layout-float-dist                  pages  2/2  glyphs   3380 paired  98.6% within 0.5 pt  97.1%
layout-float-dist-mixed            pages  2/2  glyphs   3380 paired 100.0% within 0.5 pt  99.5%
layout-float-incell                pages  1/1  glyphs    943 paired 100.0% within 0.5 pt 100.0%
layout-float-incell-before         pages  1/1  glyphs    943 paired 100.0% within 0.5 pt 100.0%
layout-float-incell-compat14       pages  1/1  glyphs    943 paired 100.0% within 0.5 pt  99.9%
layout-float-pagebreak-anchor      pages  2/2  glyphs   1690 paired 100.0% within 0.5 pt  99.3%
layout-float-poscol                pages  3/3  glyphs   8450 paired 100.0% within 0.5 pt 100.0%
layout-float-posh                  pages  1/1  glyphs   2535 paired 100.0% within 0.5 pt  99.5%
layout-float-posv                  pages  2/2  glyphs   3380 paired 100.0% within 0.5 pt  99.5%
layout-float-posv-line             pages  1/1  glyphs   1690 paired 100.0% within 0.5 pt  35.2%
layout-float-wrap-behind-front     pages  1/1  glyphs   1690 paired 100.0% within 0.5 pt  99.5%
layout-float-wrap-sides            pages  1/1  glyphs   2535 paired 100.0% within 0.5 pt  99.5%
layout-float-wrap-square           pages  1/1  glyphs   2535 paired 100.0% within 0.5 pt  99.4%
layout-float-wrap-tight            pages  2/2  glyphs   3380 paired 100.0% within 0.5 pt  99.2%
layout-float-wrap-topbottom        pages  2/2  glyphs   2535 paired 100.0% within 0.5 pt  99.5%
layout-fn-basic                    pages  3/3  glyphs   6546 paired 100.0% within 0.5 pt  78.6%
layout-fn-endnotes                 pages  3/3  glyphs   6311 paired 100.0% within 0.5 pt  28.4%
layout-fn-long                     pages  3/3  glyphs   9627 paired 100.0% within 0.5 pt  98.9%
layout-fn-mixed                    pages  2/2  glyphs   4959 paired  99.8% within 0.5 pt  49.7%
layout-frame-basic                 pages  1/1  glyphs   1515 paired  85.7% within 0.5 pt  84.8%
layout-frame-bdr                   pages  1/1  glyphs   1731 paired  87.5% within 0.5 pt  86.4%
layout-frame-dropcap               pages  1/1  glyphs    648 paired  99.7% within 0.5 pt  97.7%
layout-frame-gap                   pages  1/1  glyphs   1867 paired  94.5% within 0.5 pt  93.2%
layout-frame-nobdr                 pages  1/1  glyphs   1735 paired  81.1% within 0.5 pt  73.8%
layout-frame-page                  pages  1/1  glyphs   1515 paired  85.9% within 0.5 pt  85.0%
layout-hdr-none                    pages  3/3  glyphs   3012 paired 100.0% within 0.5 pt 100.0%
layout-hdr-none-big                pages  3/3  glyphs   3012 paired 100.0% within 0.5 pt 100.0%
layout-hdr-none-near               pages  3/3  glyphs   3012 paired 100.0% within 0.5 pt 100.0%
layout-hdr-none-normal             pages  3/3  glyphs   1491 paired 100.0% within 0.5 pt 100.0%
layout-hdr-none-style              pages  3/3  glyphs   3012 paired 100.0% within 0.5 pt 100.0%
layout-hf-basic                    pages  3/3  glyphs   8010 paired  99.7% within 0.5 pt  98.5%
layout-hf-distance                 pages  2/2  glyphs   5474 paired  98.5% within 0.5 pt  97.3%
layout-hf-first-even               pages  5/5  glyphs  13342 paired  99.5% within 0.5 pt  98.2%
layout-hf-sections                 pages  4/4  glyphs  10780 paired  98.6% within 0.5 pt  97.3%
layout-img-inline                  pages  1/1  glyphs    539 paired 100.0% within 0.5 pt  94.6%
layout-img-line                    pages  1/1  glyphs     63 paired 100.0% within 0.5 pt 100.0%
layout-img-none                    pages  1/1  glyphs    603 paired 100.0% within 0.5 pt  89.7%
layout-img-spacing                 pages  3/3  glyphs    148 paired 100.0% within 0.5 pt 100.0%
layout-img-square                  pages  2/2  glyphs   4884 paired 100.0% within 0.5 pt  99.0%
layout-img-tb                      pages  1/1  glyphs   1628 paired 100.0% within 0.5 pt  92.4%
layout-indents-spacing             pages  3/3  glyphs   6790 paired 100.0% within 0.5 pt  67.4%
layout-justify                     pages 31/31 glyphs 109900 paired 100.0% within 0.5 pt  77.7%
layout-keep                        pages 44/44 glyphs  14009 paired 100.0% within 0.5 pt 100.0%
layout-lists-multilevel            pages  1/1  glyphs    413 paired 100.0% within 0.5 pt  55.7%
layout-lists-style                 pages  1/1  glyphs    773 paired 100.0% within 0.5 pt  19.4%
layout-lists-suff                  pages  2/2  glyphs   2015 paired 100.0% within 0.5 pt  19.2%
layout-marks                       pages  1/1  glyphs    133 paired 100.0% within 0.5 pt 100.0%
layout-mixed-sizes                 pages  1/1  glyphs    581 paired 100.0% within 0.5 pt  32.9%
layout-pagination                  pages 14/14 glyphs   7796 paired 100.0% within 0.5 pt  87.2%
layout-pbdr-ctx                    pages  1/1  glyphs    559 paired  98.0% within 0.5 pt  97.3%
layout-pbdr-edges                  pages  2/2  glyphs   1516 paired 100.0% within 0.5 pt  98.8%
layout-pbdr-groups                 pages  2/2  glyphs    272 paired 100.0% within 0.5 pt 100.0%
layout-pbdr-pages                  pages  3/3  glyphs   8426 paired 100.0% within 0.5 pt  61.4%
layout-pbdr-styles                 pages  2/2  glyphs    409 paired 100.0% within 0.5 pt  83.1%
layout-pbdr-widths                 pages  2/2  glyphs    357 paired 100.0% within 0.5 pt 100.0%
layout-sdt                         pages  1/1  glyphs    449 paired 100.0% within 0.5 pt 100.0%
layout-sect-empty                  pages  5/5  glyphs     22 paired 100.0% within 0.5 pt 100.0%
layout-sect-space                  pages  3/3  glyphs     20 paired 100.0% within 0.5 pt  50.0%
layout-sect-tail                   pages  8/8  glyphs   1399 paired 100.0% within 0.5 pt 100.0%
layout-sizes                       pages  9/9  glyphs   2222 paired  99.5% within 0.5 pt  73.0%
layout-table-basic                 pages  2/2  glyphs   1055 paired 100.0% within 0.5 pt 100.0%
layout-table-flow                  pages  2/2  glyphs    923 paired 100.0% within 0.5 pt 100.0%
layout-table-pages                 pages 18/18 glyphs  15740 paired  98.9% within 0.5 pt  98.8%
layout-table-rows                  pages  2/2  glyphs   1651 paired  88.2% within 0.5 pt  86.9%
layout-table-width                 pages  2/2  glyphs   1510 paired  99.2% within 0.5 pt  98.1%
layout-tabs                        pages  1/1  glyphs    585 paired 100.0% within 0.5 pt  36.8%
layout-tblp-allow                  pages  1/1  glyphs   1563 paired  96.9% within 0.5 pt  95.7%
layout-tblp-basic                  pages  1/1  glyphs   1320 paired  98.2% within 0.5 pt  96.7%
layout-tblp-bottom                 pages  2/2  glyphs   2942 paired  97.9% within 0.5 pt  96.6%
layout-tblp-margin                 pages  1/1  glyphs   1539 paired  98.4% within 0.5 pt  97.0%
layout-tblp-never                  pages  1/1  glyphs   1563 paired  96.9% within 0.5 pt  95.7%
layout-tblp-page                   pages  1/1  glyphs   1779 paired  97.3% within 0.5 pt  95.8%
layout-tblp-right                  pages  1/1  glyphs   1320 paired  98.2% within 0.5 pt  96.5%
layout-tblp-right2                 pages  1/1  glyphs   1320 paired  98.2% within 0.5 pt  96.7%
layout-tblp-yspec                  pages  2/2  glyphs   3303 paired  98.5% within 0.5 pt  96.9%
layout-trh-asym                    pages  2/2  glyphs    173 paired 100.0% within 0.5 pt 100.0%
layout-trh-pad                     pages  2/2  glyphs    262 paired 100.0% within 0.5 pt  99.2%
layout-trh-small                   pages  3/3  glyphs    579 paired 100.0% within 0.5 pt  97.6%
layout-trh-tall                    pages 10/10 glyphs    108 paired 100.0% within 0.5 pt 100.0%
layout-txbx-basic                  pages  1/1  glyphs    355 paired  62.3% within 0.5 pt  36.3%
layout-txbx-wrap                   pages  2/2  glyphs   4041 paired 100.0% within 0.5 pt  38.6%
layout-wrap                        pages  1/1  glyphs   2270 paired 100.0% within 0.5 pt  28.7%
plain-paragraphs                   pages  1/1  glyphs     74 paired 100.0% within 0.5 pt 100.0%
sections-landscape                 pages  2/2  glyphs     33 paired 100.0% within 0.5 pt 100.0%
table-merged-cells                 pages  1/1  glyphs     64 paired 100.0% within 0.5 pt 100.0%
tracked-changes                    pages  1/1  glyphs     39 paired 100.0% within 0.5 pt 100.0%
word geometry: 125 fixtures, 438027 Word glyphs, 99.37% paired, 86.23% within 0.5 pt of Word; pages 347 ours / 347 Word, same size 347, exact 185
```

### Differences for the layout owner (fixtures under 70 % within 0.5 pt, not fixed here)

hidden-text 54 %, layout-align 25 %, layout-caps 15 %, layout-caps-sizes 47 %, layout-cols-footnote
66 % (97 % paired), layout-float-anchor-bottom 61 % / -nextpage 65 % (97 % paired),
layout-float-posv-line 35 %, layout-fn-endnotes 28 %, layout-fn-mixed 50 %, layout-indents-spacing
67 %, layout-lists-multilevel 56 %, layout-lists-style 19 %, layout-lists-suff 19 %,
layout-mixed-sizes 33 %, layout-pbdr-pages 61 %, layout-sect-space 50 %, layout-tabs 37 %,
layout-txbx-basic 36 % (62 % paired), layout-txbx-wrap 39 %, layout-wrap 29 %; also below 80 %:
layout-justify 78 % (109 900 glyphs, the largest single loss), layout-sizes 73 %, header-footer 71 %,
layout-frame-* 74 to 93 % (frames and drop caps pair only 81 to 94 %). These match the topics of
the layout oracle (alignment, tabs, list labels, caps, footnotes/endnotes, floats, text boxes,
justification); the PDF side was not changed to hide them.

## Known gaps

- PDF/A and PDF/UA were validated on a sample only (see Validation). A DeviceCMYK JPEG is not
  converted for PDF/A. UA: the footnote reference mark in the body is not a link to its note and
  notes are not `Footnote`/`Note` elements (Word writes both); header and footer link annotations
  are tagged under the Document element, not inside the header.
- Language: only `w:lang w:val` is read (not `w:eastAsia` or `w:bidi` by script; Word picks by
  script). Catalog `/Lang` differs from Word's constant `en` for non-English documents, on purpose.
- CFF: subroutines are not pruned, there is no test with a CID-keyed font and no CFF font is
  bundled, so the end-to-end CFF path (FontFile3) was exercised only on unit fixtures and one
  ad hoc font. Complex-script (Identity-H plus cluster ToUnicode) is tested with the Latin
  ligature/accent clusters of the bundled fonts only; no Arabic or Indic font is available.
- Hyperlinks: the alignment finds a link whose text a field replaced, but a link inside a text
  box is not made (text boxes are artifacts); the layout owner could remove the alignment by
  putting the link index on `GlyphRun` or `Cluster`; a link over several lines gets one annotation per line but one `Link` element placed at its
  first line in the reading order.
- Nested tables: an outer cell that contains an inner table takes its borders from the least
  nested paragraph found inside it; borders of nested cells are right, merged-cell edge cases
  are untested.
- Text reading order follows the layout (lines), not Word's story order for floats; notes are
  appended to the Document element.
- A glyph that stands for no characters (the note separator rule) gets no ToUnicode entry.
- Variable fonts are embedded at their default instance only.
- Pictures of headers, footers and notes are artifacts without alternative text in the structure
  tree; floating pictures there do not displace text; no EMF/WMF/SVG pictures (warning), no
  tiling patterns for stripe shading (fill colour only), no text rendering modes for
  outline/shadow.
- Colour is DeviceRGB, no encryption.
- Caps and small caps copy as the displayed capitals. Evidence (Word 16.0.20430, `ExportAsFixedFormat`
  of `fixtures/docx/layout-caps-chars.docx`): the PDF has no `/ActualText` at all, and the font's
  ToUnicode CMap maps the glyph of a small-cap "n" to U+004E. We match it: a cluster whose text
  differs from the glyph's reverse-`cmap` character only by case gets no CMap override and no
  `ActualText`. The round-trip test compares `layout-caps*` paragraphs case-insensitively.
- `ActualText` is per marked-content segment (a line, split at list labels and links), compares
  against the CMap only, and is not validated with an external checker.
- The CLI passes `PdfOptions` and `PdfMetadata::from_document` (see API).
  `open365_commands::PdfOptions` (the `export_pdf` command, MCP, `open365 run`) still has no
  `pdf: open365_pdf::PdfOptions` field, so those paths cannot ask for PDF/A, PDF/UA or no
  bookmarks. The desktop (`crates/ui-egui`, `DocView::export_pdf`) needs a PDF/A, PDF/UA and
  bookmarks choice in the export dialog passed to `export_pdf_with`, and
  `PdfMetadata::from_document` behind a "Document properties" check box (default off), as Word has.
- `fixtures/reference/pdf` holds Word's 132 PDFs (15 MB) with Word's subset font data in them;
  regenerate with `tools/office/make-pdf-refs.ps1` (needs Word, local only).

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
