# Application shell (Writer, Sheets)

Status: first slice, 2026-10-11. Decision: ADR-012 in `decision-log.md`.

## Shape
- `crates/ui-shell`: Office-style chrome in egui, no document logic. Title row (Quick Access Toolbar, document
  name, search box, optional caption buttons), tab row, ribbon (groups, large/small/split/dropdown/combo/
  spinner/gallery controls, group labels, dialog launchers), Backstage (Home/New/Open/Info and direct
  Save/Save As/Print/Export/Close), status bar (page, words, view buttons, zoom slider), formula strip and sheet
  tab bar. `ShellHost` is the only coupling: `state(id)`, `run(id)`, `menu(id)`, `status_text(id)`, `central(ui)`.
  Commands are data: `spec/{writer,sheets}.json` (id, label, icon, size, kind); the handler key is the id.
- `apps/writer` (`open365-writer`): .docx/.dotx/.docm/.dotm, New blank, hosts `DocView`.
- `apps/sheets` (`open365-sheets`): .xlsx/.xltx/.xlsm/.xltm, New blank, hosts `GridView`.
- `apps/desktop` and `crates/ui-egui` are unchanged.

## Measurement (clean-room)
`tools/office/ribbon-oracle.ps1` starts its own visible Word/Excel (AutomationSecurity 3, Quit and exact-pid stop
in `finally`) and writes `fixtures/reference/ui/*.json` (structure, UIA rectangles, idMso label/enabled).
Screenshots stay out of the repo. Measured: scale 125 % only (the script must not change display settings), a
dark-theme machine, en-GB labels, Draw tab absent in Word, Automate tab present in Excel. `tools/gen_spec.py`
turns the reference into the spec: structure, intrinsic widths/gaps and row slots are stored; positions are
computed by `layout.rs`. Product names are renamed (Editor, SmartArt, WordArt, Office Clipboard...), geometry kept.

## Match against the reference (125 %, +-1 px)
Writer: tab row/title/status 36/36, groups 55/55, controls 191/191. Sheets: 36/36, 60/60, 213/213 (see
`cargo test -p open365-ui-shell --test geometry -- --nocapture`). Caveats: widths are measured per control (we do
not reproduce GDI text widths), so a new control needs a measured or estimated width; 100 % and 150 % are derived
(`round(u * scale / 1.25)`), not measured; the test does not yet cover QAT items, right-side tab-row buttons or the
Sheets formula bar and sheet tab rects (the layout computes them).

## Colours and fonts
Dark colours were sampled from the oracle window. Light ("Colorful") colours are approximations: measuring them
needs the Office theme changed, which the oracle must not do. Segoe UI is loaded from `%WINDIR%\Fonts` when present
(never bundled), else bundled Carlito. Sheets keeps `GridView`'s Carlito table (see API changes).

## Icons
Tabler Icons v3.49.0 (MIT), `assets/icons/tabler/` with LICENSE and README (tag + commit). Rasterised with resvg,
tinted at draw time. Controls without a mapped icon draw an initial in a rounded square.

## Open / not done
Tab row and title row do not yet respond to narrow windows (tab labels shorten, Comments/Editing turn into icons in
Office), custom caption buttons
exist but are off (apps use the native frame, no resize edges for a frameless window yet), Backstage content is
minimal (no templates, no Account/Options/Share), most tabs are present but disabled, no persisted recent list.

## API changes wanted in crates/ui-egui
1. `GridView::show_grid(ui, target)` without its formula bar and sheet tabs, plus `GridView::formula_bar_ui(ui, rects)`
   and sheet-tab accessors, so Sheets can set `ShellOptions { formula_bar, sheet_tabs }`. Until then the grid's own bar
   and tabs show.
2. A public font hook (`FontBook` or `GridView::with_ui_font`) so Sheets can use Segoe UI for the chrome.
3. Public `Target` access from `App` or a public `write_private_temp` (Writer re-implements it).
4. Character/paragraph formatting for sheets (bold, alignment, number formats) and font name/size for Writer.

