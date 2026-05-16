# Sound Lock

**Steady sound for movies.** A standalone Windows tray app written in Rust, with a compact English interface, one main slider and Night / Headphones / Speakers profiles.

No browser extension, virtual audio driver, administrator access or VB-CABLE installation is required. The two processing paths have different capabilities:

| Mode | Setup | What it does | Limit |
|---|---|---|---|
| **System limiter** (default) | Select your listening device | Reactively turns loud scenes down using Windows Core Audio; returns toward your starting volume afterward | Cannot amplify beyond the device maximum or catch the first transient |
| **Boost + leveling** | Route the player to an existing, unused and inaudible second output | Real PCM leveling, compression, up to **400% (4× / +12.04 dB)** gain, and a 5 ms look-ahead sample-peak limiter | Requires two different outputs; does not silence the original source device |

This is an early working prototype. It does **not** promise transparent system-wide 400% boost on a single output device. Ordinary WASAPI loopback captures playback without replacing it. See [Microsoft's loopback documentation](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording).

## Build and run

Requirements: Windows 10/11 x64, a current stable [Rust MSVC toolchain](https://rustup.rs/), and Visual Studio Build Tools with **Desktop development with C++**.

```powershell
cargo run --release --locked
```

Executable: `target\release\sound-lock.exe`. The built executable runs without Rust installed. Processing starts only when you press **Start processing**. Closing the window hides it in the tray. Use **Open Sound Lock** or a left click on the tray icon to restore it; **Quit** exits.

## System limiter

1. Keep your browser/player on its normal output.
2. Open **Setup** and select that same listening device.
3. Select a profile, choose a **Peak target**, and start processing. Lower targets reduce more peaks.

This mode reads the endpoint's pre-volume peak meter and temporarily attenuates its volume. It never raises the volume above your starting level. Night uses the slowest recovery; Speakers recovers faster. Keyboard/mixer volume changes become the new upper bound. Stop and normal Quit restore the last user-selected level if the app still owns the current setting. A crash, forced termination or power loss cannot run cleanup; Windows may retain the last attenuated level.

It is a reactive volume controller, **not a sample-accurate limiter**. The meter reports the preceding device period, and the worker polls every 10 ms. Exclusive-mode streams may bypass software metering or volume control on some hardware. [Microsoft documents the meter placement](https://learn.microsoft.com/en-us/windows/win32/api/endpointvolume/nn-endpointvolume-iaudiometerinformation) and [endpoint volume behavior](https://learn.microsoft.com/en-us/windows/win32/api/endpointvolume/nn-endpointvolume-iaudioendpointvolume).

## Boost + leveling without installing a driver

Use an **already existing, inaudible second playback output**, such as an unused HDMI monitor endpoint without audible speakers:

```text
Browser/player -> unused playback output -> WASAPI loopback
              -> Sound Lock DSP -> physical headphones/speakers
```

1. Start playback so the browser appears in Windows Volume Mixer.
2. Set the browser's output to the unused device in **Settings > System > Sound > Volume mixer**.
3. In Sound Lock, select **Boost + leveling**, open **Setup**, select that unused output as **Source**, and your headphones as **Listening device**.
4. Leave the source unmuted. Its original playback must be inaudible; loopback does not suppress it. Source and listening devices must differ to avoid feedback.
5. Start processing and raise gain gradually from 100%.

If the source is audible you will hear both versions. Muting or zeroing that endpoint can also silence loopback on some devices, so it is not a reliable replacement for an inaudible source. Sound Lock does not change routing or install devices. After stopping, return the browser to its ordinary output. With only one usable output, use **System limiter**.

The DSP is stereo-linked and includes gated upward leveling, a soft-knee compressor, smoothed manual gain and a sample-peak limiter. It raises quiet dialogue along with other quiet material; it does **not** separate speech from music/effects. 400% describes amplitude gain before limiting, not four times perceived loudness or a guarantee that already loud audio can become four times louder without distortion.

| DSP profile | Max automatic lift | Compressor ratio | Sample-peak ceiling |
|---|---:|---:|---:|
| Night | +12 dB | 6:1 | −3 dBFS |
| Headphones | +9 dB | 3.5:1 | −1.5 dBFS |
| Speakers | +6 dB | 2.5:1 | −1 dBFS |

Mono/stereo PCM only; f32, f64, i16, u16 and i32. Different sample rates use linear interpolation with slow clock-drift correction. Matching 48 kHz settings are preferable. Added buffering is about 40 ms plus 5 ms look-ahead and driver latency. No surround/passthrough, true-peak oversampling, speech separation or LUFS normalization. Protected DRM content may block capture.

## Development and validation

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
```

Read-only diagnostics (use the debug build for console output):

```powershell
cargo run -- --list-devices
cargo run -- --probe-system
```

Offline DSP demonstration; opens no audio devices:

```powershell
cargo run -p soundlock-dsp --example render_demo -- .local/demo
```

Creates `before.wav` and `after.wav` from synthetic quiet scenes and bursts. This validates dynamics, not speech recognition or subjective movie quality. Audio is processed locally. The app records nothing and makes no network requests; only the explicit offline example writes generated audio.

Settings: `%APPDATA%\SoundLock\settings.json`. Mode, slider values and stable device identifiers persist. Processing does not start automatically. There is no installer, executable signing or Windows autostart yet.

See [architecture](docs/architecture.md), [manual checks](docs/manual-test.md) and [actual validation results](docs/validation.md). CI builds a Windows executable artifact.

## Source layout

```text
crates/soundlock-dsp/   dependency-free PCM dynamics core
src/system.rs          driver-free endpoint volume controller
src/audio.rs           WASAPI loopback bridge, queue and resampling
src/app.rs             fixed-size, English-only egui interface
src/tray.rs            Windows tray menu and direct HWND restoration
src/config.rs          profiles and persisted settings
```

Inspired by [Sound Lock from 3 APPES](https://www.3appes.com/en/sound-lock/). This is an independent implementation, not affiliated with 3 APPES. Browser-extension work belongs in a separate project.

MIT — [LICENSE](LICENSE).
