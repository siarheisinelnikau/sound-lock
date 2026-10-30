# Validation — 2026-10-30

Environment: Windows x64, Rust 1.96.1, Cargo 1.96.1. No virtual audio cable was installed during this work.

## Completed

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --workspace --locked`: **24 passed**, including 11 DSP, 7 native-control / routing / buffering / resampling, 4 package-handling and 2 modal UI checks. One network-dependent test is ignored by default and was run separately.
- `cargo test --locked driver::tests::official_package_prepares_without_installation -- --ignored`: passed. The actual official Pack 45 archive was downloaded through the app's HTTPS client, checksum-checked, fully extracted and its x64 setup signature verified through Windows. **The installer was not launched and no driver was installed.**
- Release 0.1.1 builds successfully with `cargo build --release --locked --target-dir .local/build-0.1.1`. A separate output directory preserves any running earlier executable.
- Native UI Start/Stop smoke check on the existing headset endpoint: processing started, the meter displayed real input (approximately −46 dBFS), and Stop returned to idle without an error.
- The user confirmed that **Hide > tray menu > Open Sound Lock** now reopens the window correctly in the release build.
- Main layout has no scrolling container and remains 420 × 446 logical pixels. Headless egui checks verify both modal pages fit, the backdrop is dimmed, real background profile buttons receive no clicks, backdrop clicks do not close the dialog, and controls work again after Escape closes it.
- Core Audio read-only probe available through `--probe-system`; each probe initializes COM on its own worker thread.
- Offline synthetic demo, Night profile at 200%: input peak **−0.45 dBFS**, output peak **−3.00 dBFS**; quiet-scene RMS **−38.77 → −20.87 dBFS**. Generated audio was not played automatically.

## Not yet verified

- End-to-end PCM bridge listening on two physical outputs, prolonged movie playback, Bluetooth reconnects, device disconnect recovery and CPU/latency measurements remain manual checks.
- Baseline volume restoration is implemented and native Start/Stop worked; actual attenuation followed by restoration, including external keyboard/mixer changes, still needs a controlled audio test.
- The elevated vendor installer, its UAC cancellation flow, restart and full VB-CABLE audio route were not exercised on this machine. Those are explicit user-operated setup steps, not automated checks.
- The first CI run used Rust 1.99.1 and found two new float-literal type-fallback warnings in UI stroke widths. Both literals were made explicitly f32; no lint suppression or behavioral change was introduced. The replacement workflow result is checked separately.

System limiter is reactive endpoint attenuation, not a hard sample-peak ceiling. The DSP limiter's sample-peak ceiling applies only to audio routed through the PCM bridge.