## Update 2026-10-11 (second slice)
- Theme: both apps call `Mode::system()` (reads HKCU `AppsUseLightTheme`, read only; Office's default "Use system
  setting"). Dark tokens are sampled from oracle screenshots (kept in scratchpad): Word canvas 090909, page 262626 with
  8c8c8c outline, auto text white; Excel cells stay ffffff on e0e0e0 lines, headers 0a0a0a, selected header 646464 with
  217346 underline. **Light mode tokens are ESTIMATES** (the Office theme cannot be changed). `Theme::visuals()` sets egui.
- Sizes: `tools/office/ribbon-oracle.ps1` now records accessKey (UIA), default/min window, pixel-scan runs
  (`metrics.scan*`: Name Box, fx group, formula field, header sizes, cell pitch, Word page edges) and
  `-ResponsiveWidths` (`*-responsive.json`, ribbon at 1900..500 px). All at 125 % (measured); other scales derived.
  The reference machine keeps Excel's formula bar at 3 lines, so the expanded strip is measured, the one-line strip derived.
- Formula bar and sheet tabs are on for Sheets; `GridView::show_grid` replaces `show`.
- Key tips: `key`/`launcher_key` in the spec come from UIA AccessKey; multi-level Alt sequences work.
- Open: row header width for
  3+ digit rows, Word page shadow/ruler/gap, light theme sampling, Word's default window is maximised (1938x1158 px incl. frame).

## Update 2026-10-11 (responsive ribbon)
- Data: `tools/gen_spec.py` reads `*-responsive.json` (every tab at 1900..500 px) and writes, per group, its **states**
  (`Group` is state 0, the widest; `Group::states` are narrower layouts, each with `scale` large/medium/small/collapsed,
  its own blocks and width), and per tab `steps`: the ordered shrink steps (`[group, state]` lists). Controls keep one id
  across states (matched by idMso, else name), so key tips, icons and handlers are shared. A group whose label is wider
  than its content may squeeze (`min_w`, content `center`ed) by up to a few pixels before the next step.
- Rule (`layout::fit_groups`): groups get `lower width - 5 - RIBBON_SPARE_U (2)` units; take the next step while the
  groups do not fit even after squeezing; then squeeze elastic groups (rightmost first) by what is still missing. This
  reproduces every recorded state at 125 %. Other scales and widths between the recorded ones are interpolated by the
  same rule, not measured (steps are only observed at 50 px intervals). The widest state is state 0, so a tab shows its
  full layout from the widest window; the old base spec (the 1600 px state) is gone.
- Collapsed group: one group button (icon, name, arrow) filling the group. Click opens a popup below it with the full
  group (state 0), drawn over the document. Escape, a click outside, a tab change or running a command closes it; the
  button toggles it. Key tips: the group's own tip (`popup_key`, e.g. `ZF`) opens the popup and its controls' tips
  answer inside it. Tips are computed from the current layout (`Shell::visible_tips`), not from the spec.
- Tests: `tests/responsive.rs` (state class and x/width of every group, x/y/w/h of every control, every recorded
  width and tab, both apps, 125 %, 1 px), `tests/popup.rs` (open, toggle, Escape, outside click, click inside, key
  tips), `tests/tokens.rs` (one theme: only `accent` and the accent-derived `share` differ between the apps; the shared
  chrome geometry is identical).
- Reference quirk: Word's View/"Dark Mode" and Excel's Data/"Queries & Connections" groups change width between runs;
  this is the squeeze above (they leave exactly 2 units of spare room), not noise.
- Open: tab row, title row and status bar at narrow widths; scroll buttons when even the narrowest state does not fit
  (Office scrolls the ribbon below about 650 px); widths between recorded points; group icons for collapsed buttons
  are chosen by us (`GROUP_ICONS`), not measured.

## Update 2026-10-11 (narrow-window measurement, visual fixes)
- Oracle: `tools/office/ribbon-oracle.ps1 -ChromeWidths 1900..407:1 [-ChromeOnly] [-ChromeSettleMs 700]` records the title
  row (QAT children, title, search, user widget, caption buttons), tab row (tabs, Comments/Editing/Share, display
  options), status bar items and, for Excel, Name Box / formula bar / sheet tab control at each outer window width into
  `fixtures/reference/ui/<app>-chrome-responsive.json` (schema 2: `[name, id, x, y, w, h, offscreen]`, pane-relative,
  client width = outer width - 18). The script now aborts when COM hands it an Office process that was already running
  and only touches its own process in `finally`.
- Word findings at 125 % (25 px sweep, client width `cw`): the user widget and caption buttons are right-anchored
  (`me.x = cw - 243`); the search box is `min(460, floor(0.27 * cw))` wide, centred on `cw / 2 + 10` until the title
  reaches its 178 u minimum, then pinned at x = 517; below about `cw = 1037` it becomes a 58 u button anchored 10 u left of
  the user widget. Comments / Editing / Share collapse to icon forms (30 / 44 / 39 u) at `cw` 1007 / 1057 / 1107 and
  below. The Quick Access Toolbar drops items at `cw` ~707 (250 u), ~682 (202), ~632 (167), ~582 (35). The tab strip ends 7 u before
  Comments; tabs shrink together (equal cut, then the longest tabs are capped to a common width, remainder to the last tabs)
  from `cw` ~ 925 to 632, where every tab is 44 u wide and the strip stops compressing (Comments/Editing/Share stay
  at x 484/524/578). Status items leave in the order Accessibility, Text Predictions, Language, Coauthoring, Word
  count, Page, then Zoom % / zoom controls, as the right group (Focus .. Zoom %) pushes in; the right group stays anchored.
- Implemented in the next update (below).
- Visual fixes: Font box shows the document's theme font (`Calibri (Body)`; the blank document now uses theme fonts,
  `SpanStyle::font_label`), Name Box / formula text uses the chrome text colour (egui widget colours set in
  `Theme::visuals`), formula strip icons sized and coloured as measured (Cancel/Enter dim 0x83, fx and chevrons 0xd4),
  selected tab drawn semibold (second pass 0.5 px right), Styles gallery cells are 135 x 70 u with a 1 u ring 0xad and 0x38
  lift on the current/hovered cell, an unfocused window dims its title.
