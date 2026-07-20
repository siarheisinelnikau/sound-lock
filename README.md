# Sound Lock

**Steady sound for movies.** A standalone Windows tray app written in Rust, with a compact English interface, one main slider and Night / Headphones / Speakers profiles.

System limiter works without a browser extension, extra driver or administrator access. **Optional VB-CABLE setup is available inside the app** for full processing without a spare physical output. The two processing paths have different capabilities:

| Mode | Setup | What it does | Limit |
|---|---|---|---|
| **System limiter** (default) | Select your listening device | Reactively turns loud scenes down using Windows Core Audio; returns toward your starting volume afterward | Cannot amplify beyond the device maximum or catch the first transient |
| **Boost + leveling** | Route the player to optional VB-CABLE or an existing inaudible output | Real PCM leveling, compression, up to **400% (4× / +12.04 dB)** gain, and a 5 ms look-ahead sample-peak limiter | Requires a separate source endpoint; VB-CABLE provides it when only one physical listening device is available |

This is an early working prototype. Without an optional virtual driver or spare output it does **not** provide transparent system-wide 400% boost on a single endpoint. Ordinary WASAPI loopback captures playback without replacing it. See [Microsoft's loopback documentation](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording).

## Build and run

Portable Windows x64 downloads are available on the [Releases page](https://github.com/siarheisinelnikau/sound-lock/releases). Extract the ZIP and run `sound-lock.exe`; it includes README and the license. Release assets also include a standalone EXE and SHA-256 checksums.

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

## Optional VB-CABLE setup

Open **Setup > Driver (optional)**, or click the **DRIVER-FREE** badge to see the optional path. That badge describes System limiter, not a restriction on the whole app.

1. Click **Download driver**. Sound Lock downloads Pack 45 from the [official VB-Audio website](https://vb-audio.com/Cable/) and verifies its checksum. Downloading never launches an installer.
2. Click **Install driver…** separately. Sound Lock unpacks the complete package, verifies the setup executable's Windows signature, and opens the vendor's interactive installer with Windows UAC. Administrator access is required. Complete the vendor's setup yourself; no silent installation or automatic acceptance of license terms is performed.
3. Restart Windows, open the same panel and click **Refresh devices**, then **Use VB-CABLE**. This selects **Boost + leveling** with **CABLE Input** as its loopback source. Keep your real headphones/speakers as **Listening device**.
4. In **Setup > Audio**, leave **Auto-route browser** enabled and select Chrome, Edge, Firefox, Brave, Opera or Vivaldi. Start processing and play something in that browser. Sound Lock sends it to **CABLE Input** and restores its previous output on Stop or Quit. Leave VB-CABLE's Loopback option enabled (the Pack 45 default).

```text
Browser/player -> CABLE Input (virtual, inaudible)
              -> Sound Lock DSP -> your headphones/speakers
```

Sound Lock captures the playback endpoint **CABLE Input**, not the recording endpoint CABLE Output. This is supported by VB-CABLE's loopback mode; see the [vendor's reference manual](https://vb-audio.com/Cable/VBCABLE_ReferenceManual.pdf). A second physical output is not needed with this route.

The installer itself can change Windows' default sound devices. Check those defaults after reboot and keep your physical headphones/speakers as the Windows default. An already detected VB-CABLE disables the install button, so opening setup does not inadvertently offer removal.

The driver is third-party software with [VB-Audio's own license](https://vb-audio.com/Services/licensing.htm); it is downloaded directly and is not bundled or relicensed under this repository's MIT license. Cached packages and extracted installer companions are kept in `%LOCALAPPDATA%\SoundLock\drivers\vb-cable-45`. If the vendor replaces the archive, checksum verification fails rather than launching unexpected content; the official website remains available as a fallback.

## Boost + leveling without installing a driver

Use an **already existing, inaudible second playback output**, such as an unused HDMI monitor endpoint without audible speakers:

```text
Browser/player -> unused playback output -> WASAPI loopback
              -> Sound Lock DSP -> physical headphones/speakers
```

1. Select **Boost + leveling**, open **Setup > Audio**, select the unused output as **Source**, and your headphones as **Listening device**.
2. Leave **Auto-route browser** enabled and select your browser.
3. Leave the source unmuted. Its original playback must be inaudible; loopback does not suppress it. Source and listening devices must differ to avoid feedback.
4. Start processing, play something in the selected browser and raise gain gradually from 100%.

If the source is audible you will hear both versions. Muting or zeroing that endpoint can also silence loopback on some devices, so it is not a reliable replacement for an inaudible source. With only one usable output, use **System limiter** or the optional VB-CABLE route above.

## Automatic browser output

**Auto-route browser** is enabled by default for Boost + leveling. Choose one browser in **Setup > Audio**; Start opens the PCM bridge first, then switches that browser's registered audio processes to Source. All tabs using its normal audio output are affected. New audio processes are detected while processing runs, including after a browser restart. Stop and normal Quit restore the original per-app Console, Multimedia and Communications output preferences, including **Follow default**. System limiter never routes apps or changes the Windows default output.

Previous preferences are saved before switching in `%APPDATA%\SoundLock\browser-routing.json`. A partial start failure rolls back changes. Recovery retries while Sound Lock is open and on its next launch. If the browser has closed, open it and start playback so Windows can resolve its audio identity; pending recovery blocks a new automatic session until it completes. A force kill, crash or power loss cannot restore immediately: relaunch Sound Lock and play browser audio to recover. There is no background service after Quit.

When migrating an existing manual assignment to the selected Source (for example CABLE Input), Stop uses **Listening device** so the browser remains audible. If the previous explicit device was disconnected, restoration uses the Windows default; if that default is Source, it uses Listening device instead.

This uses Windows' internal per-app audio policy interface, so compatibility with future Windows updates is not guaranteed. If routing is unavailable, Start reports the error and does not leave processing active. For another player, a site that explicitly selects its own sink, or manual control, disable **Auto-route browser**, use **Open Windows volume mixer** and restore that app's output yourself afterward. No browser extension, injection, driver installation or administrator access is needed for routing. Process paths/PIDs and audio-session metadata are used; tabs, URLs and audio content are not inspected.

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
cargo run -- --probe-routing
```

Offline DSP demonstration; opens no audio devices:

```powershell
cargo run -p soundlock-dsp --example render_demo -- .local/demo
```

Creates `before.wav` and `after.wav` from synthetic quiet scenes and bursts. This validates dynamics, not speech recognition or subjective movie quality. Audio is processed locally and is never uploaded or recorded. Optional driver setup downloads the official package over HTTPS and uses Windows signature verification; only the explicit offline example writes generated audio.

The normal test suite covers modal behavior, audio processing, transactional routing and recovery, and package handling. An opt-in native policy check switches and restores **only the test executable's own** preferences using an unstarted, silent audio session:

```powershell
cargo test --locked routing::windows::tests::real_policy_roundtrip_restores_own_process_preferences -- --ignored
```

An additional network-dependent check prepares the real driver package **without running its installer**:

```powershell
cargo test --locked driver::tests::official_package_prepares_without_installation -- --ignored
```

Settings: `%APPDATA%\SoundLock\settings.json`. Mode, slider values and stable device identifiers persist. Processing does not start automatically. Audio and Driver setup use a modal with a dimmed, inactive background; close it with **Close** or Esc. There is no Sound Lock application installer, executable signing or Windows autostart yet.

See [architecture](docs/architecture.md), [manual checks](docs/manual-test.md) and [actual validation results](docs/validation.md). CI builds a Windows executable artifact.

To publish a version, update Cargo's version/lockfile and `docs/releases/VERSION.md`, then push a commit whose first line is exactly `Release vVERSION` to `main`. Alternatively, run the Windows workflow on `main` with **publish_release** enabled. Publishing waits for formatting, Clippy, tests and the Windows release build; a separate job packages that exact artifact and publishes a draft only after its assets upload. Existing published releases are never overwritten.

## Source layout

```text
crates/soundlock-dsp/   dependency-free PCM dynamics core
src/system.rs          driver-free endpoint volume controller
src/audio.rs           WASAPI loopback bridge, queue and resampling
src/app.rs             fixed-size, English-only egui interface
src/tray.rs            Windows tray menu and direct HWND restoration
src/config.rs          profiles and persisted settings
src/driver.rs          optional official driver download and interactive setup
src/routing/           per-browser audio policy, durable recovery and ownership lock
```

Inspired by [Sound Lock from 3 APPES](https://www.3appes.com/en/sound-lock/). This is an independent implementation, not affiliated with 3 APPES. Browser-extension work belongs in a separate project.

MIT — [LICENSE](LICENSE).
