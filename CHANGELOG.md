# Changelog

All notable changes to Project Auralis will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0-beta.1] - 2026-10-13

### Added
- **Hardware-Accelerated Windows Media Foundation Video Player (`src/platform/windows_player.rs`)**:
  - Native `IMFPMediaPlayer` (`mfplay.dll`) hardware video decoding and Enhanced Video Renderer (EVR) presentation directly inside `WorkerW` wallpaper host windows behind desktop icons.
  - Synchronous media stream validation (`CreateMediaItemFromURL`) that rejects corrupt or unsupported media files while preserving the previous working wallpaper (`AGENTS.md` Principle 9).
  - Seamless end-of-stream loop restart, real-time audio mute/unmute, and volume control (`0..=100%`).
- **Native Win32 Configuration Studio UI (`src/platform/windows_gui.rs`)**:
  - Accessible, lightweight Win32 configuration window (`auralis` or `auralis gui`) with display selector, native file browser dialog (`GetOpenFileNameW`), playback controls (`Apply & Play`, `Pause / Resume`, `Stop & Clear`), audio/loop controls, and power/full-screen policy checkboxes.
  - Resilient background persistence (`AGENTS.md` Principle 9): closing or hiding the configuration window while a wallpaper is playing keeps the live wallpaper active in the background, and re-launching `auralis` brings the studio window back to the foreground.
- **Native Windows Platform Adapter (`src/platform/windows.rs`)**:
  - Desktop layering behind desktop icons via `Progman`/`WorkerW` (`0x052C`) architecture (including Windows 11 24H2+ fallback and automatic attachment to the interactive `"Default"` desktop).
  - Borderless native wallpaper host window creation (`CreateWindowExW`) and deterministic cleanup (`DestroyWindow`).
  - Multi-display monitor enumeration with virtual desktop coordinates via `EnumDisplayMonitors`.
  - System power status inspection (AC vs. Battery & Battery Saver) via `GetSystemPowerStatus`.
  - Full-screen foreground window occlusion detection via `GetForegroundWindow` and `GetWindowRect`.
- **Multi-Display Wallpaper Engine (`src/engine.rs`)**:
  - Coordinates per-display `MediaController` state machines, native host window handles, user pause state, sleep/wake events, battery conservation, and full-screen pause policies.
- **Safe Configuration & Media Validation (`src/config.rs`, `src/media/mod.rs`)**:
  - Atomic `.tmp`-file-and-rename JSON persistence (`auralis_config.json`) and media format validation (`mp4`, `mov`, `mkv`, `webm`, `m4v`).
- **CLI & Verification Pipeline**:
  - Subcommands: `auralis [gui]`, `auralis play <file> [seconds]`, `auralis apply <file> [display_id]`, `auralis validate <file>`, `auralis status`, and `auralis verify-desktop`.
  - Local 5-gate verification scripts (`scripts/verify.ps1`, `scripts/verify.sh`), GitHub Actions CI matrix (`.github/workflows/ci.yml`), tag-driven release workflow (`.github/workflows/release.yml`), and multi-agent engineering workflow with **Antigravity** and **Freebuff** (`knowledge.md`, `docs/MULTI_AGENT_WORKFLOW.md`).

### Known Limitations
- Windows native desktop layering (`WorkerW`), hardware video playback (`mfplay.dll`), and the Win32 Configuration Studio UI are fully implemented and verified on Windows.
- macOS compiles and passes portable core tests in CI (`macos-latest`), while native `AVFoundation`/`NSWindow` desktop layering on macOS is scheduled for the next milestone.
