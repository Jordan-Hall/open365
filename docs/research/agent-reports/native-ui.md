# Native UI / rendering toolkit recommendation (Open365)
Date: 2026-10-10. Tags: [F] verified fact from a source below; [I] inference; [U] unverified.

## Recommendation
- **Top: Qt 6 via cxx-qt** for chrome + grid, with the document canvas as a custom Rust-laid-out item inside Qt (QQuickItem/QWidget). [I] It is the only candidate where IME, UIA/NSAccessibility/AT-SPI bridging, native menus, drag/drop, rich clipboard and QPrinter/PDF already exist and are shipped by mature apps. Costs: C++ toolchain, LGPL dynamic-link obligations (legal review needed; not verified here), less "Rust-first".
- **Fallback: Rust-native stack.** winit + wgpu + vello + parley (+AccessKit) for the canvas and grid, Slint for chrome. [I] Cleaner licence/ethos, but IME, text-range accessibility and printing must be built by us.
- **Dioxus:** [F] dioxus-native is 0.8.0-alpha.1 (2026-07-31); blitz-dom 0.3.0-beta.2 (2026-08-24); Blitz README calls itself beta with "many bugs and missing features"; AccessKit is only a stated goal; IME/text-editing/printing not mentioned. Default Dioxus desktop is a WebView (fails the requirement). [I] Not viable for the editor now; revisit for admin/settings surfaces only. Not chosen on preference.

## Key point
[I] No Rust toolkit provides a paged rich-text editor, so the document canvas is custom work in every option. Pick the toolkit by what it contributes around that canvas (platform text input, accessibility, printing).

## Matrix (chrome / canvas / grid)
| Candidate | A11y | IME | Text stack | Canvas fit | 1M-row grid | Print/PDF | License | Health |
|---|---|---|---|---|---|---|---|---|
| Qt+cxx-qt | Native QAccessible [I] | Native [I] | Qt/HarfBuzz [I] | Custom item, good | QAbstractItemModel/TableView virtualised [I] | QPrinter/QPdfWriter [I] | Qt LGPL/GPL/commercial [I,U]; cxx-qt MIT/Apache [U] | cxx-qt 0.10.0 (2026-08-24) [F]; Qt Bridge for Rust 0.3 beta [F]; pre-1.0 |
| Slint | AccessKit; text-edit a11y [U] | [U] | femtovg/skia, swash [F] | Weak (StyledText "first step" to rich text [F]) | Own list virtualisation [U] | none [U] | GPL/royalty-free/commercial [I,U] | 1.18.1 (2026-09-21) [F]; funded |
| egui/eframe | AccessKit Win+macOS only [F] | [U] weak | epaint | Poor: immediate mode, huge scroll areas slow [F] | Needs custom virtualisation | none | MIT/Apache [F] | 0.36.2 (2026-09-08) [F]; breaking releases [F] |
| Iced | [U] none found | [U] | cosmic-text [I] | Possible, custom | custom | none | MIT [F] | 0.14.0 (2025-12-07), experimental [F] |
| Xilem/Masonry | AccessKit-based [F]; depth [U] | [U] (Parley IME planned in 2024 roadmap [F]) | Parley | Best Rust fit [I] | custom | none | Apache-2.0 [F] | 0.4.0 (2025-10-29) [F]; experimental [F] |
| Dioxus native/Blitz | goal only [F] | [U] | Parley/Stylo [I] | HTML layout, not paged | custom | none [U] | MIT/Apache [F] | alpha/beta [F] |
| GPUI | [U] (third-party layers claim semantics [F]) | [U] | platform text [I] | Editor-grade in Zed [I] | uniform_list [I] | none [U] | Apache-2.0 [I,U] | gpui 0.2.2 (2025-10-22), pre-1.0 [F] |
| GTK4 gtk-rs | AT-SPI native [I] | Native [I] | Pango [I] | Custom widget | GtkListView [I] | GtkPrint [I] | LGPL [I] | gtk4 0.11.5 (2026-09-20) [F]; weak macOS/Windows feel [I] |
| Per-OS shells (WinUI/AppKit/GTK) | Best | Best | Platform | Triple UI cost | Native controls | Native | n/a | Highest effort, 3 teams [I] |

Building blocks [F, versions]: AccessKit 0.25.1 (UIA, NSAccessibility, AT-SPI adapters; multi-line text input supported, rich text/hypertext not yet), parley 0.12.0 (2026-10-09), harfrust 0.14.0, cosmic-text 0.19.0, vello 0.11.0, wgpu 30.0.1, winit 0.31.0-beta.3 (still beta).

## Findings
1. [F] AccessKit lacks rich-text/hypertext support, so a Word-style screen-reader experience in a custom canvas is unproven in Rust. Qt's QAccessibleTextInterface is the shortcut. [I]
2. [F] None of the Rust toolkits documents printing; budget for our own PDF/print pipeline (wgpu is not a print path). [I]
3. [F] Most Rust UI crates are pre-1.0 (xilem/iced/gpui/winit/dioxus-native); API churn risk.
4. [I] A grid needs a custom virtualised canvas in every Rust option; 1M rows is just a viewport-window problem if the data model is separate from rendering.

## 2-3 week spike (both Top and Fallback, same shared Rust core)
Scope: paged rich text (3 fonts, bold/italic, table, bidi paragraph), 100k-row x 20-col grid, print-to-PDF, Windows+macOS+Linux.
Pass/fail:
- IME: Japanese/Chinese composition on Win+mac, Linux (ibus/fcitx); inline preedit, candidate window within 20px of caret, commit correct: 100% of 20 scripted cases.
- Screen reader: NVDA, VoiceOver, Orca read caret line/word/char, selection and a table cell; 0 blockers.
- Grid: scroll 100k rows at p95 frame <=16.7ms on stated reference hardware; memory <=300 MB; synthetic 1M-row run does not hang.
- Typing latency p95 <=50ms in a 50-page document.
- Print: PDF export of 20 pages, fonts embedded, text selectable, glyph positions match on-screen layout; native print dialog works.
- Clipboard: round-trip HTML and RTF with Word/Excel paste; a custom OOXML MIME type.
- High-DPI: 100/150/200% and mixed-monitor move without blur.
- Packaging: signed installer per OS under 100 MB; Qt option documents the LGPL relinking mechanism.
Decision: choose Top unless it fails licence review or the Rust-first requirement is binding; choose Fallback if its accessibility and IME pass with <=2 weeks extra estimated effort.

## Unverified
Qt/Slint licence texts, Slint and Iced IME/a11y state, GPUI licence, Blitz a11y/IME, all Qt/GTK capability claims (from general knowledge), performance numbers.

## Sources (accessed 2026-10-10)
- https://crates.io/api/v1/crates/{slint,iced,egui,xilem,masonry,gpui,accesskit,parley,cosmic-text,vello,cxx-qt,gtk4,dioxus-native,blitz-dom,winit,wgpu,harfrust}
- https://github.com/DioxusLabs/blitz
- https://slint.dev/blog/slint-1.16-released
- https://github.com/linebender/xilem
- https://linebender.org/wiki/long-term-roadmap/
- https://github.com/iced-rs/iced
- https://github.com/AccessKit/accesskit
- https://github.com/emilk/egui
- https://www.qt.io/blog/qt-bridge-for-rust-0.3-cxx-qt-compatibility-and-soundness
- https://github.com/KDAB/cxx-qt
- https://docs.rs/gpui-base/0.6.0
