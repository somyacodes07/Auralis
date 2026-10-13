# Auralis

A premium, Rust-first live wallpaper application targeting **Windows** and **macOS**.

## Features (`v0.1.0-beta.1`)

- **Native Hardware-Decoded Video Wallpapers (Windows)**: Renders `.mp4`, `.mov`, `.mkv`, `.webm`, and `.m4v` videos directly behind desktop icons (`Progman`/`WorkerW`) using Windows Media Foundation (`mfplay.dll` / `IMFPMediaPlayer`) with Per-Monitor V2 physical DPI scaling and pre-EOS seamless loop seeking.
- **Native Configuration Studio UI & In-App Video Preview**: Launch `auralis.exe` (or `cargo run`) to open the native Windows Configuration Studio featuring an embedded **16:9 In-App Video Preview Panel**, Drag-and-Drop file loading, monitor selection, live playback/audio/loop controls, and real-time measured RAM/CPU telemetry.
- **System Tray & Windows Startup Integration**: Minimizes cleanly to the Windows System Tray (`Shell_NotifyIconW`) with a right-click quick-control menu (`Open`, `Pause / Resume`, `Mute / Unmute`, `Stop`, `Quit`) and optional Windows Login Startup registration (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`).
- **Resilient Runtime**: Closing or minimizing the configuration window keeps your live wallpaper playing in the background. Failed media replacements automatically preserve your previous working wallpaper, and corrupt config files are safely backed up to `.corrupt.bak`.
- **Smart Power, Sleep/Wake & Full-Screen Policies**: Automatically pauses video playback when a full-screen application/game occludes the monitor, when the system enters sleep (`WM_POWERBROADCAST`), or when running on battery power.
- **Multi-Agent Engineering**: Built collaboratively with **Antigravity** (primary implementation agent) and **Freebuff** (independent verification agent via [`knowledge.md`](knowledge.md)).

For detailed engineering guidelines, see [AGENTS.md](AGENTS.md) and [docs/MULTI_AGENT_WORKFLOW.md](docs/MULTI_AGENT_WORKFLOW.md).

## Usage & CLI Commands

```powershell
# Launch the interactive Auralis Configuration Studio UI (default when double-clicking auralis.exe)
cargo run --release
# or launch directly minimized in the Windows System Tray:
.\target\release\auralis.exe gui --minimized

# Play a live video wallpaper behind desktop icons for N seconds from the CLI
.\target\release\auralis.exe play path\to\video.mp4 15

# Configure a wallpaper for a display (and reload the active Studio session if running)
.\target\release\auralis.exe apply path\to\video.mp4 [DISPLAY1]

# Inspect platform capabilities, power status, measured RAM/CPU usage, and connected displays
.\target\release\auralis.exe status

# Validate a video file for wallpaper compatibility
.\target\release\auralis.exe validate path\to\video.mp4

# Verify native desktop WorkerW window creation and cleanup behind desktop icons
.\target\release\auralis.exe verify-desktop
```

## Quality Gates & Local Verification

Before submitting changes or pushing, all 5 local quality gates must pass:

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

## Publishing a Release & Triggering Auto-Build CI/CD

Releases are tag-driven via [`.github/workflows/release.yml`](.github/workflows/release.yml):
1. Push `main` and the matching annotated SemVer tag (e.g., `v0.1.0-beta.1`) to GitHub:
   ```powershell
   git push -u origin main
   git tag -a v0.1.0-beta.1 -m "Auralis v0.1.0-beta.1 Beta Release"
   git push origin v0.1.0-beta.1
   ```
2. GitHub Actions automatically verifies that the tag matches `Cargo.toml` (`0.1.0-beta.1`), runs the full verification suite on native `windows-latest` and `macos-latest` runners, packages `auralis-v0.1.0-beta.1-x86_64-pc-windows-msvc.zip` and `auralis-v0.1.0-beta.1-aarch64-apple-darwin.tar.gz` with SHA-256 checksums, and creates the GitHub Release.
