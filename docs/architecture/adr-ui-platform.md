# ADR-UI-PLATFORM Own UI layer on winit + wgpu + AccessKit

Status: Accepted (lead, 2026-10-11). Supersedes the egui/eframe part of ADR-004 in `decision-log.md`. The rest of ADR-004 stays: engine-first design, thin replaceable UI crate, commands through the command layer (ADR-002), own AccessKit subtree for the canvas and grid, IME and printing owned by us.

## Context

The product bar is pixel parity with Office (1 px tolerance) for both chrome and text. ADR-004 chose egui/eframe for speed. Two things in egui sit in the way of that bar:

- Glyph rasterisation, subpixel positioning, hinting and text blending belong to `epaint`. We cannot make them match DirectWrite ClearType output without forking epaint.
- Layout and sizing are egui's (spacing, rounding, stroke snapping). Office draws chrome on whole device pixels at 100%, 125%, 150% and 200% scale. egui paints in float points and snaps by its own rules.

The workspace already owns the hard text parts: `open365-text` (fontdb, harfrust shaping, UAX #14 breaks) and `open365-layout` use skrifa for metrics. What is missing is a renderer that consumes skrifa outlines directly, plus a window host.

Facts checked on 2026-10-11 against the dependency tree: eframe 0.36.2 resolves wgpu 30.0.1, winit 0.30.13, accesskit 0.24.1 and accesskit_winit 0.32.2. The new crates use exactly those versions, so the tree holds one wgpu and one winit (`cargo tree -d` shows neither duplicated).

## Alternatives

Assessments below are design-time judgements from the documented behaviour of each project, not benchmarks we ran. The "would change" section lists what to measure.

| Option | Why not (or not now) |
|---|---|
| Stay on egui | Text and chrome pixels are not ours to control (see Context). Every parity fix becomes an upstream or fork problem. Immediate mode also fights virtualised document layout. |
| iced (wgpu, cosmic-text) | Also owns its text stack and widget metrics. Elm-style retained tree would be a rewrite of ui-shell with the same parity ceiling. |
| vello (GPU compute vector renderer) | Strong for vector shapes and charts, but analytic antialiasing without a ClearType-style LCD mode, and it needs compute-capable GPUs. Revisit for chart and shape content in the document canvas, composited under our text. |
| Slint | Dual GPLv3 / royalty-free with conditions / commercial. Conflicts with an MIT product (ADR-008). Own text rendering as well. |
| Raw ash (Vulkan) or glow (GL) | We would rebuild device selection, swapchain handling and Metal/DX12 paths that wgpu already gives. wgpu is the right floor: one API over DX12, Vulkan and Metal, already in the tree. |
| Skia bindings | C++ build and licence surface; its text rasteriser would again be someone else's. Not evaluated further. |

## Decision

Replace egui/eframe with our own layers, staged so egui keeps building until the new layer passes the same tests:

- `open365-gfx` (`crates/gfx`): wgpu 2D renderer. Rects, rounded rects (SDF), exact-snapped lines, clip stack (scissor), images, SVG icons (resvg to a coverage mask in an atlas), glyph atlas from skrifa outlines with hinting and subpixel x-bins, grayscale AA by default and an LCD path behind `TextOptions::lcd`, blending in linear light (sRGB render targets), offscreen render with RGBA readback.
- `open365-ui-host` (`crates/ui-host`): winit 0.30 host. Window, wgpu surface, per-monitor DPI, on-demand redraw with timed wake-ups, AccessKit adapter, IME plumbed into the `ImeHandler` trait, light/dark title bar.
- Later: `ui-shell` stops depending on egui and draws through `gfx`; grid and document canvas are drawn by `gfx` directly.

Design choices inside phase 1, each with the reason:

- **Device-pixel draw list.** All `DrawList` coordinates are device pixels. Callers convert from logical units, so snapping is explicit and testable. `snap_rect`, `snap_stroke` and `split_subpixel` are pure functions with unit tests.
- **Own exact-area rasteriser.** skrifa produces (hinted) outlines, not bitmaps. `raster.rs` accumulates signed area per pixel (FreeType/font-rs approach), so a pixel-aligned edge gets exactly 100% or 0%. About 250 lines, no new dependency.
- **Hinting.** `Hinting::Normal` uses skrifa's `Engine::AutoFallback` in smooth mode: the font's own TrueType instructions when present (vertical movement only in smooth mode, comparable to ClearType natural), the auto-hinter otherwise. Measured on bundled Carlito `H` at 16 px: hinting snaps the top serif row and bar vertically and leaves horizontal positions alone.
- **Subpixel x positions.** Four bins by default (`TextOptions::subpixel_bins`), baseline y rounded to a whole pixel, bitmap quads placed on whole pixels.
- **LCD without dual-source blending.** LCD glyph batches are drawn three times, once per colour channel (write mask R, G, B) with that channel's coverage as alpha. Works on every wgpu backend. Requires an opaque background. Filter is FreeType's default 5-tap FIR, a placeholder until fitted to DirectWrite captures.
- **Gamma.** Targets are `*Srgb` formats, so the GPU blends in linear light. `RenderParams { text_gamma, text_contrast }` shape text coverage in the shader; defaults are the identity. Fitting them to Office captures is phase 2 work.
- **CPU mirror of atlases.** Pixels live in `Atlas` on the CPU and only dirty regions upload. Draw lists, glyph rasterisation and atlas packing therefore test without a GPU.

## Consequences

- We own text pixels and chrome pixels. Parity bugs become fixable in our code.
- We own widgets, focus, hit testing, layout of chrome and event routing. That is real work (phase 2) and the largest risk to schedule.
- No immediate-mode escape hatch: debug UIs and dialogs need our own widgets.
- Platform integration moves from egui-winit to us (list below). Several items need Win32 calls that are `unsafe`; the workspace forbids `unsafe`, so they are deferred to a platform crate that needs its own decision (see "Platform integration").
- Test strategy moves from `egui_kittest` to a headless harness on `gfx::Offscreen` (phase 4).

## Confidence and what would change the decision

Confidence: medium-high that this is the right base, medium that the schedule holds.

Evidence that would change it:

- Phase 2 shows chrome pixel diffs against Office captures cannot get inside 1 px using this renderer (for example rounded-rect SDF edges or text gamma are not tunable to match). Then reconsider vello for shapes while keeping our text.
- wgpu cannot give acceptable frame times on the target Windows hardware for the grid at 4K (measure on the documented test machine before claiming).
- AccessKit's Windows adapter cannot expose the text ranges NVDA and Narrator need for the canvas. Then fix upstream (planned anyway); fall back to a UIA-direct layer for the canvas only.

## Migration phases

Each phase ends with its tests green and egui still building until phase 5.

1. **gfx and host (this change).** Crates, ADR, example `hello`. Exit: renderer unit and GPU tests pass; example window opens, draws rect, text, icon, caret blink, IME and AccessKit hookup.
2. **Chrome (`ui-shell`).** Retained widget tree, layout, focus, theming tokens, ribbon, backstage, status bar, formula bar, tab strip; draw through `gfx`. Fit `RenderParams` and the LCD filter to Office captures. Alt key tips, high contrast, caret metrics (platform crate). Exit: chrome screenshots within 1 px of the capture set at 100%, 125%, 150%, 200%.
2b. **Phase 2 status (2026-10-11): done in new code, `ui-shell` not yet ported.** Evidence below under "Phase 2 evidence".
3. **Grid and document canvas.** Virtualised grid and paged document drawn by `gfx` from the layout crates; AccessKit subtree for cells and text runs; IME at the caret. Exit: the existing grid and canvas tests pass on the new path.
4. **Headless test harness replacing `egui_kittest`.** Event injection, semantic queries from the AccessKit tree, pixel snapshots through `Offscreen::read_rgba`, golden image diff with tolerance. Port the kittest tests; keep them as the acceptance set for phase 5.
5. **egui removal.** Delete `ui-egui`, remove `eframe`, `egui`, `egui_kittest`, `rfd`-through-egui paths, `BSL-1.0` from `deny.toml` if nothing else needs it, and the `app-shell.md` egui sections. Exit: `cargo tree` shows no egui crates.

## Platform integration

Status is as of phase 1. "Gap" means documented, not hidden.

| Item | Mechanism | Phase 1 state | Next |
|---|---|---|---|
| IME | winit `Ime` events to `ImeHandler` (`preedit`, `commit`, enable, disable); `set_ime_allowed` and `set_ime_cursor_area` follow `wants_ime` and `cursor_area` after every frame | Done and unit-tested (translation, ordering). Not yet tested with a real IME | Preedit segment styling needs TSF-level data winit does not expose; evaluate a TSF text store in the platform crate in phase 3 |
| Clipboard | Text via a permissive crate (arboard, BSL-1.0, already allowed); rich formats (CF_HTML, RTF, OOXML parts) and delayed rendering via Win32 | Not started | Phase 2 text, phase 3 rich formats |
| DPI per monitor | winit 0.30 runs PerMonitorV2 on Windows. `ScaleFactorChanged` and `Resized` reconfigure the surface; `Cx::scale_factor` is passed to the app; draw lists are in device pixels | Done. Not yet exercised across two monitors | Chrome metrics must be authored in logical units and snapped per scale in phase 2 |
| Caret blink time and width | `HKCU\Control Panel\Desktop` `CursorBlinkRate` and `CaretWidth` (the values `GetCaretBlinkTime` and `SPI_GETCARETWIDTH` return), read with the safe `windows-registry` crate | Done in `open365_ui_core::settings`; `SystemMetrics::read()` uses it, fallback 530 ms and 1 px elsewhere. No `unsafe`, no exception needed. Changes made while the app runs are not observed yet | Re-read on `WM_SETTINGCHANGE` (needs the window message hook from winit) |
| High contrast | `HKCU\Control Panel\Accessibility\HighContrast` `Flags` bit 0, colours from `HKCU\Control Panel\Colors` | Done: `PlatformSettings`, `Theme::for_settings` (high contrast wins over dark; the user's system colours replace the default set). Not yet tested with a real high contrast theme switched on | Re-read on `WM_SETTINGCHANGE`; verify against a real theme |
| Dark/light app mode | winit sets DWM immersive dark mode on the window and sends `ThemeChanged` when the user flips the setting. `TitleBarTheme::{FollowSystem, Light, Dark}` | Done with no `windows-sys` and no `unsafe` (checked: winit 0.30.13 `platform_impl/windows/dark_mode.rs`). Reaches the app as `UiEvent::Theme` | App chrome colours follow `ColorScheme` in phase 2 |
| Alt key tips | Bare Alt press and release; `KeyTips` state machine in `ui-core` | State machine and `Ui` wiring done and tested headless (Alt shows tab tips, a letter descends and selects the tab, Esc goes up, Alt+letter stays a modifier). The winit key translation is not written | Phase 3: map `WindowEvent::KeyboardInput` to `InputEvent::Key`; check that bare Alt does not open the system menu |
| Printing hooks | PDF from `open365-pdf` then the OS print path (Windows spooler or print dialog); Ctrl+P routed through the command layer | Not started | Phase 5 or later; needs COM or WinRT, so the platform crate |
| Accessibility | `accesskit_winit` adapter created before the window is shown; initial tree requested through `UiApp::accessibility_tree`; actions through `UiApp::accessibility_action` | Done. Example publishes a window with a text input. Not yet tested with NVDA or Narrator | Phase 3 builds the real subtree |
| Frame scheduling | `ControlFlow::Wait` plus `WaitUntil(next wake)`; redraw only when requested; vsync `Fifo` | Done | Dirty-rect presentation later if profiling asks for it |

## Dependencies added

All versions are the ones already in the lock file through eframe 0.36.2, except where noted. `cargo deny check` passes with the existing `deny.toml`; no licence was added.

| Crate | Version | Licence | Used by |
|---|---|---|---|
| wgpu | 30.0.1 | MIT OR Apache-2.0 | gfx, ui-host |
| winit | 0.30.13 | Apache-2.0 | ui-host |
| accesskit | 0.24.1 | MIT OR Apache-2.0 | ui-host |
| accesskit_winit | 0.32.2 | Apache-2.0 | ui-host |
| bytemuck (+ bytemuck_derive 1.12.1) | 1.25.2 | Zlib OR Apache-2.0 OR MIT | gfx |
| pollster | 1.0.1 (same as eframe) | Apache-2.0 OR MIT | gfx |
| resvg | 0.48.1 (already used by ui-shell) | Apache-2.0 OR MIT | gfx |
| skrifa | 0.44.0 (already in workspace) | MIT OR Apache-2.0 | gfx |
| thiserror | 2.0.21 (already used by text) | MIT OR Apache-2.0 | gfx, ui-host |

## Phase 2 evidence

New crate `open365-ui-core` (`crates/ui-core`) holds the retained widget tree. `ui-shell`, `ui-egui` and `apps/*` are untouched; `ui-core` reads the `ui-shell` spec JSON through `include_str!` and has no egui dependency.

- **Tree and layout.** Generational node arena; logical-unit bounds; Absolute, Row and Column layout with fixed and flex extents; device rectangles come from rounding edges (`Scale::snap`), so touching neighbours share an edge at every scale. Hit testing is topmost-first with clip and popup extents.
- **Widgets.** Button, toggle, split button, dropdown, combo with editable text, menu with sub-menus, scroll bar, tab strip, tooltip, text field (selection, clipboard, caret blink from the OS rate), checkbox, radio, spinner.
- **Input.** Tab order (tree order, labels and disabled nodes skipped), access keys (Alt+letter), pointer capture, hover and press states, timers on a virtual clock, IME preedit and commit, `Clipboard` trait.
- **Alt key tips.** `KeyTips` state machine as above.
- **Theme.** Light, dark and high contrast colour sets are JSON in `crates/ui-core/themes`. The chrome JSON in `ui-shell/spec` holds geometry only (no colours), so the dark values were copied from `ui-shell/src/theme.rs` (sampled from Office); the light values stay estimates, as there.
- **AccessKit.** Roles, names, toggled and selected state, values, numeric ranges, bounds in device pixels, focus.
- **Headless harness.** `ui_core::harness::Harness`: pointer and keyboard simulation, offscreen render through `gfx::Offscreen`, `Image::diff`, AccessKit queries. It replaces `egui_kittest` for the migration.
- **gfx additions.** Paths (fill, stroke, rounded rect, ellipse) rasterised by the exact-area rasteriser; linear gradients (sRGB interpolation; any number of stops for horizontal and vertical, two stops at other angles); Gaussian drop shadows of rounded boxes; rounded and path clips (coverage mask in the mask atlas, bound per batch with a dynamic uniform offset, nested clips multiply on the CPU); underline and strike-through from font metrics; fallback fonts (`Assets::set_fallbacks`, `resolve_char`, `layout_text`; no kerning).
- **Proof surface.** `ui_core::chrome::build_tab_row` builds the Excel Home tab row and the File button from `sheets.json` and `chrome-sheets.json`. `tests/surface.rs`: rectangles equal the fixture exactly at 125 % (11 of 11, worst edge delta 0 px); pixel diff of the tree's outlines against outlines drawn from the fixture rectangles is 0 of 1 600 000 pixels at 125 %. At 100 %, 150 % and 200 % the reference is derived (`round(u / 1.25 * scale)`), not measured, and also gives 0 differing pixels; that is a consistency check, not a parity claim.
- **Text fit.** `crates/gfx/examples/fit_text.rs` against PrintWindow captures of a private Excel instance (kept in the scratch folder only). Ink-mass fit: light ink gamma 0.65, dark ink (LCD) gamma 2.50, now `RenderParams::office_fitted()`. Contrast stayed at 0. The per-pixel residual is still about 29 levels RMS (glyph advances and hinting differ), so text is **not** within the 1 px bar; Office draws the ribbon tab labels in grayscale and the cell text with ClearType.

Phase 2 gaps: no `winit` to `InputEvent` translation, no clipboard implementation on the platform, `ui-shell` not ported, no menus measured against Office (row heights are placeholders), nothing tested with NVDA, Narrator or a real IME.

## Preview status (2026-10-11)

`apps/preview` (package `open365-preview`, bins `open365-writer-next` and `open365-sheets-next`) renders the full Writer and Sheets chrome on ui-core widgets from the `ui-shell` spec JSON, side by side with the egui apps. `spec.rs`, `chrome.rs`, `layout.rs` and `icon_table.rs` are compiled in from `ui-shell` by path (read only), so layout and responsive rules are shared; they must stay egui-free there.

- **Host.** `ui_host::InputBridge` (winit to `InputEvent`), `UiAdapter`/`UiHandler` (`Ui` behind `UiApp`), `SystemClipboard` (arboard 3, text only, MIT OR Apache-2.0, `cargo deny` passes), `SettingsWatcher` (caret, high contrast and theme re-read on focus gain and on winit theme events; winit has no safe `WM_SETTINGCHANGE` hook). Bare Alt: winit passes `WM_SYSKEYDOWN` of Alt to `DefWindowProc` but swallows `WM_SYSKEYUP` when the window has no native menu, so system menu mode never starts (read from winit 0.30.13 source; tested through the bridge into `KeyTips`, not yet observed on a real window).
- **ui-core additions.** `Look` (icon, content layout, fills, bar, chevron), split buttons with the arrow below, `Widget::Slider`, clipped containers, command-level key tips (`set_tip_nodes`).
- **Not done.** Text residual fit against Office (integer advances, hinting and LCD filter fit, dark-ink capture) and the measured menu and widget metrics with a pixel comparison at 125 % were not started; menu row heights and combo arrow widths are still placeholders. No Office capture was taken in this phase. The document area is a placeholder page or grid.

## Phase 1 evidence and open limits

Tests (run on an NVIDIA RTX 2000 Ada, Vulkan; GPU tests skip with a note when no adapter exists):

- 44 CPU unit tests: snapping maths, shelf packing, tessellation, clip stack, rasteriser coverage (half-pixel edges give 128/255/128; a triangle sums to its area), LCD filter ramps.
- 10 glyph, icon and image tests against the bundled Carlito font, no GPU.
- 10 GPU tests with exact pixel checks: snapped rects, sRGB round trip within 1 level, 50% black over white blends to 187 (linear light) not 127, hairlines exactly one pixel, rounded corners, image texel-per-pixel, icon tint, grayscale and LCD text.
- 11 host tests: scheduler, caret blink phase, IME translation and ordering, theme mapping.

Not yet shown, so not claimed: any comparison with real Office or DirectWrite pixels; the LCD filter, text gamma and contrast values; behaviour on two monitors with different scale; screen reader or IME behaviour with real software.

Known limits to plan for:

- Clips are an integer scissor rectangle plus an optional coverage mask (rounded and path clips, phase 2).
- Paths, linear gradients, shadows and text decorations exist since phase 2.
- Images must fit one atlas (2048 px default) and filter linearly, so scaled images can bleed at edges; 1:1 is exact.
- LCD text needs an opaque background.
- Atlas overflow clears the glyph atlases and asks the host to redraw (`Assets::take_overflow`).
- Fallback fonts exist (`Assets::set_fallbacks`), without kerning; emoji and colour fonts are not handled.
- Full-frame redraw each time; no damage tracking.

## Phase 4 status (2026-10-11)

**Capture.** `tools/office/capture-chrome.ps1` launches its own Excel or Word (aborts if COM returns an existing process, `AutomationSecurity = 3`, Quit in `finally`, only its own PID is stopped), captures the window and the Home tab popups with PrintWindow at 1600 x 1000, 125 %, and writes PNG and raw RGBA to a scratch folder only. Office was in the **dark** theme on the capture day and the theme was not switched, so no light-theme chrome colour was measured: the light values in `themes/light.json` and the preview palette stay estimates. Only light surfaces Office draws in dark mode (white grid, cell text) are visible in the captures.

**Measured (device px at 125 %, dip = px / 1.25).**

| Item | Measured | Now in code |
|---|---|---|
| Menu row | 36 px (28.8 dip) | `MENU_ROW_H` 28.8 (was 28) |
| Menu border plus inset | 1 px border + 1 px inset, rows span x 2..w-2 | `MENU_PAD` 1.6 (was 4) |
| Group separator | 3 px: 1 px gap, a 1 px line, 1 px gap (line full width; unlabelled group line starts 42 px in) | `MENU_SEP_H` 2.4 (was 9), line 1 px |
| Group heading | 37 px first group, 38 px after, bold label | not drawn yet |
| Menu label x | text origin 44 px from the row edge (icon cell first) | `MENU_TEXT_X` 35.2 (was 28) |
| Menu icon | 20 x 20 px (16 dip) at x 14 from the row edge; checked item framed 22 px | not drawn yet |
| Submenu chevron | ink ends 12 px from the right edge | unchanged |
| Combo arrow | 16 px wide "Open" button, chevron ink 8 x 5 px | `DROPDOWN_ARROW_W` 12.8 (was 20) |
| List rows | Font size list 30 px (24 dip), font list 33 px, number format gallery 60 px, "More" row 36 px | dropdown lists still use `MENU_ROW_H` |
| Ribbon small button | 30 x 30 px; icon 20 x 20 px at (5, 5) | unchanged (16 dip icons) |
| Chevron-only button | 19 x 30 px; chevron ink 8 x 5 px at (4, 13) | unchanged |
| Large button icon | ink 36 to 37 px square, top 6 to 7 px | unchanged (32 dip box) |
| Label rule | Fill, Clear, Orientation (50 px wide) show icon and chevron only; labels appear only when icon, label and chevron fit | `shell.rs` hides the label when it does not fit |
| Popup colours (dark) | border `adadad`, inset and body `292929`, hot row `3d3d3d`, group line `666666`, focus frame white 2 px | already in the dark theme |

**Pixel compare.** `open365_preview::compare` plus `examples/compare.rs` score `Preview::render` against a capture per region (RMS in levels over RGB). The preview needs Office's own invisible window frame (inset 9, 1, 9, 9) to line up (`Options::inset`).

| Region | Before (no inset) | After |
|---|---|---|
| title row | 35.80 | 32.72 |
| tabs | 58.80 | 35.85 |
| group Clipboard | 56.82 | 50.25 |
| group Font | 45.36 | 36.36 |
| group Alignment | 46.05 | 43.96 |
| group Number | 44.96 | 38.37 |
| group Styles | 73.83 | 33.18 |
| group Cells | 54.85 | 37.74 |
| group Editing | 59.83 | 56.17 |
| group Add-ins | 62.25 | 57.99 |
| formula bar | 117.70 | 115.74 |
| status bar | 54.50 | 40.15 |
| whole window | 69.65 | 59.87 |

Most of the remaining error is not text: Office draws its own coloured icons (we use Tabler outlines), a pill AutoSave switch, an expanded three-line formula bar and a white grid; the preview shows a dark grid and a one-line formula bar.

**Sheets cell entry (done).** `Widget::Canvas` (role Grid, takes focus, IME allowed, candidate window at the active cell), `Node::intercept` and the `Emit::KeyPressed`, `CanvasText`, `CanvasIme`, `CanvasPointer` events; `apps/preview/src/sheet.rs` wraps the command layer's `Target` (every edit is `Command::xlsx_enter`, undoable, recalculated by calc); `shell/grid.rs` implements Excel's rules (type to replace, F2 and double click edit in place, formula bar and Name Box edit the active cell, Enter, Tab and arrows commit and move, Esc cancels, Delete clears, Ctrl+Z, Ctrl+Y, Ctrl+S). `open365-sheets-next [file.xlsx]` opens a workbook. Tests in `apps/preview/tests/cells.rs` include A1 = 2, B1 = 3, C1 `=A1+B1` showing 5, IME, AccessKit and save and reopen.

**Accessibility actions (done).** `Ui::accessibility_action` maps Focus, Click (the pointer click path), SetValue (commit as Enter) and Increment or Decrement to widgets; `UiAdapter` forwards AccessKit requests and pumps the resulting events.

**Not done, needed in phase 5.** Text fit (integer advances, per-glyph hinting, LCD filter, dark-ink grayscale capture; the 29-level residual is unchanged; the capture script has `-TextProbes` for it); light-theme measurement (needs a light Office session); menu icons, headings and gallery cells; list row heights; expanded formula bar and white grid in the preview; column and row sizing, selection ranges, scrolling by wheel, copy and paste and a file dialog in the grid; the editor draws with the dark field colours instead of Excel's white cell.

## Phase 5 status (2026-10-11)

**Text fit (measured, dark Office theme, own COM instance).** `capture-chrome.ps1 -TextProbes` now puts Segoe UI 11 pt and 9 pt cell text and an 11 pt text box (black on white) on the sheet. `crates/gfx/examples/fit_text.rs` aligns each line (quarter-pixel x, integer baseline) for every hinting, advance and LCD filter choice, then fits gamma. RMS in levels:

| Text | Best setting | RMS |
|---|---|---|
| Text box 11 pt, grayscale, dark ink | natural advances, Normal hinting, gamma 2.0 (integer advances: 45) | 20 |
| Cells 11 pt, ClearType | integer advances at integer ppem (18, not 18.33), Normal, `Soft` filter, gamma 1.4 | 23 |
| Cells 9 pt, ClearType | integer advances, Normal, `Soft`, gamma 1.4 (default filter 21, unrounded 33) | 19 |
| Tab labels 14 dip, light ink | integer advances at ppem 17 (17.5 rounds down), Normal, gamma 0.8 (ppem 17.5: 22) | 14 |

Before: about 29 to 65 levels. Applied: `UI_TEXT` (integer advances) and `gdi_ppem` in ui-core, `LcdFilter` and `TextOptions::office`, `RenderParams::office_fitted` (gamma 1.4 dark, 0.8 light), and the host now sets the fitted params. Gray dark ink in the drawing layer wants natural advances and gamma 2.0 but shares `text_gamma` with LCD. Light-theme values are estimates (Office was not switched to light). Not within 1 px: hinting differs from Office's.

**Compare RMS (Sheets Home, 1600 x 1000, 125 %, fresh capture).** Before to after: title row 32.89 to 31.49, tabs 41.01 to 38.91, Clipboard 50.25 to 50.15, Font 36.36 to 36.27, Alignment 43.96 to 43.83, Number 38.37 to 37.70, Styles 33.18 to 31.09, Cells 37.74 to 34.79, Editing 56.17 to 54.87, Add-ins 57.99 to 57.10, formula bar 113.30 to 19.41 (expanded three-line bar is now the Sheets default), status bar 40.15 to 36.67, whole window 63.62 to 39.45.

**Done also.** Pill AutoSave switch drawn in our own style (`Look::switch`). One canvas event set: `CanvasPointer`, `CanvasDrag`, `CanvasRelease`, `CanvasWheel`, `CanvasAction`, canvas caret blink, `Key::F12`, `Ui::set_canvas_access`; the Writer pane is wired into the window.

**Still open.** Menu icons, group headings, gallery cells and list row heights (30/33/60/36 px) were not started; the "Ori" clipping and Fill chevron overlap; Office's coloured ribbon icons; light-theme measurements; separate gamma for grayscale dark ink.

## Phase 6 status (2026-10-11)

**Real-window smoke test (release exes copied to a scratch folder, driven by a PowerShell SendInput harness, own PIDs only).** `open365-writer-next` with `layout-table-basic.docx` and `footnotes-endnotes.docx`: click, Ctrl+Home, typed text (Unicode SendInput), arrows, wheel, Ctrl+S; the saved DOCX has the typed text and the table intact. `open365-sheets-next` with `dates-1900.xlsx`: clicked C2, typed `42` Enter and `=A1*2` Enter, wheel, Ctrl+S; the saved sheet holds `<v>42</v>` and `<f>A1*2</f><v>118</v>`. Findings: winit's `MainWindowHandle` is a 14 px helper window, so windows are found by EnumWindows per PID; ribbon Undo/Redo enable state is still static (Undo greyed after typing); the formula bar shows the raw serial for a date cell.

**Done.** Ribbon formatting through the command layer (`Format`, `WriterPane::format`: bold, italic, underline, strikethrough, font, size, grow and shrink, alignment; combos by list or Enter; toggle and combo state follows the caret). Writer vertical scroll bar (thumb, track click, drag). Share dialog from the title-bar Share button (`shell/share.rs`): send a copy, send as PDF, inspect, remove and send a clean copy, show in folder; link and people controls disabled with "Connect a server". Sheets Ctrl+D and Ctrl+R, weekday and month name series, date cells fill by day. Tests: `ribbon_format.rs`, `share_dialog.rs`, `grid_fill.rs`.

**Still open.** Table borders and shading, pictures, superscript raise for note references (the layout exposes `cells`, `images` and `rules` on `Page`; the pane does not draw them), ruler, zoom in the pane, Undo/Redo state, menu icons, group headings, gallery cells, list row heights, "Ori" clipping and Fill chevron overlap, frozen panes from the file (`Sheet::frozen_panes` exists; the grid geometry, painting and scrolling do not use it), OS share sheet (the dialog falls back to showing the file in its folder).
