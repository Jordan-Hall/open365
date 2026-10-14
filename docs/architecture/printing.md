# Printing (File > Print)

Status: first slice built 2026-10-12. Decision record: ADR-018 in [`decision-log.md`](decision-log.md). The UI platform ADR left printing as "PDF from `open365-pdf`, then the OS print path; needs COM or WinRT, so the platform crate" (`adr-ui-platform.md`, platform integration table, "Printing hooks"). This slice decides that path and builds the page.

## What exists

| Part | Where | What it does |
|---|---|---|
| Print back end | `crates/print` (`open365-print`) | Page-range parsing, paper and margin tables, `PrintSettings` and the page sequence (copies, collate), page surgery on a PDF (`compose`), an "ink map" of a page for the preview, and the `PrintSystem` trait with the OS and in-memory implementations. |
| Print page | `apps/preview/src/print/` | `PrintModel` (state, no drawing), `PrintPage` (the Backstage page on `ui-core` widgets), `PrintSource` (`WriterSource`, `SheetsSource`). |
| Tests | `crates/print/tests`, `apps/preview/tests/print_backstage.rs` | Page selection, copies, N-up, scale, ink; the page driven through the headless harness against a `MemoryPrintSystem`. No test sends a real job. |

## Data flow

1. The open document renders a PDF of its pages. Writer: the command layer's `share_send_pdf` (layout, then `open365-pdf`), which does not touch the document. Sheets: the sheet pager and its PDF writer (`SheetModel::pdf_bytes`).
2. `PdfSource::load` reads that PDF. The preview draws from it, so it shows the pages the printer receives.
3. `PrintSettings::sequence` turns the choices (range, copies, collate) into a list of source pages. `compose` writes a new PDF: one page per sheet, each source page a Form XObject placed with `PdfSource::placements` (N pages per sheet, scale down to fit, centred). A page printed three times is stored once. Links, outline and structure tree are dropped.
4. `PrintSystem::submit` sends the PDF with the job attributes. Only the Print button calls it.

Settings that change the layout reach the document through `PrintSource` and re-render: paper size, orientation and margins (Writer: `DocxLayout` ops on every section, so the document changes as it does in Word's Print page; Sheets: the worksheet page setup) and scaling (Sheets: fit sheet, fit columns, fit rows, custom percent, through the existing `set_fit_width`, `set_fit_height`, `set_print_scale`). Range, copies, collate, sides and pages per sheet only change the job.

## Print page contents

Print button, Copies, Printer list (default marked, driver note), pages to print (All, Current Page, Custom with a Pages box), sides (one-sided, long edge, short edge), Collated or Uncollated, Portrait or Landscape, paper size, margin presets (Writer: Normal, Narrow, Moderate, Wide, Mirrored; Sheets: Normal, Wide, Narrow), then for Sheets No Scaling, Fit Sheet on One Page, Fit All Columns on One Page, Fit All Rows on One Page, Custom Scaling Options with a percentage box; for Writer 1, 2, 4, 6, 8 or 16 pages per sheet. The right side previews one sheet with Previous page and Next page, and "n of m".

## OS hand-off and its limits

`OsPrintSystem` uses the `printers` crate (MIT).

* Unix, CUPS: the PDF is sent with IPP attributes `document-format=application/pdf`, `sides`, `media` and `print-scaling=none`. CUPS converts PDF for the queue. Not exercised on this machine (Windows only).
* Windows, winspool: the PDF is written with `WritePrinter` as a RAW job. That prints only on a queue whose driver reads PDF or PostScript. `PrinterInfo::reads_pdf` is true only when the driver name says PostScript or PDF, and the page shows a note otherwise. Queues that need GDI or XPS (most inkjets, "Microsoft Print To PDF") will not print a RAW PDF. `sides` and `media` are not sent; `SubmitReport::ignored` says so and the page tells the user.

Not done, in order of value:

1. Windows graphics path: render the PDF pages to an XPS or GDI job through the print ticket (duplex, tray, paper from the driver's capabilities). It needs COM (`XpsPrint`, `PrintTicket`) or a rasteriser (pdfium, BSD-3 and Apache-2.0, binary), so it is an `unsafe` exception decision for the platform crate, or a sandboxed helper process. Owner decision.
2. Printer Properties and the system print dialog.
3. Real printer capability lists (paper sizes, trays, duplex) instead of the fixed paper table.
4. Glyph-accurate preview. The preview shows text lines as bars, with rules, fills and pictures at their places, read from the PDF content stream. A glyph preview needs the page drawn through `gfx` from the layout.
5. Print Selection (Writer), Print Entire Workbook and Print Selection (Sheets), Page Setup dialog link, print preview zoom, Ctrl+P and the Backstage Print item routing (see the hooks in the work report).
6. Agent control: a `print` command in the command layer with confirmation and audit, dry run through `MemoryPrintSystem`.

## Behaviour checks still open (no Office run was made for this slice)

These follow public Office documentation and common behaviour; none is measured against a running Office yet.

* Page-range text: comma or semicolon separators, `a-b`, open-ended `a-`; pages print once, ascending, however listed. Word may print in the order typed, and may reject `a-`.
* Copies upper limit (32 767 here) and the page cap per job (50 000 here).
* Margin preset values (Writer: Normal 1 in; Narrow 0.5 in; Moderate 1 in top and bottom, 0.75 in sides; Wide 1 in and 2 in; Mirrored 1 in with 1.25 in inside) and Sheets (Normal 0.75 in top and bottom, 0.7 in sides; Wide 1 in; Narrow 0.75 in and 0.25 in).
* N-up grids (2 = 2x1, 4 = 2x2, 6 = 2x3, 8 = 2x4, 16 = 4x4) and that a page is never enlarged.
* The wording of the dropdown items and of the "n pages on m sheets of paper" line.

## Dependencies

| Crate | Version | Licence | Used for |
|---|---|---|---|
| `printers` | 2.3.0 | MIT | Printer list and RAW/CUPS job submission. Contains `unsafe` for winspool and CUPS FFI; our crates stay `unsafe_code = "forbid"`. Single maintainer. |
| `lopdf` | 0.45.0, no default features | MIT | Reading the PDF, building the job PDF. Pulls `aes`, `cbc`, `ecb`, `md-5`, `sha2`, `rand`, `getrandom`, `nom`, `flate2`, `weezl`, `encoding_rs`, `stringprep`, `thiserror`, `indexmap`, `rangemap` and others, all MIT, Apache-2.0 or BSD-3 (`cargo deny check licenses` passes; no licence added to `deny.toml`). |

`lopdf` parses a PDF this process wrote, and the code treats it as untrusted anyway: content decoding is limited to 64 MiB and the ink map to 4 000 items per page.
