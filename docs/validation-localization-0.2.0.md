# Localization and appearance verification for 0.2.0

## Automated checks

- `cargo fmt --all -- --check`.
- `cargo test --offline --locked --workspace`: 51 passed, 2 intentionally ignored tests requiring network or Windows audio-policy mutation. No real audio endpoint, driver setup or user settings were changed.
- `cargo clippy --offline --locked --workspace --all-targets --all-features -- -D warnings`: passed.
- Focused translated-layout checks cover every language in both themes, all Settings pages, available-update/error states, and both popup processing paths. They caught and fixed a Russian update-row overflow. Long diagnostics now use the actual remaining row width.
- The catalog has 110 messages with six complete columns. Tests reject duplicate keys, empty translations and changed placeholders. Unknown user/device text stays unchanged.
- Settings migration tests preserve audio preferences, device identifiers and update-check opt-out when adding language/theme defaults and round-trip non-default choices.

## Actual application screenshots

The optimized `ui-capture` build rendered through the production CPU/GDI host with fake preview endpoints and isolated AppData paths. Each invocation saved its own framebuffer, completed with exit code 0 and exited under an external 15-second timeout. No desktop capture or injected OS input was used. The capture path has an internal eight-second readiness timeout and rejects unknown view names.

Reviewed at native 100% scale: English dark popup, Audio and Updates; Russian light Audio; German light Updates; French light attention dialog. Labels, spacing, controls, contrast and dialogs fit without clipping. Current popup and Settings screenshots, plus Russian/French light examples, are embedded in README. Captures show the client area; they do not prove OS tray-menu appearance or keyboard navigation.

## Bounded rendering/load gate

The host uses `egui_software_backend::EguiSoftwareRender` for CPU rasterization and Windows GDI bitmap presentation. The winit event loop sleeps with `Wait`/`WaitUntil`; language/theme application runs only on startup or a preference change. Hidden logic and visible meter updates retain their existing event-driven scheduling. Existing instances were checked before launching disposable preview processes. A shared named mutex serialized native preview launches against other project work.

| Preview state | PID | Sample after startup | One-core CPU | Machine CPU (20 logical processors) | Root/popup paints after 2 s warmup | Logic after warmup |
| --- | --- | --- | --- | --- | --- | --- |
| Visible Russian light Settings | 38784 | 5.006 s after 3 s | 0.0% | 0.0% | 0 / 0 | 0 |
| Hidden English dark Settings | 37144 | 5.000 s after 3 s | 0.0% | 0.0% | 0 / 0 | 0 |

Both processes exited normally after nine seconds. No OpenGL/DirectX graphics modules were observed in these preview processes; source inspection establishes the actual CPU/GDI path. GPU telemetry was unavailable. CPU timer resolution limits the reported zero values; this is an idle sample, not a broad performance benchmark.

## Remaining manual scope

Live language/theme interaction through native OS input, native tray-menu appearance, keyboard/screen-reader traversal, physical mixed-DPI moves, hide/restore under input and real audio workload were not exercised in this change. Existing unit tests cover tray action dispatch and input behavior through egui fixtures; static PNGs do not replace that manual scope. Real driver installation and an in-place update during playback remain unverified here.
