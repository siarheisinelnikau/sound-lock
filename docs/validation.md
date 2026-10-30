# Validation — 2026-10-30

Environment: Windows x64, Rust 1.96.1, Cargo 1.96.1. No virtual audio cable was installed during this work.

## Completed

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --workspace --locked`: **35 passed**, including 11 DSP, 7 audio/native-control, 11 browser-routing/ABI/journal, 4 package-handling and 2 modal UI checks. Two environment-dependent tests are ignored by default and were run separately.
- `cargo test --locked routing::windows::tests::real_policy_roundtrip_restores_own_process_preferences -- --ignored`: passed on the actual Windows per-app audio policy. An unstarted, silent audio stream registers only the test executable; all three output-role preferences were read, changed to an explicit endpoint, read back, restored and verified. No browser or system-default route was changed. Before registering the test session, Windows returned E_INVALIDARG; production discovery therefore only routes/restores processes with registered audio sessions.
- Transactional routing tests cover per-role/Follow-default restoration, partial-change rollback, journal-write failure preventing mutation, new browser children retaining the baseline, browser closure/restart recovery, executable-path/PID isolation, old manual-cable migration, disconnected outputs/default-cable fallback, restoration retry, device-ID conversion and exclusive/atomic journal storage. The journal replacement test needs normal Windows filesystem access; sandboxed replacement initially returned Access denied, and the unrestricted test passed.
- `cargo test --locked driver::tests::official_package_prepares_without_installation -- --ignored`: passed. The actual official Pack 45 archive was downloaded through the app's HTTPS client, checksum-checked, fully extracted and its x64 setup signature verified through Windows. **The installer was not launched and no driver was installed.**
- Release 0.1.2 builds successfully with `cargo build --release --locked --target-dir .local/build-0.1.2`. A separate output directory preserves any running earlier executable.
- Native UI Start/Stop smoke check on the existing headset endpoint: processing started, the meter displayed real input (approximately −46 dBFS), and Stop returned to idle without an error.
- The user confirmed that **Hide > tray menu > Open Sound Lock** now reopens the window correctly in the release build.
- Main layout has no scrolling container and remains 420 × 446 logical pixels. Headless egui checks verify both modal pages fit, the backdrop is dimmed, real background profile buttons receive no clicks, backdrop clicks do not close the dialog, and controls work again after Escape closes it.
- Core Audio read-only probe available through `--probe-system`; each probe initializes COM on its own worker thread.
- `--probe-routing` activated the native policy successfully. No browser audio processes were registered at the time of the probe. The device list still had no VB-CABLE endpoint; no driver was installed by these changes.
- Offline synthetic demo, Night profile at 200%: input peak **−0.45 dBFS**, output peak **−3.00 dBFS**; quiet-scene RMS **−38.77 → −20.87 dBFS**. Generated audio was not played automatically.

## Not yet verified

- End-to-end PCM bridge listening on two physical outputs, prolonged movie playback, Bluetooth reconnects, device disconnect recovery and CPU/latency measurements remain manual checks.
- Baseline volume restoration is implemented and native Start/Stop worked; actual attenuation followed by restoration, including external keyboard/mixer changes, still needs a controlled audio test.
- The elevated vendor installer, its UAC cancellation flow, restart and full VB-CABLE audio route were not exercised on this machine. Those are explicit user-operated setup steps, not automated checks.
- End-to-end automatic routing of real Chrome/Edge/Firefox audio, including browser restarts and Quit during playback, remains a manual check. The native own-process API roundtrip and simulated transaction tests do not establish browser-specific live migration behavior. A browser with no audio session must play audio for pending recovery to complete while Sound Lock is open. Force-kill recovery requires relaunching Sound Lock; there is no watchdog after Quit.
- The previous release's Windows CI passed on Rust 1.99.1 after explicitly typing two UI stroke widths as f32. The current routing change is also checked by the Windows workflow when published.

System limiter is reactive endpoint attenuation, not a hard sample-peak ceiling. The DSP limiter's sample-peak ceiling applies only to audio routed through the PCM bridge.