- Screenshot comparison (threshold 24, 1582 x 991, `OPEN365_SHOT`): Writer 5.22 % -> 5.17 %, Sheets 10.45 % (unchanged;
  its Font box is not wired to the active cell, so it shows the placeholder where Excel shows the cell font).

## Update 2026-10-11 (responsive chrome)
- `crates/ui-shell/src/chrome.rs` plans the title row, tab row and status bar in `u`; thresholds Office decides by its own
  arithmetic are data in `crates/ui-shell/spec/chrome-{writer,sheets}.json` (`AppSpec::rules`). `tests/chrome.rs` replays the
  1 px Word sweep (1494 widths, 73 290 checks, 1 px) and the Excel sweep when it is a schema 2 file.
- Rules (Word, client width `cw`; Excel values below): search box `w = min(460, 27 % of cw)`, `x = clamp(cw/2 + 10 - w/2, 517, user.x - 10 - w)` (integer
  halves), a 58 u button below `cw = 1040`; the title keeps 78 u: the user widget leaves first, then the search button. QAT stages
  (285/250/202/167/35 u) while `cw - width >= 433 + stage`; the first stage holds the title row as at `cw = 728`. Comments / Editing /
  Share become icons (30/44/39 u) at 1113 / 1067 / 1011. Tabs: the row ends 7 u before Comments, which stops at x = 484 (all tabs
  44 u). Shrink: 12 rounds of -1 u per tab right to left, then the widest tabs lose 1 u per round, the newest joiner first. Below
  `cw = 632` a 30 u "Column right" button appears; items at or right of its x are off screen. Status: left items stay while 9 u
  remain before the full right-hand group; Zoom %, then the zoom slider group leave when the group would start left of x = 10 (left
  items do not return). Word's 14 u size box is reserved by `status.grip`.
- Ribbon scroll: when the groups at their narrowest overflow the body (`content > body`), a 30 u "Column right" button sits at
  the body's right end, groups right of its x are off screen; `compute_scrolled` and the shell's two buttons scroll by 120 u.
