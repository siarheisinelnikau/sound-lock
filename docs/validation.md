# Validation — 2026-10-30

Environment: Windows x64, Rust 1.96.1, Cargo 1.96.1. No virtual audio cable was installed during this work.

## Completed

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --workspace --locked`: **17 passed**, including 11 DSP and 6 native-control / buffering / resampling checks.
- `cargo build --release --locked`: passed; executable is 5,724,160 bytes.
- Native UI Start/Stop smoke check on the existing headset endpoint: processing started, the meter displayed real input (approximately −46 dBFS), and Stop returned to idle without an error.
- The user confirmed that **Hide > tray menu > Open Sound Lock** now reopens the window correctly in the release build.
- English interface and setup separation inspected in the live app. Main layout has no scrolling container. The viewport was subsequently reduced from 420 × 500 to 420 × 446 logical pixels to remove bottom whitespace; that final size has not had another live visual inspection.
- Core Audio read-only probe available through `--probe-system`; each probe initializes COM on its own worker thread.
- Offline synthetic demo, Night profile at 200%: input peak **−0.45 dBFS**, output peak **−3.00 dBFS**; quiet-scene RMS **−38.77 → −20.87 dBFS**. Generated audio was not played automatically.

## Not yet verified

- End-to-end PCM bridge listening on two physical outputs, prolonged movie playback, Bluetooth reconnects, device disconnect recovery and CPU/latency measurements remain manual checks.
- Baseline volume restoration is implemented and native Start/Stop worked; actual attenuation followed by restoration, including external keyboard/mixer changes, still needs a controlled audio test.
- The first CI run used Rust 1.99.1 and found two new float-literal type-fallback warnings in UI stroke widths. Both literals were made explicitly f32; no lint suppression or behavioral change was introduced. The replacement workflow result is checked separately.

System limiter is reactive endpoint attenuation, not a hard sample-peak ceiling. The DSP limiter's sample-peak ceiling applies only to audio routed through the PCM bridge.
