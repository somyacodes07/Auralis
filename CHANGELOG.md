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

### Known Limitations
- Native wallpaper layering behind desktop icons is currently in foundational architectural design; platform bridges for Windows (Media Foundation) and macOS (AVFoundation) are staged for initial implementation in upcoming tasks.
- Local host environment does not currently have `cargo` in PATH; full local compilation gate requires Rust toolchain installation on developer host.
