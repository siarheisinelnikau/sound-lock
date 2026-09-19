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

The root native window now contains only Settings (Audio, Driver and Updates), with width 460 and a content-sized height capped at 462 logical pixels. The normal launch keeps it hidden and opens a separate 368 × 304, undecorated, always-on-top quick popup near the tray. A second tray click, Esc or native focus loss dismisses only that popup. Opening Settings hides the popup; closing Settings keeps the process and workers running. The right-click menu offers both surfaces and background actions.

`Application` owns `Rc<RefCell<App>>` on the event-loop thread. Deferred viewport callbacks use a thread-local weak reference, so they share settings/meters without capturing non-Send tray handles or creating ownership cycles. The current host in `src/desktop/` uses egui 0.37 and winit directly, separating background logic from UI passes. A tray callback requests a root pass with `RequestPaintWhileHidden` to create/show a viewport without briefly restoring Settings. Rust 1.95+ is required; Core Audio's Variant feature is declared explicitly.

The Windows renderer is CPU-only: tessellated egui primitives go through the pinned software rasterizer in `vendor/egui-software`, then into a retained top-down BGRA DIB. Settings presents it with GDI BitBlt; the transparent popup uses UpdateLayeredWindow with premultiplied per-pixel alpha. The real winit window handle is used directly. Native undecorated shadows are disabled, so the panel's own border/shadow is the only outer contour. Winit rewrites extended styles when showing a window, so layered alpha and retained pixels are restored after showing as well as before presenting.

Input and worker results wake the event loop. Background polling has a separate deadline and does not force a UI frame. Meter changes invalidate only the popup; dirty settings retain their debounce deadline; busy download/progress states can repaint while active. Browser-routing events wake the root explicitly. Tray menu/tooltip state is cached instead of being reapplied every tick. With no work due, the loop uses Wait; otherwise it uses WaitUntil. Stale cross-viewport output delays are not rearmed. Expose events present the retained bitmap, while resize/DPI changes invalidate it. Logic ticks leave queued input intact for the next UI pass. Close requests reach the UI before deciding whether to hide or quit.

Font/image texture deltas from hidden frames remain queued until a visible rasterization applies them. Screenshots read the CPU bitmap and require no GPU readback. The software rasterizer preserves its upstream licenses; local changes adapt the texture-delta API and hashing imports to egui 0.37, without using its optional window host.

AccessKit remains connected to every native viewport, including initial-tree requests, accessibility actions and platform tree updates. Switching the renderer does not remove the existing accessibility integration.

`popup.rs` combines the tray icon rectangle, monitor work area and effective DPI, clamps the popup inside that work area and converts back to viewport coordinates. Focus is tracked only after a real focus acquisition. Loss of focus while the pointer is over the tray is excluded so a second icon click closes the panel instead of immediately reopening it.

`theme.rs` defines graphite surfaces, restrained blue-gray selection, Segoe UI from the local Windows installation, thin borders and soft shadows. Detail dialogs use `egui::Modal` with a 170-alpha black backdrop, a lighter one-pixel border and a 30-pixel blur shadow. Backdrop clicks are swallowed; Close/Escape dismiss the dialog and restore background input. All authored UI/docs remain English and neither main surface uses a scrolling container. Headless checks exercise real controls under the modal and all populated layouts. Opt-in preview, screenshot and native tray checks do not open audio devices or save user settings.

## Optional driver setup

Driver download and installation are separate actions on a background worker. Driver setup fetches and launches nothing on startup. The HTTPS client uses Windows TLS roots, a global timeout and a download size bound. The archive is pinned to the inspected Pack 45 SHA-256 and is saved atomically only after verification.

Installation rechecks the cached package, extracts all companions into a fresh cache subdirectory and verifies the setup executable with `WinVerifyTrust`. Extraction rejects traversal, Windows device/stream names, symlinks, duplicate files and excessive size/file counts. `ShellExecuteExW` with the `runas` verb then opens the vendor's interactive setup; there are no silent flags. The app neither accepts license terms nor triggers a restart itself. The UI distinguishes installer closure from actual endpoint detection, and installation is disabled while audio processing is active or VB-CABLE is already detected.

The worker waits for the setup process without blocking the UI, then refreshes devices. Successful setup still requires Windows restart. `Use VB-CABLE` explicitly selects the virtual source and Bridge mode; Audio setup selects the browser for automatic routing. No vendor binaries are committed to this repository.

## Browser routing and recovery

