# Standalone architecture

## System limiter

A dedicated COM-initialized worker owns `IAudioMeterInformation` and `IAudioEndpointVolume` for the selected rendering endpoint. Windows documents the meter as preceding endpoint volume attenuation. Every 10 ms the worker computes `min(1, target / peak)` and applies attenuation in dB relative to the user's baseline. Attack is fast; release depends on Night / Headphones / Speakers. No gain above the baseline is allowed.

Driver-quantized volume values are read back after setting them. An external volume change becomes the new baseline. A scoped controller restores that baseline on Stop, normal exit or a handled worker error, unless an external change superseded the last written value. A forced process termination cannot restore anything. Muting is never changed.

This is reactive gain control, not DSP. It cannot anticipate transients or amplify beyond device limits. This path requires no APO, driver, browser extension or virtual endpoint. Optional driver setup is separate and starts only from explicit user actions.

## PCM bridge

The player sends its original output to an inaudible playback endpoint: optional VB-CABLE's CABLE Input or an existing unused physical output. CPAL on Windows builds an input stream on that rendering device with `AUDCLNT_STREAMFLAGS_LOOPBACK`. The device's **output** configuration is used when opening capture. A different rendering endpoint receives processed audio. With VB-CABLE only the listening endpoint needs physical hardware. The original source must be inaudible; capture does not suppress it.

The capture callback converts mono/stereo samples to `[f32; 2]` and pushes frames into a bounded wait-free SPSC queue. The output callback interpolates frames, processes dynamics and converts to the listening device's format. No file I/O, logging, mutex or allocation occurs in the normal audio callbacks. Atomics carry settings, meter values and stream errors to/from the UI.

The queue holds 250 ms with a 40 ms target. A slow occupancy controller adjusts the input/output rate ratio by up to ±0.5% for independent device clocks. Empty input yields silence and reprimes; excessive backlog is trimmed. A short fade-in softens recovery. Linear resampling is a prototype compromise; a band-limited implementation and long-duration drift tests remain future work.

## DSP

The dependency-free Rust core uses a linked stereo envelope based on the louder channel, smoothed over 25 ms. Upward leveling fades out below −42 dBFS and is disabled below −55 dBFS. A downward compressor uses a 6 dB soft knee. Manual gain is 1–4×. Combined gain changes use an 8 ms downward and 300 ms upward time constant.

| Profile | Target RMS | Threshold | Ratio | Max lift | Ceiling |
|---|---:|---:|---:|---:|---:|
| Night | −24 dBFS | −26 dBFS | 6:1 | +12 dB | −3 dBFS |
| Headphones | −22 dBFS | −20 dBFS | 3.5:1 | +9 dB | −1.5 dBFS |
| Speakers | −20 dBFS | −16 dBFS | 2.5:1 | +6 dB | −1 dBFS |

A 5 ms delay and preallocated monotonic maximum queue drive the final stereo-linked limiter, with instant gain reduction and 90 ms release. A final clamp preserves the sample-peak ceiling during profile changes. There is no oversampled true-peak detection. Disabling leveling in bridge mode returns gain to unity while retaining a −1 dBFS protective limiter.

## UI and tray lifecycle

The main viewport is 420 × 446 logical pixels and contains no scrolling container. Audio and optional Driver settings live in an `egui::Modal`. Its full-viewport black backdrop has alpha 170 and intercepts pointer/keyboard input to underlying controls. Backdrop clicks neither activate controls nor dismiss the modal. Close and Escape dismiss it; an open combo box consumes Escape first. Headless tests exercise real profile buttons while the modal is open and verify both pages fit the viewport. All authored UI text and documentation are English.

## Optional driver setup

Download and installation are separate actions on a background worker. Nothing is fetched or launched on startup. The HTTPS client uses Windows TLS roots, a global timeout and a download size bound. The archive is pinned to the inspected Pack 45 SHA-256 and is saved atomically only after verification.

Installation rechecks the cached package, extracts all companions into a fresh cache subdirectory and verifies the setup executable with `WinVerifyTrust`. Extraction rejects traversal, Windows device/stream names, symlinks, duplicate files and excessive size/file counts. `ShellExecuteExW` with the `runas` verb then opens the vendor's interactive setup; there are no silent flags. The app neither accepts license terms nor triggers a restart itself. The UI distinguishes installer closure from actual endpoint detection, and installation is disabled while audio processing is active or VB-CABLE is already detected.

The worker waits for the setup process without blocking the UI, then refreshes devices. Successful setup still requires Windows restart. `Use VB-CABLE` explicitly selects the virtual source and Bridge mode; Audio setup selects the browser for automatic routing. No vendor binaries are committed to this repository.

## Browser routing and recovery

`src/routing/` has a testable transaction model, a durable store and a Windows adapter. A separate MTA worker owns the internal `Windows.Media.Internal.AudioPolicyConfig` factory, using the 21H2 IID with a downlevel fallback. ABI details are verified against [EarTrumpet's interface declarations](https://github.com/File-New-Project/EarTrumpet/blob/master/EarTrumpet/Interop/MMDeviceAPI/IAudioPolicyConfigFactoryVariantFor21H2.cs). Only per-process render preferences are called; no global default changes, registry reset, app injection or subprocess helper is involved.

The worker enumerates active render endpoints and non-expired Core Audio sessions, intersects their PIDs with a fixed browser executable allowlist, then verifies full executable path and process creation time. An open process handle is retained during each policy call to prevent PID reuse. Routing only targets registered audio processes: the native Get API returns E_INVALIDARG for an unregistered test process. Discovery runs every 500 ms while Sound Lock is open. New browser audio process births are routed; a single snapshot per executable path preserves the original baseline across child process changes.

Before the first mutation, the three original render-role preferences are atomically saved to an fsynced recovery journal. An exclusive Windows file handle permits one routing owner per user. Each start opens the PCM engine before applying and reading back the route; partial failures restore original roles. Stop/normal exit restore routing before dropping the engine. Timeouts cancel the request and queue restoration; the worker also rolls back if its start reply is abandoned. Restore failures keep the journal and retry. No journal is deleted until restore calls succeed.

Follow-default is represented by a null HSTRING, separate from an explicit device. An old manual Source assignment becomes the physical listening output on Stop. An unavailable previous device falls back to default, or to Listening device when default points at Source. Recovery only matches the same browser and executable path. If the browser has no registered audio process, recovery waits for playback and blocks a new automatic session. On a forced termination recovery needs Sound Lock to launch again; no watchdog remains after exit. Other applications and input roles are untouched. A future Windows API change is reported as an error; manual mixer routing remains available by disabling automatic routing.

Hidden winit windows may not repaint, so queuing an Open action plus requesting repaint can leave the action unprocessed. Tray callbacks now restore the actual root HWND with `ShowWindow(SW_RESTORE)` and request foreground focus **before** waiting for egui to consume the menu event. Both the Open menu and left-click use this path. Other tray actions temporarily wake the event loop; a previously hidden window is hidden again after handling them. The native processing worker continues independently while the UI is hidden.

## Boundaries and next work

Transparent universal PCM replacement on a single endpoint without an installed audio component is not implemented. Process-loopback APIs provide capture; suppression and restoration of original playback must be validated separately before claiming a universal driver-free 400% path. API hooking or injecting into browsers/players is outside this prototype.

Next priorities: actual multi-device movie listening tests, Bluetooth/disconnect handling, long-duration resampling measurements, native restoration regression coverage, signed distribution and optional autostart. A browser-extension implementation belongs in a separate repository.
