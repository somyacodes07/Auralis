# Auralis

A premium, Rust-first live wallpaper application targeting **macOS** and **Windows**.

## Mission & Architecture

Auralis delivers a reliable, smooth video wallpaper experience behind desktop icons and application windows:
- **Rust-First Core**: Shared application state, configuration (`src/config.rs`), playback controller (`src/media/`), and multi-display engine orchestrator (`src/engine.rs`).
- **Native Bridges**: Clean, decoupled platform adapters (`src/platform/` — AVFoundation on macOS, Media Foundation & `Progman`/`WorkerW` on Windows).
- **Multi-Agent Engineering**: Built collaboratively with **Antigravity** (primary implementation agent) and **Freebuff** (independent verification agent via [`knowledge.md`](knowledge.md)).

For detailed engineering guidelines, see [AGENTS.md](AGENTS.md) and [docs/MULTI_AGENT_WORKFLOW.md](docs/MULTI_AGENT_WORKFLOW.md).

## CLI Commands

```powershell
# Inspect platform capabilities, power status, and connected displays
cargo run -- status

# Validate a video file for wallpaper compatibility
cargo run -- validate path/to/video.mp4

# Verify native desktop WorkerW window creation and cleanup behind desktop icons
cargo run -- verify-desktop

# Apply a wallpaper to a display and persist configuration
cargo run -- apply path/to/video.mp4 [DISPLAY_1]
```

## Quality Gates & Local Verification

Before submitting changes or pushing, all quality gates must pass:

```powershell
# Windows PowerShell
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1
```

```bash
# macOS / Linux Shell
bash scripts/verify.sh
```

These scripts verify:
1. Formatting: `cargo fmt --all -- --check`
2. Static Analysis: `cargo clippy --workspace --all-targets -- -D warnings`
3. Test Suite: `cargo test --workspace`
4. Release Compilation: `cargo build --workspace --release`
5. Patch Hygiene: `git diff --check`

## Continuous Integration

GitHub Actions workflows are configured to run native compilation and tests on:
- Windows (`windows-latest`)
- macOS (`macos-latest`)

See [docs/CI_CD_SETUP.md](docs/CI_CD_SETUP.md) for CI details.
