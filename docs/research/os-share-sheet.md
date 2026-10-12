# OS share sheet from a native Rust app (Share > "Send a copy")

Researched 2026-10-11. Source dates are the page's own "updated"/"ms.date" metadata where the page gave one.
Crate data comes from the crates.io API, fetched 2026-10-11. Maintenance is judged only from the `updated_at` field,
so treat it as a prompt to check each repository, not as a finding.
Apple pages are JavaScript-rendered; the fetch returned only the page title, so Apple claims below are marked
"unverified" and need a check against developer.apple.com before the ADR is accepted.

## Summary

| Platform | API | Crate | unsafe in our code? | Needs package identity? | Recommendation |
|---|---|---|---|---|---|
| Windows | DataTransferManager + IDataTransferManagerInterop | `windows` | Likely yes (see below) | No (docs say packaged or unpackaged) | Spike, isolate in one crate |
| macOS | NSSharingServicePicker | `objc2-app-kit` | Yes for the call sites (unverified) | No | Use objc2, isolate |
| Linux | No share portal; Email + OpenURI portals | `ashpd` | No | No | Email portal + OpenFile "ask" |
| iOS | UIActivityViewController | `objc2-ui-kit` | Yes (unverified) | No | Phase 2, objc2 |
| Android | ACTION_SEND + Intent.createChooser + FileProvider | `jni` | Mostly no via `jni` safe API, plus Java glue | No | Phase 2, small Java shim |

## Constraint: unsafe_code is forbidden workspace-wide

None of the OS-level calls below can be made through an entirely safe, share-specific, vetted crate.
I found none; the crates.io listing was not searched. The practical options:

1. Put each platform binding in its own small crate, `open365-share-<os>`, that is not under the
   workspace `forbid(unsafe_code)` lint. Give it a narrow, safe public API such as
   `share_file(window: &RawWindowHandle, path: &Path, title: &str) -> Result<()>`. Add review and tests there.
   This is a deviation from the workspace rule and needs owner sign-off.
2. Or call a helper process or script. Not recommended: it breaks window parenting and the sharing UI anchor.

Note that `forbid` cannot be overridden by a nested `allow`. That crate must be excluded from the workspace
lint table, or the workspace must use `deny` instead.

## Windows

- API: `DataTransferManager` (WinRT). A desktop app must not use `GetForCurrentView` / `ShowShareUI`; those are UWP-only.
  It must use `IDataTransferManagerInterop::GetForWindow(hwnd, riid)` to get a per-window manager, subscribe to
  `DataRequested`, then call `IDataTransferManagerInterop::ShowShareUIForWindow(hwnd)`.
  Source: Microsoft Learn, "Share content from your app" (ms.date 2026-09-27, updated 2026-09-28):
  https://learn.microsoft.com/en-us/windows/apps/develop/windows-integration/integrate-sharesheet-send
- Package identity: that page says "Desktop apps - whether packaged or unpackaged - use the `IDataTransferManagerInterop`
  interface" and lists WinUI 3, WPF and WinForms. So the interop is needed, and identity is not required, according to the docs.
  The older UWP page (updated 2025-10-15) also says "Desktop apps should use the IDataTransferManagerInterop":
  https://learn.microsoft.com/en-us/windows/uwp/app-to-app/share-data
  The interface reference (updated 2024-02-22) lists a minimum client of "Windows 8 [UWP apps only]". That conflicts with the
  newer page, so run a spike on an unpackaged winit window before relying on it:
  https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-idatatransfermanagerinterop
  The Microsoft sample is https://github.com/microsoft/Windows-classic-samples/tree/main/Samples/ShareSource (not read).
- Sharing a file: `DataPackage.SetStorageItems` with an `IStorageItem` from `StorageFile.GetFileFromPathAsync(path)`;
  `Properties.Title` is mandatory. Use `SetWebLink` for URLs. Same page as above.
  Because file data is requested on demand in `DataRequested`, the file must still exist (a temp copy) until the share completes.
  `ShareCompleted` / `ShareCanceled` events exist on `DataPackage` for cleanup.