`src/routing/` has a testable transaction model, a durable store and a Windows adapter. A separate MTA worker owns the internal `Windows.Media.Internal.AudioPolicyConfig` factory, using the 21H2 IID with a downlevel fallback. ABI details are verified against [EarTrumpet's interface declarations](https://github.com/File-New-Project/EarTrumpet/blob/master/EarTrumpet/Interop/MMDeviceAPI/IAudioPolicyConfigFactoryVariantFor21H2.cs). Only per-process render preferences are called; no global default changes, registry reset, app injection or subprocess helper is involved.

The worker enumerates active render endpoints and non-expired Core Audio sessions, intersects their PIDs with a fixed browser executable allowlist, then verifies full executable path and process creation time. An open process handle is retained during each policy call to prevent PID reuse. Routing only targets registered audio processes: the native Get API returns E_INVALIDARG for an unregistered test process. Discovery runs every 500 ms while Sound Lock is open. New browser audio process births are routed; a single snapshot per executable path preserves the original baseline across child process changes.

Before the first mutation, the three original render-role preferences are atomically saved to an fsynced recovery journal. An exclusive Windows file handle permits one routing owner per user. Each start opens the PCM engine before applying and reading back the route; partial failures restore original roles. Stop/normal exit restore routing before dropping the engine. Timeouts cancel the request and queue restoration; the worker also rolls back if its start reply is abandoned. Restore failures keep the journal and retry. No journal is deleted until restore calls succeed.

Follow-default is represented by a null HSTRING, separate from an explicit device. An old manual Source assignment becomes the physical listening output on Stop. An unavailable previous device falls back to default, or to Listening device when default points at Source. Recovery only matches the same browser and executable path. If the browser has no registered audio process, recovery waits for playback and blocks a new automatic session. On a forced termination recovery needs Sound Lock to launch again; no watchdog remains after exit. Other applications and input roles are untouched. A future Windows API change is reported as an error; manual mixer routing remains available by disabling automatic routing.


## In-app updates

`src/update/` fetches the latest published stable GitHub release on a background worker. Automatic checks occur once per launch when enabled; manual checks are always available. SemVer comparisons reject equal/older versions. Exact repository/tag/asset URLs and names, uniqueness, sizes, digest syntax, bounded responses and stable-release flags are validated before an update is offered. The EXE is streamed to a temporary directory with atomic progress reporting. Its SHA-256 must match the release manifest and any GitHub-provided asset digest, then its PE header must identify Windows x64. HTTPS uses Windows roots. No GitHub token or browser access is needed. The current distribution is unsigned, so these checks provide integrity under the repository/HTTPS trust model, not an independent signing authority.

Download and install records an explicit installation request; polling installs only after its verified download completes. Install later clears that request without discarding the download, and Quit cancels it. Driver setup in progress defers installation. Preview mode cannot install updates. On installation, preparation copies the verified EXE and the running version's helper into a unique directory on the target filesystem. An fsynced, bounded plan records target, hashes, new version and parent PID/creation time. A pipe handshake confirms the helper validated the plan and opened the exact parent process. Before handoff, the app saves settings and restores/stops audio; failure drops the preparation, kills its still-waiting helper and leaves the installed EXE unchanged. No shell command strings or elevated process launches are used.

The helper waits on the parent handle, rechecks hashes and uses [ReplaceFileW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew) with a backup. It starts the new EXE with an internal receipt argument; the new version validates the receipt's path/hash/version and acknowledges its first initialized UI frame. A launch failure or observed exit before acknowledgment restores and relaunches the previous EXE. A live, slow new process is never killed or replaced; the backup remains if acknowledgment times out. Receipt-write/query failures also retain recovery files. Confirmed startup cleans only fixed known filenames and removes the staging directory only when empty. The original filename and external settings survive; audio processing is not auto-resumed. Native tests use real helper and fixture processes, including Unicode/punctuation paths, with no actual app replacement or audio device access.

## Boundaries and next work

Transparent universal PCM replacement on a single endpoint without an installed audio component is not implemented. Process-loopback APIs provide capture; suppression and restoration of original playback must be validated separately before claiming a universal driver-free 400% path. API hooking or injecting into browsers/players is outside this prototype.

Next priorities: actual multi-device movie listening tests, Bluetooth/disconnect handling, long-duration resampling measurements, native restoration regression coverage, signed distribution and optional autostart. A browser-extension implementation belongs in a separate repository.

Release packaging now emits `SoundLock-vVERSION.exe` for the single Windows x64 variant, with no legacy asset alias. The client prefers this name, then an explicit `-x64` name for multi-architecture releases, then historical metadata names. The preexisting 0.1.3 client cannot discover the new name and requires a manual upgrade to 0.1.4.
