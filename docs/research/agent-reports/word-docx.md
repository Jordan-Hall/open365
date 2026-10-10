# Word/DOCX engine strategy and fidelity testing (2026-10-10)

Legend: [F] verified fact (source below), [I] inference, [U] unverified.

## 1. Document model
Model must cover: [I]
- Paragraphs/runs with style inheritance (docDefaults -> table style -> paragraph style -> character style -> direct formatting), plus themes (fonts/colors resolved late).
- Numbering (abstractNum/num, level overrides, restart), sections (page size, columns, header/footer refs, titlePg, even/odd).
- Tables: grid, gridSpan/vMerge merged cells, nested tables, conditional table-style bands.
- Fields (simple and complex, cached result vs instruction), headers/footers, footnotes/endnotes, comments, tracked changes (ins/del/moveFrom/To, rPrChange, pPrChange), bookmarks, content controls (SDT), images/anchors/text boxes (DrawingML, VML fallback), OLE/embedded objects, equations (OMML), settings/compat flags.

Lossless-preservation strategy [I]:
- Keep the package as a part graph: every part and relationship retained byte-for-byte unless its model owner edited it.
- Typed model nodes carry an `extras` bag: unrecognised child elements/attributes stored as raw XML (namespace-preserving), with a position index so order is retained. Unknown namespaces and `mc:AlternateContent` stored whole.
- Unknown parts (customXml, vbaProject, OLE bins, unknown media) kept as opaque blobs; relationships rewritten only by ID remap.
- Save invariant: any unmodelled input byte-range either reappears or is reported in a "dropped content" log that blocks silent save (user-visible warning). Dirty-part tracking: re-serialise only edited parts.
- rsid/`w14:paraId` attributes preserved as extras.

## 2. docx-rs vs own OOXML layer
Facts:
- docx-rs: MIT [F], crate 0.4.22, updated 2026-07-21 [F], ~3.6M downloads [F], 557 stars, 842 commits [F]. Has `read_docx` parsing into the writer's model [F]. README: "support is not exhaustive" [F].
- Round-trip fidelity of unknown elements: not documented; [U] (needs test, likely drops unknowns since reader maps into typed model).
- Alternative Rust crates: not evaluated against primary sources here [U]. `ooxmlsdk` appeared in search (docs.rs 0.4.0) as an SDK-schema-derived crate; maturity [U].

Recommendation [I]: do not make docx-rs the core model. Build own thin OOXML layer (zip + quick-xml + part graph + extras bag) with typed nodes for the supported subset. Optionally use docx-rs only as a comparison oracle in tests (reads/writes) and for cheap prototype. Evidence that would change this: a spike showing docx-rs preserves unknown XML on round-trip over the corpus.

## 3. Layout engine
- Pieces: paragraph shaping (harfrust/skrifa/parley/cosmic-text, per prior research), line breaking UAX#14 (`unicode-linebreak`/ICU4X [U versions]), bidi UAX#9, hyphenation (Knuth-Liang patterns; Word hyphenation is off by default so low priority), widow/orphan and keep-with-next/keep-lines, page-break-before, tables split across pages (row split, repeated header rows, cantSplit), footnote placement, floating objects with wrap (square/tight/through/top-bottom), headers/footers, columns, field page numbers (iterative pagination).
- Expensive Word-parity areas [I]: tight/through wrap polygons, table autofit and column width algorithm, justification/kerning/compat-flag differences, footnote-vs-page interaction, line height from font metrics (platform font substitution), field-driven reflow (TOC, NUMPAGES). Exact Word pagination parity is not achievable without Word as oracle; set a target of match-within-N-lines per page, not pixel parity.
- Incremental relayout [I]: cache shaped lines per paragraph (key: content+style hash+width); relayout from dirty paragraph until page-break state converges with previous layout (stop when a page start matches old state); background layout of pages beyond viewport; estimated heights for scrollbar.

## 4. Fidelity test strategy
Corpus [I]:
- User-made fixtures (Word, LibreOffice, own tool), each exercising one feature; committed with provenance notes. Cleanest licence.
- Public corpora: Apache POI test-data directory exists in repo root [F]; its licence for individual test files not stated on the pages fetched [U] (POI code is Apache-2.0 [U, not fetched]). LibreOffice core test documents: licence per file [U]. Treat both as local-only, non-redistributed, licence-audited per file before committing; also many contain third-party/real-world content.
- Clean-room caution: using third-party files as inputs is fine; do not copy their implementation.

Invariants [I]:
1. Canonical XML diff (C14N, attribute order, ignorable whitespace) for unedited parts: open->save must be identical for parts not touched.
2. Semantic model diff: parse(save(parse(x))) == parse(x).
3. Unknown-part preservation: set of parts, content types, and rel targets superset of input.
4. Relationship integrity: every r:id resolves; no orphans; content-type overrides present; unique IDs for bookmarks/comments/drawings.
5. Visual: render to PDF both ways via LibreOffice sidecar (soffice --headless --convert-to pdf) and compare page count/ text positions with tolerances; Word is a manual/optional oracle on a Windows runner. LibreOffice is itself imperfect oracle: report "diffs vs LO" not "vs Word" [I].
6. Fuzz: cargo-fuzz targets on zip entry parsing (zip bombs, path traversal, duplicate names) and XML parsing (entity expansion/DTD rejected, depth limits) and model->serialise->parse. cargo-fuzz exists as rust-fuzz tool [U, not fetched].
7. Perf targets (propose, to be measured on a named machine) [I]: open 500-page/1M-char document < 2 s to first page; keystroke relayout < 16 ms median; peak RSS budget set after baseline.
8. Silent-corruption: automated "opens in LibreOffice without repair" (convert succeeds) plus Open XML SDK validator. SDK is MIT [F: search result, NuGet/MS note]; OpenXmlValidator is part of SDK [F]; the Productivity Tool's licence was not stated and its source was excluded from the 2.5 release [F]; use the SDK's validator via a .NET CI helper only after confirming the current repo LICENSE [U]. Word-opens check requires licensed Word, manual.

## 5. Smallest vertical slice
Scope: open .docx -> edit text in paragraphs with bold/italic and one heading style -> save -> reopen.
Acceptance:
- 10 user-made fixtures (plain, styles, list, table, image, header/footer, comment, tracked change, footnote, unknown custom XML part): open without error.
- Edit text in a paragraph; save; every untouched part is byte-identical (zip entries compared); unmodelled features (comments, tracked changes, images, tables) round-trip with canonical XML equal.
- LibreOffice opens saved file and converts to PDF; text content equals original's PDF text.
- OpenXmlValidator reports no new errors vs original.
- Fuzz target runs 1 h without panic/OOM; zip-bomb and DTD inputs rejected.
- A dropped-content log is empty for the fixtures, and non-empty for a deliberately unsupported element test.
- Rendering: single-column pages with paragraphs and basic tables; no floats.

## Sources (accessed 2026-10-10)
- https://github.com/bokuweb/docx-rs (MIT, read_docx, "not exhaustive")
- https://crates.io/api/v1/crates/docx-rs (0.4.22, 2026-07-21, MIT)
- https://github.com/apache/poi (test-data dir; licence not stated on page)
- https://learn.microsoft.com/en-us/archive/blogs/officeinteroperability/open-xml-sdk-version-2-8-1-is-available (SDK MIT)
- https://msdn.microsoft.com/library/dd633612(office.12) (OpenXmlValidator, 2009)
- https://www.ericwhite.com/blog/?p=2757 (Productivity Tool source excluded)
- https://docs.rs/crate/ooxmlsdk/0.4.0 (alternative crate, not evaluated)
