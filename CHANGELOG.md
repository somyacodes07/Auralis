# Changelog

All notable changes to Project Auralis will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Multi-agent engineering workflow establishing Antigravity as primary implementation agent and Freebuff (`freebuff` v0.0.155) as independent verification agent ([docs/MULTI_AGENT_WORKFLOW.md](docs/MULTI_AGENT_WORKFLOW.md) and [`knowledge.md`](knowledge.md)).
- Local verification quality gate scripts for Windows (`scripts/verify.ps1`) and macOS/Linux (`scripts/verify.sh`).
- GitHub Actions CI matrix workflow (`.github/workflows/ci.yml`) on native `windows-latest` and `macos-latest` runners.
- Deliberate, tag-driven release workflow (`.github/workflows/release.yml`) with version alignment verification and draft release publishing.
- Core Rust crate structure (`Cargo.toml`, `src/lib.rs`, `src/main.rs`) and toolchain definition (`rust-toolchain.toml`).
- Safe configuration persistence module (`src/config.rs`) with atomic file write and media format validation.
- Media playback controller state machine (`src/media/mod.rs`) with volume clamping and resilient fallback recovery.
- Native Windows platform adapter (`src/platform/windows.rs`) supporting:
  - Desktop layering behind desktop icons via `Progman`/`WorkerW` architecture (including Windows 11 24H2+ fallback).
  - Borderless native wallpaper host window creation (`CreateWindowExW`) and deterministic cleanup (`DestroyWindow`).
  - Multi-display monitor enumeration with virtual desktop coordinates via `EnumDisplayMonitors`.
  - System power status inspection (AC vs. Battery & Battery Saver) via `GetSystemPowerStatus`.
  - Full-screen foreground window occlusion detection via `GetForegroundWindow` and `GetWindowRect`.
- Multi-display `WallpaperEngine` orchestrator (`src/engine.rs`) coordinating per-display controllers, sleep/wake events, battery conservation, and full-screen pause policies.
- Interactive CLI commands in `src/main.rs` (`status`, `validate`, `apply`, `verify-desktop`).

### Changed
- Replaced OpenCode (`opencode.json`) with Freebuff (`knowledge.md`) as the repository's independent verification and testing agent.

### Fixed
- Local verification suite (`scripts/verify.ps1`) enhanced to auto-detect and configure local cargo and MinGW linker environment paths.

### Known Limitations
- Hardware video decoder frame presentation (Media Foundation on Windows and AVFoundation on macOS) into the native host window is scheduled for the next milestone.
- Local verification completed and passing on Windows host; macOS native execution is verified via GitHub Actions CI runner.
