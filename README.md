# Wobbly System (Windows)

System-wide Compiz-style wobbly windows for Windows. Drag any window on your desktop — it jiggles. Not just one app. Every app.

## How it works

1. **Mouse hook** (own thread): a left press on a window's title bar is held back. If the mouse moves, it becomes a wobbly drag; if not, the click is replayed, so clicks and double-click-to-maximise still work.
2. **Mirror**: the window is drawn into a transparent, click-through overlay as an 8×8 grid of live DWM thumbnail tiles, and the real window is parked just off-screen.
3. **Physics**: a 4×4 grid of masses joined by springs, the same model Compiz uses. The point you grabbed sticks to the cursor; the rest trails behind. The tiles follow the Bézier surface over the masses, so the window bends.
4. **Release**: the sheet springs back into shape, then the real window is put at the final spot and the mirror disappears.

## Tuning

Set these environment variables before starting it (no rebuild needed):

| Variable | Default | Effect |
|---|---|---|
| `WOBBLY_SPRING` | 2500 | Higher = stiffer, less lag |
| `WOBBLY_FRICTION` | 14 | Lower = more jiggle after release |
| `WOBBLY_SETTLE` | 600 | How hard it snaps back into shape after release |
| `WOBBLY_TILES` | 8 | Tiles per side (2–16). More = smoother bends, more GPU work |

Logs go to `%TEMP%\wobbly-system.log`.

## Known limits

| | |
|---|---|
| Stepped edges | DWM thumbnails can stretch but not warp, so edges bend in small steps (one per tile). A D3D11 renderer could make them smooth curves. |
| Maximised windows | Keep Windows' normal drag (restore on drag). |
| Elevated (admin) windows | Can't be moved from a non-elevated process, so they keep the normal drag. Run wobbly-system as admin to include them. |
| Aero Snap | Drag-to-edge snapping doesn't trigger during a wobbly drag. |
| Exclusive fullscreen games | Bypass DWM entirely. |
| Mixed-DPI monitors | The parked window sits past the right edge of the desktop; an app may rescale itself if that monitor has a different DPI. |

## Build

```bash
cargo build --release
./target/release/wobbly-system.exe
```

Stop it with `taskkill /IM wobbly-system.exe /F`.

To cross-compile from Mac/Linux you'd need the Windows GNU or MSVC targets installed. Easiest path: push, let GitHub Actions build it, download the artifact.

## Why this exists

Compiz Wobbly on Linux was one of the signature desktop effects of its era. Nothing has shipped at that bar on macOS or Windows in modern form. macOS makes it basically impossible (private compositor). Windows is doable but non-trivial. This is the "doable" bet.

Commercial prior art: Stardock's **WindowFX** has done this on Windows for years. This is an attempt at an open-source Rust version.