- Excel (re-swept at 1 px, 1494 widths, 71 826 checks, 0 misses) differs only in data: QAT 298 u with the same stages, hold at
  `cw = 741`, search box pin x = 493 and box down to `cw = 1007`, two right-hand buttons (Share icon at 1086, Comments at 1040),
  status groups judged with a 13 u gap and only Zoom % leaving.
- Open: tab-row scroll button is drawn but inert, Excel Name Box / formula bar / sheet tabs at narrow
  widths, `Column left` geometry (no data), Writer page geometry (shadow, page gap, ruler ticks; not measured).

## Update 2026-10-11 (menus, combos, scroll buttons, Word page)
- Oracle: `ribbon-oracle.ps1 -App Excel -Popups [-PopupIds a,b]` opens the Home tab's menus, colour pickers, combo lists and
  first sub-menus through UIA ExpandCollapse and records `fixtures/reference/ui/excel-popups.json` (popup window rect, every
  row found by hit-testing a grid of points, because the NetUI popups have no UIA children, plus colour-run pixel scans).
  `-App Word -PageGeometry` records `word-page.json` (3 pages at 100 %, ruler off and on; the ruler is switched back in `finally`).
  A full `-Popups` run takes about 9 minutes. Sub-menu windows were not captured (their geometry is unmeasured).
- `crates/ui-shell/src/menu.rs`: `plan()` (pure geometry in u) and `show()` (draw, hover, keyboard, scroll, sub-menus). `MenuEntry`
  now has check marks, separators, headings, swatch rows (`EntryKind::Swatches`), tile grids, sub-menus, detail line, row height.
  Measured (125 %, u): frame+pad 2, row 36, heading 37/38, separator 3, colour cell 30 on a 34 pitch (tints 30), tile heading 37 and
  10 under each group, scroll bar 21, font list rows 33, size list rows 30 (16 shown), Number Format rows 60. `tests/menus.rs` replays
  the fixture (+-1 px, heights and every row/cell position; widths follow text and are not compared).
- Editable combos (`Shell::combo`): typed input, inline auto-complete (non-numeric lists), Enter commits through
  `ShellHost::combo_commit`, Escape cancels, arrow/Up/Down open the list, list scrolls to the checked entry.
- Sheets uses all of it; `format_popup.rs` and the host popups are gone. Writer's font name/size boxes are editable too.
- Tab row "Column right" / "Column left" and the ribbon scroll buttons scroll (`tests/scroll.rs`).
- Writer page (125 %, dark): first page 15 u under the canvas, page pitch 1421 u (A4 1403.2 + 18 u gap), 1 px 8c8c8c outline and no
  shadow in dark mode, page centred in the viewport minus a 21 u scroll bar. `PAGE_TOP_PT` 9, `PAGE_GAP_PT` 10.8. With the ruler on:
  horizontal ruler band 30 u (page moves down 30 u), vertical ruler 30 u (page moves right 15 u), half-cm ticks 6 u tall, quarter-cm 2 u.
- Open: Excel's "High-contrast only" filter strip (52 u) in colour pickers is not drawn; sub-menu offsets, popup text widths and
  the font list's maximum height are assumptions; light-mode menu colours are estimates; Writer has no ruler yet (numbers only in the
  fixture); light-mode page shadow unmeasured; Name Box / formula bar / sheet tabs at narrow widths not done.

## Update 2026-10-11 (caret, Format Painter clock, ruler measurement)
- Caret: `DocView` blinks like a Windows caret. Blink time and width come from `HKCU\Control Panel\Desktop`
  (`CursorBlinkRate`, `CaretWidth`; read only, via the safe `windows-registry` crate, cached: `ui-shell/src/caret.rs`; the
  workspace forbids `unsafe`, so not `GetCaretBlinkTime`; `Mode::system` reads `AppsUseLightTheme` the same way). Default 530 ms, 1 px; a negative rate means no blink. The cycle restarts on every key, click, caret
  move and focus change; the caret hides while the window is inactive; repaints are scheduled for the next toggle only. The cell
  editor and formula bar use egui's own blink, configured from the same settings in `Theme::visuals`. Time is egui's input time, so
  `tests/caret.rs` runs without a wall clock. Writer focuses the document at launch, after every ribbon command and when a menu,
  list, group pop-up, combo editor or Backstage closes (`ShellHost::chrome_closed`).
