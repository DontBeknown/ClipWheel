# ClipWheel

A native Windows clipboard inventory wheel, built with Rust, Direct2D, DirectWrite, and DirectComposition. No browser runtime, service, account, or installer.

## Run

Extract **all files** from `ClipWheel-Windows-x64.zip` into one folder and run `clipwheel.exe`. Keep `libunwind.dll` beside it. Requires Windows 11 x64 and a graphics driver supporting Direct3D 11; software rendering is a fallback.

1. Copy text, an image, or files normally.
2. Press **Ctrl+Alt+V** to open the wheel near your cursor.
3. Hover a slice to highlight it. A preview appears after 100 ms only when its content does not fit inside the slice. Scroll over the wheel to browse pages; scroll inside the preview to read more.
4. Click a slice to restore its content and paste into the previously active application. Escape, the shortcut again, or clicking outside cancels.

Right-click the tray icon to open the wheel, pause/resume capture, clear history, change the shortcut, or quit. Shortcut settings accept Ctrl/Alt/Shift plus a letter or digit, and report conflicts. Preferences live in `%LOCALAPPDATA%/ClipWheel/settings.txt`. Startup with Windows is not enabled.

## Appearance

A compact 280 DIP clipboard wheel in opaque Catppuccin Mocha: charcoal surfaces, lavender selection, thin separators, and uniform 14 DIP Segoe UI text. No blur or glass transparency. The area outside the wheel and tooltip stays transparent so the popup floats on the desktop.

Single-line labels end with an ellipsis when needed. Hovering an overflowing item opens a content-sized tooltip, up to 240 × 180 DIP; longer content remains scrollable. Images use 32 × 32 DIP thumbnail bounds. The center shows a clipboard mark and one page dot per history page.

The original ClipWheel logo is included in `assets/` as SVG, PNG and a multi-resolution ICO. It is embedded in the executable and used by the tray icon. `scripts/create-logo.py` regenerates the assets using Python and Pillow. Windows builds require the SDK resource compiler (`rc.exe`, MSVC developer shell) or LLVM-MinGW `windres`; the bundled build script supplies the latter.

## Behavior and limits

- Up to 32 recent unique items, eight per wheel page, with a frozen view while open. New copies continue entering history for the next opening.
- Plain Unicode text, uncompressed 24/32-bit bitmap clipboard data, and file/folder path lists. Defined image alpha is preserved. Text formatting and virtual files are not retained.
- History and thumbnail/preview data share a 128 MiB budget; each entry, including its preview, is limited to 32 MiB. Old entries are evicted automatically. A frozen wheel pins its displayed entries; if those fill the budget, a new copy is rejected with a notice until the wheel closes.
- Large text/file previews use bounded, scrollable sections so the entire content remains accessible without laying out megabytes on the UI thread.
- Images use cached previews up to 512 pixels on the longest edge; original pixels are retained for pasting. File previews show paths, not file contents. Missing files are rejected before changing the clipboard.
- Clipboard payloads stay in memory and disappear on exit. Clear history does not clear the system clipboard. Clipboard contents are not logged or sent anywhere.
- Capturing follows Windows clipboard notifications, including context-menu Copy. Unsupported formats are skipped. Rapid changes faster than the OS/worker can read may coalesce to the latest content.
- Automatic paste requires the previous window to remain available and accept standard Ctrl+V. If focus restoration or input injection fails, a notice asks you to paste manually. Windows may block injection into elevated applications. The app does not elevate itself.
- The invisible area around the wheel belongs to the popup: clicking there dismisses it; that click is consumed.

## Build

The supplied workspace has a project-local Rust LLVM toolchain and LLVM-MinGW under `.tools/`. No global PATH changes were made.

```powershell
./scripts/cargo.ps1 test --locked
./scripts/build.ps1
```

On a clean developer machine, install stable Rust with the MSVC target and Visual Studio C++ Build Tools (Windows SDK included), then run `cargo test` and `cargo build --release` from a Visual Studio Developer PowerShell with the Windows SDK resource compiler on PATH. The MSVC build does not use the LLVM-MinGW helper or its `libunwind.dll` packaging step. Rust 1.88 or newer is required.

## Verify

- `scripts/smoke.ps1`: launches a non-destructive text-only demo, captures the wheel/preview, verifies Escape and quit, and measures idle process CPU/memory. It does not select a slice or alter the clipboard.
- `powershell.exe -STA -File scripts/integration.ps1`: backs up readable clipboard formats in memory, runs the ignored real Windows clipboard worker test, and restores the backup. Checks Unicode text, image alpha, file references, and missing-file rejection; sends no keyboard input.
- `scripts/paste-check.ps1`: optional foreground-window test for automatic paste. Stops if Windows will not activate its test field, rather than typing into another app.
- `clipwheel.exe --demo`: opens sample text history with capture paused. Selecting a demo entry still restores/pastes it; use Escape to dismiss without changing the clipboard.

Verification scripts write reports and captures to `target/verification/`. The memory limit applies to retained clipboard payloads/previews, not total process memory, graphics resources, or transient clipboard conversions.
