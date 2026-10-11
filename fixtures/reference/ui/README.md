# UI reference (chrome geometry)

`word-ribbon.json` and `excel-ribbon.json` are written by `tools/office/ribbon-oracle.ps1`: UI Automation
structure and bounding rectangles (physical pixels, window-relative) of the ribbon, title row, status bar,
formula bar, sheet tabs and Backstage navigation, plus idMso label/enabled state. Measured at 125 % scale
(120 dpi) in a 1600 x 1000 px window, Office 16.0 build 16.0.20430, dark theme. User and document names are
scrubbed. Screenshots are never stored here (they contain Microsoft artwork).
`crates/ui-shell/tests/geometry.rs` compares the shell layout with these files.

`word-responsive.json` and `excel-responsive.json` record the ribbon of every tab at window widths 1900..500 px (step 50)
at the same scale. `tools/gen_spec.py` derives the per-group states and shrink steps from them;
`crates/ui-shell/tests/responsive.rs` compares the layout with every recorded width.

`word-chrome-responsive.json` and `excel-chrome-responsive.json` record the title row, tab row and status bar (Excel: also
Name Box, formula bar, sheet tab control) at window widths swept by `ribbon-oracle.ps1 -ChromeWidths`; see
`docs/architecture/app-shell.md` (narrow-window measurement). No test consumes them yet.