- Sheets: the Format Painter double-click window reads an injected `Clock` (tests freeze it); 20 consecutive runs pass.
- Ruler measurement: `ribbon-oracle.ps1 -App Word -RulerGeometry` writes `word-ruler.json` (every row 220..256, every column
  6..42; plain paragraph, and one with indents 36/18/36 pt and left/centre/right/decimal tabs at 144/216/288/360 pt). Findings at
  125 % (px, window): band y 222..252 with a 1 px 0x61 line at y 251; vertical ruler x 9..39 with a line at x 38; track fill 0x29,
  y 226..243 (vertical: x 13..31, from y 252); tick zero about 3 px left of the margin edge (426.2 for 429), half-cm pitch 23.62;
  half-cm ticks 6 high (y 233..238), quarter ticks 2 high (y 235..236), digits 9 high (y 231..239) at every cm inside the text
  area only, margins carry ticks without digits; vertical ruler digits are rotated; first-line marker 14 wide at margin - 8
  (flat 224..228, taper to a point at 234), hanging marker plus 0xbd box 245..250 under it, right-indent marker 14 wide at
  margin edge - 60 px - 6 with no box; tab glyph: 2 px stem y 238..243 ending at p-1, foot 2 px high y 244..245, 8 wide (left
  [p-2,p+6), right [p-8,p)) or 10 wide (centre/decimal [p-6,p+4)), decimal adds a 2x2 dot; corner shows only a left-tab glyph
  (stem 3x8 at x 20, foot 8x3). Not implemented yet.


## Update 2026-10-11 (Writer ruler)
- Reconciled the content top: the shell's content starts at y 222 (pane height), Word's first page outline is at y 243 with the ruler
  off and 273 with it on (band 222..252), so the page is 21 u under the canvas or band, not 15. `PAGE_TOP_PT` is now 12.6 (21 u).
- `crates/ui-egui/src/ruler.rs` (egui-free, `u`; 1 u = 0.8 points, 1.667 u per point at 100 %): `RulerInfo`, `horizontal()` /
  `vertical()` geometry, `hit_test`, `drag_edit`, `click_edit`, `preview`, `RulerEdit`. `ruler_view.rs` draws and handles the pointer
  (`DocView::set_ruler`, `ruler_info(&Target)` getter, `take_ruler_edit`). Measured (125 %, dark): band 30 u with a 1 u line on its
  last row; track y 4..22, x from margin - 1 to right margin - 2; tick zero = margin - 1.8, half-cm pitch 23.62 u; tall tick rows
  11..17, quarter 13..15, digits 9..18 (text area only); markers 14 u wide centred on the position: first-line pentagon rows 2..13
  (flat to 8, then taper), hanging/right pentagon rows 13..23, left-indent box rows 23..29; tab glyph stem 2 x 6 (rows 16..22),
  foot 2 high (rows 22..24), left [p-1,p+7), right [p-7,p+1), centre/decimal [p-5,p+5), decimal dot [p+3,p+5) x rows 19..21;
  corner selector stem 3 x 8 at x 10, foot 8 x 3. Vertical ruler: same numbers turned on their side (not in the fixture; digits
  rotated, unmeasured). With the ruler the page moves 30 u down and 15 u right (centred in the area minus the 30 u vertical band).
  View > Show > Ruler toggles it (`view.show.ruler`); Writer starts with it on. Tests: `ruler::tests` against `word-ruler.json`
  (+-1 px), `tests/ruler.rs` (page shift, info getter).
- Open: dragging and clicking compute the edit and preview it, but `open365-commands`/`open365-docx` have no paragraph indent or tab
  stop command, so `apply_ruler_edit` only hands the edit to the host (Writer shows it in the status line). Needed: commands
  `DocxSetIndents { para, left_pt, first_line_pt, right_pt }`, `DocxAddTab { para, pos_pt, kind }`, `DocxMoveTab`, `DocxClearTab`
  and the matching `ppr.rs` setters (`w:ind`, `w:tabs`). Light-mode ruler colours are estimates.
