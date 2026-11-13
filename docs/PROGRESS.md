# Native Rust clipboard wheel implementation

Authority: user-approved Native Rust Clipboard Wheel plan in this chat.

- Foundation: local Rust + portable LLVM-MinGW, no global PATH changes.
- Ruling: this is an empty non-Git directory; implement in place, no worktree needed.
- Ruling: use windows 0.58 bindings and Direct2D/DirectWrite/DirectComposition; retain Cargo.lock.
- Pre-flight: clipboard payload ownership flows into bounded history; overlay takes Arc snapshots; paste consumes the selected payload. A snapshot must be released when dismissed.
- Task 1: complete — history/geometry tests observed failing, then passing.
- Task 2: complete — native transparent DirectComposition renderer, wheel geometry, preview sections and thumbnail caches. Live screenshots verified.
- Task 3: complete — clipboard worker, deduplication fingerprints, bounded memory including frozen-view references, alpha-preserving image round trips, file validation.
- Task 4: complete — hotkey/tray/settings, focus restoration and guarded paste, event-driven animations, no idle animation loop.
- Review: fresh reviewer identified unbounded text layouts, tray target loss, copies skipped during open wheel, and lost image alpha. All four addressed with bounded sections, preserved target, shared pinned-memory accounting, and DIBV5 alpha support.
- Verification: 16 regular tests passing; separate native clipboard test passing; strict Clippy passing. See docs/verification/RESULTS.md for measured results and manual checks not completed.
- Ruling: retain GPU resources while hidden for quick reopening; no rendering/timers while idle. Temporarily request 1 ms timer resolution only for active animations.
- Ruling: very large previews use 4 KiB UTF-8 sections, with scrolling across sections, to expose all content without unbounded UI-thread layout.
- Delivery: portable executable, LLVM runtime DLL and third-party licenses in dist/ClipWheel, plus a ZIP archive.
- Final animation fix: use a thread-pool timer posting normal-priority frame messages; WM_TIMER was starved by continuous mouse input. Final median frame callback interval 15.48 ms, warm-open P95 26.23 ms, idle CPU 0.0% over five seconds, working set 61.88 MiB.