- Crate: `windows` (microsoft/windows-rs), 0.62.2, licence MIT OR Apache-2.0, `updated_at` 2025-10-06
  (https://crates.io/api/v1/crates/windows). It exposes `Windows.ApplicationModel.DataTransfer.*` and
  `Win32::UI::Shell::IDataTransferManagerInterop` behind cargo features. Version and date are as fetched, and a newer release may exist.
- unsafe: the interop calls take an `HWND` and an IID and are expected to be `unsafe` in windows-rs; I did not verify that.
  The WinRT `DataTransferManager` side (event add, `DataPackage`, `StorageFile`) is safe Rust.
  The `HWND` comes from winit's `raw-window-handle`, whose `Win32WindowHandle` accessor is safe.
  The interop call may need to run on the UI (STA) thread, which winit's event loop thread is; verify COM/WinRT initialisation in the spike.
- Packaging: none required per the docs. MSIX would add identity but is not needed for Share.
- Recommendation: implement via `windows` in an isolated crate. Spike first (unpackaged exe, file share, Mail/Teams/Nearby targets).
  Fallback if the spike fails: sparse package for identity, or a "Reveal in folder" action.

## macOS

- API: `NSSharingServicePicker(items:)` then `show(relativeTo:of:preferredEdge:)` anchored to a view/rect. Items can be `NSURL` file URLs.
  Available since macOS 10.8 (unverified; Apple page not rendered by the fetcher):
  https://developer.apple.com/documentation/appkit/nssharingservicepicker
  Alternative: `NSSharingService` for a fixed service (for example Mail). Not read.
- winit gives an `NSView` pointer through `raw-window-handle` (`AppKitWindowHandle.ns_view`), which is the anchor view.
- Crate: `objc2-app-kit` 0.3.2, licence Zlib OR Apache-2.0 OR MIT, `updated_at` 2025-10-04, repository madsmtm/objc2
  (https://crates.io/api/v1/crates/objc2-app-kit). Companions `objc2`, `objc2-foundation` (NSURL, NSArray).
- unsafe: raw-pointer-to-view conversion is unsafe. objc2 offers safe wrappers for many AppKit methods (it requires a
  `MainThreadMarker`, which is a safe proof), but the view cast is unsafe. Expect a few unsafe lines; isolate them.
- Packaging: none for the picker. A sandboxed, Mac App Store build needs the user-selected file entitlement for files outside the sandbox. Unverified.
- Recommendation: use objc2 in the isolated crate; present from the Share button's rect.

## Linux

- There is no Share portal. The xdg-desktop-portal API reference lists 33 interfaces (Account, Background, Camera, Clipboard,
  Documents, Dynamic Launcher, Email, File Chooser, File Transfer, ..., OpenURI, Print, ..., Trash, Wallpaper); none is a share sheet
  (https://flatpak.github.io/xdg-desktop-portal/docs/, fetched 2026-10-11).
- Fallbacks, both real portal methods:
  - `org.freedesktop.portal.Email.ComposeEmail(parent_window, options)`: options `subject`, `body`, `addresses`, `attachment_fds`
    (file descriptors, so the file attaches). Interface version 4. The host default mail client must support `mailto:`
    (https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Email.html).
  - `org.freedesktop.portal.OpenURI.OpenFile(parent_window, fd, options)` with `ask: true` (OpenURI v3) shows an
    "open with" chooser, so the user picks an installed app. `OpenDirectory` reveals the file in the file manager (v3).
    OpenURI does not accept `file://` URIs; use `OpenFile`
    (https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.OpenURI.html).
  - `File Transfer` (`org.freedesktop.portal.FileTransfer`) is for app-to-app drag and drop; not a share sheet. Not read in detail.
- Crate: `ashpd` 0.13.13, MIT, `updated_at` 2026-07-17, ~17.3M downloads, repository bilelmoussaoui/ashpd
  (https://crates.io/api/v1/crates/ashpd). Wraps `Email`, `OpenURI`, `FileChooser` and others, including window identifiers.
- unsafe: none in our code. `ashpd` is async (zbus), so run it on a background executor thread. Window identifier: X11 uses
  `x11:<xid>`; Wayland needs the xdg-foreign "exported" handle. ashpd's `WindowIdentifier` covers both when given raw-window-handle.
- Packaging: no manifest permission for the portals. A Flatpak sandbox has the portals natively; deb/AppImage works if a
  portal backend is running. Without a portal, ashpd returns an error, which we should surface in the UI.
- Recommendation: offer "Send a copy" as Email portal (attach) plus "Open with..." (OpenFile with ask). Say plainly in the UI that Linux has no unified share sheet.

## iOS (phase 2)

- API: `UIActivityViewController(activityItems:applicationActivities:)` with a file `URL`, presented from a view controller.
  On iPad it needs `popoverPresentationController.sourceView/sourceRect` or `barButtonItem`, else UIKit raises an exception.
  Available since iOS 6.0 (unverified; Apple page not rendered):
  https://developer.apple.com/documentation/uikit/uiactivityviewcontroller
- Crate: `objc2-ui-kit` 0.3.2, Zlib OR Apache-2.0 OR MIT, `updated_at` 2025-10-04 (https://crates.io/api/v1/crates/objc2-ui-kit).
  winit gives a `UIView` (`UiKitWindowHandle.ui_view` and `ui_view_controller`).
- unsafe: same as macOS (pointer cast). Packaging: none; shared file must be in the app container (temp copy).
- Recommendation: same pattern as macOS, in the isolated crate.

## Android (phase 2)

- API: `Intent(ACTION_SEND)` with `EXTRA_STREAM` set to a `content://` URI, a specific MIME type, wrapped in
  `Intent.createChooser(intent, null)`, then `startActivity`. A `file://` URI does not work. Use `FileProvider` (preferred;
  per-URI temporary grants) and set `FLAG_GRANT_READ_URI_PERMISSION`. Use `ACTION_SEND_MULTIPLE` for several files. Use the most specific MIME type.
  Source: https://developer.android.com/training/sharing/send (last updated 2026-10-01).
- Crate: `jni` 0.22.4, MIT OR Apache-2.0, `updated_at` 2026-03-16, repository jni-rs/jni-rs (https://crates.io/api/v1/crates/jni).
  The Activity/JavaVM pointers come from `android-activity` (winit's Android backend); not researched here.
- unsafe: the `jni` API is mostly safe, but obtaining the `JavaVM` and Activity from raw pointers is unsafe. A small Java/Kotlin helper
  class that does the FileProvider and chooser work reduces JNI calls to one safe-looking call. Unverified.
- Packaging: the manifest needs a `<provider>` for `androidx.core.content.FileProvider` plus an XML file-path config, and the AndroidX Core dependency.
- Recommendation: Java helper class plus a single JNI call, in the isolated crate; test on a physical device.

## Open questions

- Does the Windows interop work for an unpackaged winit window, including file sharing? (Spike needed.)
- Does windows-rs mark the interop methods `unsafe`? (Check at spike time.)
- Whether the workspace can exclude one crate from the lint, or whether `forbid` must become `deny`.
- Apple claims need a check against the rendered Apple pages or the objc2 docs.