- Combo editors: `Shell::combo_edit()`; two bugs fixed (`tests/combo.rs`): Enter never committed (a single-line egui TextEdit
  gives up focus on Enter itself, so `has_focus()` was already false), and the first typed letter did not auto-complete.
  Open: after an inline completion the headless harness resets the cursor to 0 on the next frame, so a second typed letter is
  inserted at the start; not yet checked in the real window.

## Update 2026-10-11 (ruler commands, Merge & Centre)
- Ruler edits run through the command layer: `DocxSetIndents`, `DocxAddTab`, `DocxMoveTab`, `DocxClearTab`
  (`crates/commands`), setters in `crates/docx/src/ppr.rs` (`w:ind` left/right plus `firstLine` or `hanging`; `w:tabs` kept
  sorted; clearing a style-inherited stop writes `w:val="clear"`). `DocView::apply_ruler_edit` dispatches them and refreshes the
  paragraph; `take_ruler_edit` is gone. One undo step each. Writer has no Paragraph dialog yet, so no dialog fields are wired.
  Only the caret paragraph is edited (Word applies to the whole selection).
- Merge & Centre: `Command::MergeCells { sheet, range, mode: MergeMode }` (`MergeCenter`, `Across`, `Cells`, `Unmerge`);
  model in `crates/xlsx/src/workbook/merge.rs` (`Workbook::merge_cells`, `merge_discards`) over `mergeCells`. Only the
  upper-left value of each area stays (others are emptied with recalculation), Merge & Centre also centres, Unmerge keeps value and
  alignment, an overlapped merge is replaced. Sheets: the split button toggles merge/unmerge and lights while a merged cell is
  selected; the four menu items; a modal asks "Merging cells only keeps the upper-left value and discards other values." (OK/Cancel,
  Enter/Escape) when values would be lost. Not measured: covered-cell styles after a merge, menu item icons and row geometry.
- Combo inline-completion caret (checked, NOT fixed): in `egui 0.36.2` the text edit's caret is 0 at the start of the frame after a
  completion although `TextEditState` held (1, 7) before it and the pointer was removed, so the next letter lands at index 0. It
  also resets on idle frames. The cause was not found in the text edit code; a cut-back-and-set-caret workaround did not help, so the
  shell is unchanged. Real window not checked.

## Update 2026-10-11 (combo caret, tile fonts, Excel UIA staleness, spell command)
- Combo caret bug, ROOT CAUSE FOUND: not `TextEditState`. `Galley::clamp_cursor` clamps every cursor to the glyphs of the laid-out
  row, and this workspace builds egui without default fonts, so a headless harness with no font has glyph-less rows and resets
  the cursor to index 0 on the frame after every edit (a bare `egui::TextEdit` does it too). With `fonts::install` in the rig the
  selection (typed part .. completed suffix) survives idle frames and the next letter continues the completion. Fix is in
  `tests/combo.rs` (rig installs the UI font; two new tests). The shell code needed no change. Real window not run by us.
- Cell Styles tiles: `Tile.font: Option<TileFont { family, fake_bold, fake_italic }>`; `GridView::preview_face(name, bold, italic)`
  registers the real face (system font, else bundled Carlito/Caladea) through the grid's `FontBook` and returns the egui family
  (one pass later; until then the UI family with the weight faked by overstrike and the slant by egui). `menu.rs` draws the tile
  text with that family. Test: `cell_style_tiles_show_the_style_look` area in `apps/sheets/src/format_tests.rs`.
