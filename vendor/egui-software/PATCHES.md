# Pinned software rasterizer

Source: `egui_software_backend` 0.0.2 from crates.io, upstream commit `e6a5f37264199d85d4370dd5fe903f7830fe8591`.

The original MIT and Apache-2.0 licenses are included. Sound Lock uses the CPU rasterization core with egui 0.37.0. Its optional winit host and example/test-render integrations are excluded; Sound Lock owns its reactive Windows host and GDI presentation, including per-pixel popup transparency. Compatibility changes are confined to egui API adaptation.

Local adaptations: use the direct ahash dependency instead of egui's removed re-export, and apply each ordered ImageDelta within egui 0.37's per-texture delta list. The upstream rasterization and blending algorithms are unchanged.
