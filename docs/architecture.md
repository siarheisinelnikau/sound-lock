# Standalone architecture

## System limiter

A dedicated COM-initialized worker owns `IAudioMeterInformation` and `IAudioEndpointVolume` for the selected rendering endpoint. Windows documents the meter as preceding endpoint volume attenuation. Every 10 ms the worker computes `min(1, target / peak)` and applies attenuation in dB relative to the user's baseline. Attack is fast; release depends on Night / Headphones / Speakers. No gain above the baseline is allowed.

Driver-quantized volume values are read back after setting them. An external volume change becomes the new baseline. A scoped controller restores that baseline on Stop, normal exit or a handled worker error, unless an external change superseded the last written value. A forced process termination cannot restore anything. Muting is never changed.

This is reactive gain control, not DSP. It cannot anticipate transients or amplify beyond device limits. The app does not install or register an APO, driver, browser extension or virtual endpoint.

## PCM bridge

The player sends its original output to an existing unused playback endpoint. CPAL on Windows builds an input stream on that rendering device with `AUDCLNT_STREAMFLAGS_LOOPBACK`. The device's **output** configuration is used when opening capture. A second, different rendering device receives processed audio. The original source must be inaudible; capture does not suppress it.

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

The main viewport is 420 × 446 logical pixels and contains no scrolling container. Device controls and explanations live in a separate setup dialog. All authored UI text and documentation are English.

Hidden winit windows may not repaint, so queuing an Open action plus requesting repaint can leave the action unprocessed. Tray callbacks now restore the actual root HWND with `ShowWindow(SW_RESTORE)` and request foreground focus **before** waiting for egui to consume the menu event. Both the Open menu and left-click use this path. Other tray actions temporarily wake the event loop; a previously hidden window is hidden again after handling them. The native processing worker continues independently while the UI is hidden.

## Boundaries and next work

Transparent universal PCM replacement on a single endpoint without an installed audio component is not implemented. Process-loopback APIs provide capture; suppression and restoration of original playback must be validated separately before claiming a universal driver-free 400% path. API hooking or injecting into browsers/players is outside this prototype.

Next priorities: actual multi-device movie listening tests, Bluetooth/disconnect handling, long-duration resampling measurements, native restoration regression coverage, signed distribution and optional autostart. A browser-extension implementation belongs in a separate repository.