- Excel narrow chrome, measurement: the UI Automation rectangles of Name Box, formula bar and sheet-tab control are STALE (they keep
  the first layout's rectangles at every width, also when the elements are found again for every reading), so re-finding does not
  fix it. `ribbon-oracle.ps1 -ChromeWidths` now records, for Excel, the visible Win32 child windows (`EXCEL;` Name Box host,
  `ComboBox`, `EXCEL<` formula bar, `XLDESK`, `NUIScrollbar`, `XLCTL` split handle) and two colour-run rows through the sheet tab card
  and underline (`tabs`), schema 3. Spot readings at 125 % (window width -> formula bar width, tab strip end, XLCTL x, horizontal
  scrollbar): Name Box host stays 274 u and the combo 125 u at every width; the formula bar is `win - 292`; the sheet-tab strip ends at
  about `0.6 * client - 15` (XLCTL x = 1123 at 1900, 763 at 1300, 583 at 1000, 463 at 800, 373 at 650, 313 at 550, 271 at 480, 227 at 407); the
  horizontal scroll bar starts 40 px right of that and ends at the vertical scroll bar; at 407 the Sheet1 tab no longer fits and is
  not drawn (only the scroll and add buttons stay). The existing layout already follows the first two (Name Box fixed, field
  right-anchored). `tests/chrome.rs` accepts schema 2 or 3. The full 1 px Excel re-sweep was started into the scratch folder and is
  NOT yet committed to `fixtures/reference/ui/`; the committed `excel-chrome-responsive.json` is still the stale schema 2 file.
  Excel sweep (61 widths, 3352 checks) now replays with 0 misses. Rules: search box pins at x=501 (`pin_x`) and is shown from
  client width 1032 u (box fits beside the user widget; below it the 58 u button, title keeps the rest); the Sheet1 tab is
  clipped to `split_x - tab.x - 74` u (card = tab - 11 u) and dropped once the card is <= 20 u.
- Spell: command `DocxCorrectWord { para, start, end, expected, replacement }` (Destructive; fails closed with
  `CommandError::Proofing` when the paragraph no longer holds `expected` at `start..end`, or the replacement has a line break; one
  undo step; `tests/correct_word.rs`). Open: squiggle painting, `w:lang` spans, the right-click menu (5 suggestions, Ignore All, Add to
  Dictionary) in Writer and the Sheets cell editor, and the Word squiggle measurement (colour and geometry stay unmeasured).


## Update 2026-10-11 (Excel narrow chrome, spell squiggles)
- Excel narrow chrome in `layout.rs`: formula bar `win - 292` (always to the pane edge), Name Box fixed, `Layout.sheet_split`
  (Excel's XLCTL, x = floor(0.6 * pane - 15) u, 40 u wide) and `Layout.hscroll` (from split + 40 u to 23 u before the pane edge).
  Sheet1 is dropped when its right edge passes the split (hidden at 407); Add Sheet then moves to Sheet1's x (assumption, unmeasured).
  `shell.rs sheet_bar` follows the split. `tests/chrome.rs` replays the Win32 child rectangles and the Sheet1 card.
  The full 1 px re-sweep (1900..407) did NOT finish (about 35 min, no output; the oracle writes only at the end), so it was stopped.
  `fixtures/reference/ui/excel-chrome-responsive.json` is now a schema 3 file with only 3 widths (1900, 1000, 407).
  Open: re-run `ribbon-oracle.ps1 -App Excel -ChromeOnly -ChromeWidths 1900..407:25` (or :1) and commit; the horizontal scroll bar
  rect is in the layout but the grid still draws its own egui scroll bars.
- Spell: `crates/ui-egui/src/proofing.rs` (`ProofState`, async `ProofWorker`, squiggle, menu data). `w:lang`/`w:noProof` runs come
  from `docmodel::LangRun`; default language en-GB. Writer: squiggles in `paint_page`, right-click menu (<= 5 suggestions, Ignore All,
  Add to Dictionary) replaces through `DocxCorrectWord`. Sheets: the same in the cell editor text box (formulas are not checked).
  Squiggle measured with `fixtures/reference/ui/word-squiggle.json` (own Word instance): triangle wave, period 6 px, height 4 rows,
  independent of zoom, `ff4a4a` on the dark theme (light colour unmeasured), top about baseline + 0.109 em - 1 px.
  Open: user dictionary is in memory unless the app calls `ProofState::set_user_dictionary_path`.
