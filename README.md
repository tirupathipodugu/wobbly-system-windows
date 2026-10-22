# Wobbly System (Windows)

System-wide Compiz-style wobbly windows for Windows. Drag any window on your desktop — it jiggles. Not just one app. Every app.

## Status

**Prototype architecture, built on a Mac — I cannot test it here.** The code is written against the real Win32 + DWM API via Microsoft's official `windows` Rust crate. It should compile on Windows. It'll need iteration on a Windows box.

GitHub Actions builds it on `windows-latest` on every push. See `.github/workflows/build.yml`.

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│ Low-level mouse hook (WH_MOUSE_LL)                       │
│   detects left-button-down + motion → drag start/stop    │
└──────────┬───────────────────────────────────────────────┘
           │
           ▼
┌──────────────────────────────────────────────────────────┐
│ Window enumeration (EnumWindows + DWM filters)           │
│   finds the top-level window under the cursor,           │
│   skipping cloaked / tool / shell windows                │
└──────────┬───────────────────────────────────────────────┘
           │
           ▼
┌──────────────────────────────────────────────────────────┐
│ Transparent fullscreen overlay (WS_EX_LAYERED + topmost) │
│   click-through, no activation, no taskbar entry         │
└──────────┬───────────────────────────────────────────────┘
           │
           ▼
┌──────────────────────────────────────────────────────────┐
│ DWM thumbnail (DwmRegisterThumbnail)                     │
│   GPU-side mirror of the source window into the overlay  │
│   — placed at a lagging rectangle each frame             │
└──────────┬───────────────────────────────────────────────┘
           │
           ▼
┌──────────────────────────────────────────────────────────┐
│ Spring-mass sim (10×10 grid)                             │
│   per-point displacement with trailing-edge weighting +  │
│   neighbor cohesion. Centroid drives the thumbnail       │
│   placement in v1; mesh deform via D3D11 is v2.          │
└──────────────────────────────────────────────────────────┘
```

## Known limits (documented, not blockers to v1)

| | Status |
|---|---|
| Single monitor only | v1 scope. Multi-monitor needs one overlay per display + per-monitor DPI |
| Direct3D exclusive fullscreen games | They bypass DWM — can't be thumbnailed |
| UAC-elevated windows | Can't be thumbnailed from a non-elevated process |
| Per-monitor DPI | Manifest + `SetProcessDpiAwarenessContext` not wired yet |
| True mesh deform | v1 lags the whole window as a single rect. v2 upgrades to D3D11 vertex-shader deform |
| Fast-flick spring config | Spring params are hard-coded. v2 adds a settings UI |

## v1 → v2 path

**v1 (what's here):** DWM thumbnail with single-rect lag. Not a true wobble — a lag. Compiz-adjacent but not Compiz-identical.

**v2:** Capture the source window to a D3D11 texture (via `IDXGISwapChain1` present-redirection or `Graphics.Capture.GraphicsCaptureItem` from WinRT). Render it on our overlay through a vertex shader that reads the spring grid as a vertex offset texture. That's the actual Compiz-level deform.

**v3:** multi-monitor overlays, DPI awareness, user settings UI, auto-start, system tray.

## Build

```bash
cargo build --release
./target/release/wobbly-system.exe
```

To cross-compile from Mac/Linux you'd need the Windows GNU or MSVC targets installed. Easiest path: push, let GitHub Actions build it, download the artifact.

## Why this exists

Compiz Wobbly on Linux was one of the signature desktop effects of its era. Nothing has shipped at that bar on macOS or Windows in modern form. macOS makes it basically impossible (private compositor). Windows is doable but non-trivial. This is the "doable" bet.

Commercial prior art: Stardock's **WindowFX** has done this on Windows for years. This is an attempt at an open-source Rust version.
