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
- NOT implemented yet: the layout still places the tab row, title row and status bar at full width. The measured rules
  above and the 1 px sweep fixtures are the input for that work (see Open).
- Visual fixes: Font box shows the document's theme font (`Calibri (Body)`; the blank document now uses theme fonts,
  `SpanStyle::font_label`), Name Box / formula text uses the chrome text colour (egui widget colours set in
  `Theme::visuals`), formula strip icons sized and coloured as measured (Cancel/Enter dim 0x83, fx and chevrons 0xd4),
  selected tab drawn semibold (second pass 0.5 px right), Styles gallery cells are 135 x 70 u with a 1 u ring 0xad and 0x38
  lift on the current/hovered cell, an unfocused window dims its title.
- Screenshot comparison (threshold 24, 1582 x 991, `OPEN365_SHOT`): Writer 5.22 % -> 5.17 %, Sheets 10.45 % (unchanged;
  its Font box is not wired to the active cell, so it shows the placeholder where Excel shows the cell font).
