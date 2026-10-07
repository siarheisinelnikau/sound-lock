# CPU rendering validation — 2026-10-31

These checks cover the development CPU/GDI host after published v0.1.4. They do not change that release's binaries or establish live audio performance.

## Implementation and scope

- egui 0.37 UI, CPU software rasterization, retained Windows GDI bitmaps. The Windows dependency graph has no eframe/glow/glutin/wgpu rendering host. Loaded-module probes in isolated preview processes found no OpenGL, NVIDIA OpenGL, DXGI or Direct3D modules.
- The popup has per-pixel alpha, a custom rounded border/shadow and no native undecorated shadow. Native checks verify layered alpha and equal client/window dimensions, including after reopening.
- Input and background work have separate scheduling. Inactive UI has no permanent paint timer. Changes to meters invalidate only the popup; unchanged readings do not paint. OS expose events reuse the bitmap. Identical scenes skip CPU rasterization and presentation; pending font/image uploads, size and pixel-density changes invalidate that reuse.
- UI requests are coalesced to the current monitor's refresh interval. Logic deadlines remain separate. The loop uses Wait/WaitUntil, never busy polling.
- Updater startup acknowledgment now follows successful native presentation, rather than just UI construction.

## Completed checks

- Formatter, all-feature Clippy and **55 standard tests** pass. Two new regressions cover idle/debounced-work scheduling and real Settings layouts/font density at 100%, 125%, 150% and 200%.
- Native preview regression passed for normal and quiet launch: popup toggle/reopen, focus loss, separate Settings, minimize/restore, close-to-tray, background menu commands and hidden Quit. Tests use the actual installed callbacks and inspect the actual native windows. They are distinct from real mouse/menu input.
- A stationary Settings diagnostic with only its initial focus event recorded **zero painted frames and zero logic calls after a two-second warm-up**. A separate hidden-idle sample also recorded zero frames and zero post-warm-up logic calls, with measured CPU below the process timer's resolution.
- Some attempted Settings samples received hundreds of pointer moves and button events from the interactive desktop. Those runs are input-active samples and are not reported as idle. Their trace exposed excessive tooltip repaint requests, leading to refresh-rate coalescing and unchanged-scene reuse.
- Final 10-second runs with accessibility enabled used a two-second warm-up and a five-second process-CPU sample on a 20-logical-CPU machine. Visible Settings and hidden idle each painted **0 frames after warm-up**. Settings measured **1.25% of one core / 0.06% of the machine**; hidden idle was below timer resolution. In the changing-meter preview, only the popup painted: **159 frames in eight measured seconds**, with **201 unchanged passes skipped** across the run. Full painted frames had median **1.79 ms**, p95 **4.14 ms** and maximum **5.19 ms**. CPU during the meter sample was **4.68% of one core / 0.23% of the machine**. No relevant GPU modules were loaded. These are local samples, not hardware-independent limits.
- The popup-idle sample lost focus and dismissed itself, so it is not evidence of visible-popup idle cost. The controlled changing-meter sample held its popup visible and had no pointer input; that deliberate QA mode is excluded from normal focus-dismiss verification.
- CPU-rendered framebuffer captures of the popup and Audio page at 100%, 125%, 150% and 200%, plus Updates and the modal backdrop, were inspected. Text and controls fit; the dialog retained its distinct border and dimmed background.
- The actual published 0.1.3 update helper launched the CPU/GDI GUI in an isolated fixture, observed the presentation-based startup receipt, cleaned its helper/backup and allowed the new preview to capture and exit normally. This validates installer protocol compatibility, not old-client discovery of canonical release filenames.

## Reproducing measurements

Build an optimized preview executable, then enable an aggregate local report:

```powershell
cargo build --release --locked --features ui-capture
$env:SOUND_LOCK_RENDER_REPORT = Join-Path (Get-Location) '.local/render-report.json'
$env:SOUND_LOCK_RENDER_SECONDS = '10'
./target/release/sound-lock.exe --preview --settings
```

The timer exits only preview mode. Use `--preview --startup` for hidden idle or `--preview` for the quick panel. Keep pointer input out of an idle sample; the report counts input categories without recording keys, text or coordinates. It includes actual window visibility, painted-frame counts, skipped unchanged scenes, logic counts and full CPU-frame durations, excluding the first two seconds. Frame durations cover UI layout, rasterization and GDI presentation; they are not end-to-end input latency.

For a controlled changing-meter workload, set `SOUND_LOCK_CAPTURE_VIEW=popup` and `SOUND_LOCK_RENDER_ACTIVITY=1` in an `ui-capture` preview build, without setting a capture path. This drives simulated meters and keeps the panel visible despite foreign focus changes. It opens no audio devices and does not represent real audio processing. Remove those variables for normal focus-dismiss checks.

For own-bitmap PNG capture, use `SOUND_LOCK_CAPTURE_TO`, `SOUND_LOCK_CAPTURE_VIEW` and optional `SOUND_LOCK_UI_SCALE` from 1 to 2. UI zoom captures exercise scale-dependent rasterization, not physical movement between monitors with different Windows DPI settings.

## Remaining manual checks

Physical mixed-DPI monitor switching, subjective pointer/keyboard latency, prolonged playback and live VB-CABLE/browser or player audio remain manual. The Windows Computer Use helper was occupied during this work; a real Settings screenshot was inspected before input control became unavailable. Native callbacks, framebuffers and timings establish their measured behavior, not universal smoothness on every desktop.
