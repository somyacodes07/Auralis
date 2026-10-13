# Changelog

All notable changes to Project Auralis will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0-beta.1] - 2026-10-13

### Added
- **Hardware-Accelerated Windows Media Foundation Video Player (`src/platform/windows_player.rs`)**:
  - Native `IMFPMediaPlayer` (`mfplay.dll`) hardware video decoding and Enhanced Video Renderer (EVR) presentation directly inside `WorkerW` wallpaper host windows behind desktop icons.
  - Synchronous media stream validation (`CreateMediaItemFromURL`) that rejects corrupt or unsupported media files while preserving the previous working wallpaper (`AGENTS.md` Principle 9).
  - Pre-EOS seamless loop seeking (`SetPosition` at `duration - 150ms` while keeping the presentation clock running), eliminating stop/start flicker during loop transitions.
  - Real-time audio mute/unmute and volume control (`0..=100%`).
- **Native Win32 Configuration Studio UI (`src/platform/windows_gui.rs`)**:
  - Accessible, lightweight Win32 configuration window (`auralis` or `auralis gui`) with display selector, native file browser dialog (`GetOpenFileNameW`), playback controls (`Apply && Play`, `Pause / Resume`, `Stop && Clear`), audio/loop controls, and power/full-screen policy checkboxes.
  - Live display/power event handling (`WM_DISPLAYCHANGE` and `WM_POWERBROADCAST` suspend/resume/battery notifications) wired directly to `WallpaperEngine::handle_event`.
  - Resilient background persistence (`AGENTS.md` Principle 9): closing or hiding the configuration window while a wallpaper is playing keeps the live wallpaper active in the background, re-launching `auralis` brings the studio window back to the foreground, and `auralis apply` reloads the running session live via `WM_AURALIS_RELOAD_CONFIG`.
- **Native Windows Platform Adapter (`src/platform/windows.rs`)**:
  - Desktop layering behind desktop icons via `Progman`/`WorkerW` (`0x052C`) architecture with verified `SetParent` + `IsChild` attachment and virtual-desktop-to-`WorkerW` coordinate translation.
  - Per-Monitor V2 DPI awareness (`DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`) and physical display mode resolution (`EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS)`) so wallpapers always span the full physical monitor resolution (e.g., `1920x1080` or `3840x2160`) regardless of 125%/150% Windows scaling.
  - Custom flicker-free wallpaper host window class (`AuralisWallpaperHostWindowClass`) with null background brush and `WM_ERASEBKGND` suppression so GDI never paints over EVR hardware frames.
  - Stable OS device-based display identifiers (`DISPLAY1`, `DISPLAY2`) and desktop shell filtering (`Progman`, `WorkerW`, `Shell_TrayWnd`) in full-screen occlusion detection.
- **Multi-Display Wallpaper Engine (`src/engine.rs`)**:
  - Coordinates per-display `MediaController` state machines, native host window handles, user pause state, sleep/wake events, battery conservation, and full-screen pause policies.
- **Safe Configuration & Media Validation (`src/config.rs`, `src/media/mod.rs`)**:
  - Platform-standard configuration path (`%APPDATA%\Auralis\auralis_config.json` on Windows) with `schema_version`, legacy display key migration (`DISPLAY_1` -> `DISPLAY1`), corrupt file backup (`.corrupt.bak`) and recovery warning, and atomic `.tmp`-file-and-rename writes.
- **CLI & Verification Pipeline**:
  - Subcommands: `auralis [gui]`, `auralis play <file> [seconds]`, `auralis apply <file> [display_id]`, `auralis validate <file>`, `auralis status`, and `auralis verify-desktop`.
  - Local 5-gate verification scripts (`scripts/verify.ps1`, `scripts/verify.sh`), GitHub Actions CI matrix (`.github/workflows/ci.yml`), tag-driven draft release workflow (`.github/workflows/release.yml`), and multi-agent engineering workflow with **Antigravity** and **Freebuff** (`knowledge.md`, `docs/MULTI_AGENT_WORKFLOW.md`).

### Known Limitations
- Windows native desktop layering (`WorkerW`), hardware video playback (`mfplay.dll`), Per-Monitor V2 full-screen scaling, and the Win32 Configuration Studio UI are implemented and verified end-to-end on Windows.
- macOS native `AVFoundation`/`NSWindow` desktop wallpaper layering is not yet implemented (`PlatformCapabilities::detect()` reports `false` on macOS until the native adapter is added in the next milestone).
