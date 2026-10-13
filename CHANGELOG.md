# Changelog

All notable changes to Project Auralis will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Multi-agent engineering workflow establishing Antigravity as primary implementation agent and OpenCode as independent verification agent ([docs/MULTI_AGENT_WORKFLOW.md](docs/MULTI_AGENT_WORKFLOW.md)).
- OpenCode configuration (`opencode.json`) defining the `verifier` agent profile in review-and-test mode with write/edit restrictions.
- Local verification quality gate scripts for Windows (`scripts/verify.ps1`) and macOS/Linux (`scripts/verify.sh`).
- GitHub Actions CI matrix workflow (`.github/workflows/ci.yml`) on native `windows-latest` and `macos-latest` runners.
- Deliberate, tag-driven release workflow (`.github/workflows/release.yml`) with version alignment verification and draft release publishing.
- Core Rust crate structure (`Cargo.toml`, `src/lib.rs`, `src/main.rs`) and toolchain definition (`rust-toolchain.toml`).
- Safe configuration persistence module (`src/config.rs`) with atomic file write and media format validation.
- Media playback controller state machine (`src/media/mod.rs`) with volume clamping and resilient fallback recovery.
- Native Windows platform adapter (`src/platform/windows.rs`) supporting desktop layering via Progman/WorkerW architecture and multi-display monitor enumeration via `EnumDisplayMonitors`.

### Fixed
- Local verification suite (`scripts/verify.ps1`) enhanced to auto-detect and configure local cargo and MinGW linker environment paths.

### Known Limitations
- Hardware video decoder backend (Media Foundation on Windows and AVFoundation on macOS) is in architectural staging; full rendering window integration scheduled for next iteration.
- Local verification completed and passing on Windows host; macOS native execution is verified via GitHub Actions CI runner.
