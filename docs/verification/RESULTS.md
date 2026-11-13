# Compact Catppuccin redesign — 22 October 2026

- Replaced glass with opaque Mocha surfaces; wheel diameter is now 280 DIP, compared with approximately 420 DIP previously. Overlay text remains Segoe UI 14 DIP. Tooltips size to content, bounded at 240 × 180 DIP.
- 19 standard tests passed; separate GPU visual-matrix test passed. New hit-testing regression was observed failing before the radius change. A section-switching regression was observed failing before stabilizing multi-section tooltip bounds, and now passes.
- Real GPU renders cover 7 states at 100%, 150%, and 200% scaling, including short tooltips, long text, files, images, empty and single-item history. See `compact/`. Light/dark/busy backgrounds are synthetic composites; physical mixed-DPI monitor transitions remain a manual check.
- Clippy passed with warnings denied. Packaged Release build passed live smoke checks for overflow-only hover, preview scrolling, fitting-item dismissal, Escape, tray quit, and 20 reopening cycles. Latest measurements: idle CPU 0%, working set 63.43 MiB, private memory 46.21 MiB, working set after 20 cycles 63.53 MiB, warm opening p95 22.92 ms.
- Original logo assets are in `assets/`: editable SVG, 512 px PNG, size-specific PNGs, and an ICO containing 16/20/24/32/48/64/128/256 px variants. Verified the custom icon by extracting it from the final executable (`compact/executable-icon.png`); the app uses the embedded resource for its window and tray icons.
- Independent review identified tooltip bounds changing across long-content sections; fixed with stable bounds and a regression check. MSVC builds now explicitly require a Visual Studio Developer shell for the SDK resource compiler.
- Output is `dist/ClipWheel-Compact/clipwheel.exe` and `dist/ClipWheel-Compact-Windows-x64.zip`, separate from the previous build. No running user application was terminated. Clipboard writing/pasting into destination applications was not exercised by the non-destructive smoke check.

Reproduce visual captures with `scripts/cargo.ps1 -CargoArgs @('test','export_compact_visual_matrix','--','--ignored')` and `scripts/render-compact-matrix.py`. Build the separate package with `scripts/build.ps1 -OutputName ClipWheel-Compact`.

---

# Liquid glass redesign verification — 22 October 2026

- 18 standard tests passed; the opt-in GPU visual matrix test also passed. The clipboard integration test remains opt-in and was not rerun for this visual change.
- Clippy passed with warnings denied. Release build and Windows ZIP packaging passed.
- Actual DirectWrite checks cover fitting text, Unicode, explicit newlines, wrapping, long tokens, multi-section content, file lists, image bounds, and changed viewport sizes. Geometry checks cover all 1–8 item layouts.
- GPU captures in `glass/` cover empty, single-item, eight-item fitting/overflowing, file, and image states at 100%, 150%, and 200%. These were composited over light, dark, and busy synthetic backgrounds for visual inspection. This does not simulate physically moving between mixed-DPI monitors.
- The packaged executable passed the expanded live smoke check: overflowing item opens preview; pointer enters preview; scrolling advances content; a fitting item hides preview; Escape dismisses; 20 reopen/dismiss cycles; tray quit.
- Live captures: `preview.png`, `preview-scrolled.png`, and `fitting-item.png`. The smoke window is placed away from the physical pointer because concurrent mouse movement can override synthetic hover messages.
- Latest smoke measurements: idle CPU 0.0%, working set 75.25 MiB, private memory 65.42 MiB, working set after 20 cycles 77.20 MiB, warm opening p95 24.10 ms. These use the same measurement limitations described below.
- Static independent review reported no important findings.
- Automatic paste into destination applications and physical mixed-DPI monitor transitions were not revalidated; existing limitations below still apply. The demo smoke check never selects an item or writes clipboard content.

Reproduce the visual matrix with `scripts/cargo.ps1 -CargoArgs @('test','export_glass_visual_matrix','--','--ignored')`, then run `scripts/render-glass-matrix.py` with Python, Pillow, and NumPy. The conversion script removes temporary raw captures and retains PNG/JPEG artifacts.

---

# Verification — 21 October 2026

## Automated checks

- Rust tests: 16 passing behavior/unit tests. Covers ordering, duplicate promotion, count/byte eviction, pinned snapshots, bounded Unicode preview sections, thumbnail accounting, pagination, hit-testing, negative monitor coordinates, shortcut parsing, hostile DIB headers, row orientation, and defined image alpha.
- Separate ignored native integration test: passed through `scripts/integration.ps1`. Real Windows clipboard worker round-tripped Unicode text, transparent DIBV5 images, and file references; missing paths were rejected. The wrapper restored the saved clipboard formats afterward.
- `cargo clippy --all-targets -- -D warnings`: passed.
- Release build: succeeded. The LLVM build includes `libunwind.dll`; omission of that DLL was caught by the first launch check and corrected in packaging.
- Live overlay: transparent drawing, segment hover expansion, detail card, Escape dismissal, and tray quit verified. Captures: `wheel.png` and `preview.png`.
- Repeated opening: 20 open/dismiss cycles completed. Process CPU and memory samples are saved in `smoke.json`.

## Measurement method

Final packaged-build sample:

| Metric | Observed |
| --- | --- |
| Idle CPU over five seconds | 0.0% |
| Working set after dismissal | 61.88 MiB |
| Private memory | 51.43 MiB |
| Warm opening, 95th percentile of 20 cycles | 26.23 ms |
| Median initial-animation callback interval | 15.48 ms |
| Working set after 20 cycles | 64.09 MiB |

Continuous mouse input initially starved low-priority `WM_TIMER` animation ticks. The final build uses a thread-pool timer to post normal-priority frame messages and scopes 1 ms timer resolution to active animations. Both stop at animation completion/dismissal.

`scripts/smoke.ps1` runs the packaged Release build with six sample text entries. It samples idle process CPU over five seconds, records working set/private memory, times reopening until Windows reports the popup visible, and measures renderer callback intervals during the initial animation. These are measurements from this machine under its current desktop workload, not guaranteed limits for all hardware.

The reopen metric measures window visibility, not an optical first-pixel measurement. Renderer intervals are not a compositor presentation trace; a reliable rendered-FPS claim requires ETW/PresentMon profiling. Memory excludes GPU allocations and the text-only sample does not represent a full image history. The 128 MiB limit is retained clipboard/preview data, not whole-process memory.

## Remaining manual acceptance checks

- End-to-end automatic paste into Notepad/an editor, Paint, and File Explorer. The test harness could restore selected text to the clipboard but Windows refused to foreground its separate test form. The guarded test therefore stops before input injection into an unintended application. The isolated clipboard test does not verify destination-app behavior.
- Elevated-target paste fallback and shortcut conflicts with other applications.
- Physical mixed-DPI/multiple-monitor transitions and screen-edge usability. Geometry is unit-tested; a physical display matrix was not exercised.
- Sustained large-image workloads and cold opening on other graphics hardware.

Use `scripts/paste-check.ps1` from a desktop where its test form can take focus for the optional automatic-paste check. Do not treat these manual checks as completed.
