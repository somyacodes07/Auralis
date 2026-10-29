# Auralis

A premium, Rust-first live wallpaper application targeting **Windows** and **macOS**.

## Features (`v0.2.0-beta.1`)

- **Native Hardware-Decoded Video Wallpapers (Windows)**: Renders `.mp4`, `.mov`, `.mkv`, `.webm`, and `.m4v` videos directly behind desktop icons (`Progman`/`WorkerW`) using Windows Media Foundation (`mfplay.dll` / `IMFPMediaPlayer`) with Per-Monitor V2 physical DPI scaling and pre-EOS seamless loop seeking.
- **Memory-conscious playback flow**: Auralis stops its preview decoder before applying a wallpaper, avoiding two simultaneous video decoders. Playback keeps the source resolution and uses the platform video path; resource use depends on the selected video and hardware. The Studio reports process Working Set and Private Bytes separately, since trimming resident pages does not reduce committed private memory.
- **Live Playback Speed, Aspect Scaling & Recent Wallpapers**: Adjust playback speed (`0.25x` – `2.00x`), toggle aspect scaling between **Fill (Full Screen)** and **Fit (Letterbox)**, and cycle through up to 10 recent wallpapers from the Studio UI or System Tray menu.
- **Monochrome Configuration Studio UI & In-App Video Preview**: Launch `auralis.exe` (or `cargo run`) to open the native Windows Studio with an embedded video preview, drag-and-drop file loading, monitor selection, playback/audio/loop/speed/scale controls, and live Working Set, Private Bytes, and CPU telemetry. Its SVG wordmark and grayscale live indicator match the refreshed interface; a saved Reduce Motion preference also honors Windows accessibility settings.
- **Windows System Tray & Login Startup Integration**: Minimizes to the Windows System Tray (`Shell_NotifyIconW`) with a right-click quick-control menu (`Open`, `Next Recent Wallpaper`, `Pause / Resume`, `Mute / Unmute`, `Stop`, `Quit`) and optional login startup registration (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`).
- **Resilient Runtime**: Closing or minimizing the configuration window keeps your live wallpaper playing in the background. Failed media replacements automatically preserve your previous working wallpaper, and corrupt config files are safely backed up to `.corrupt.bak`.
- **Smart Power, Sleep/Wake & Full-Screen Policies**: Automatically pauses video playback when a full-screen application/game occludes the monitor, when the system enters sleep (`WM_POWERBROADCAST`), or when running on battery power.
- **Multi-Agent Engineering**: Built collaboratively with **Antigravity** (primary implementation agent) and **Freebuff** (independent verification agent via [`knowledge.md`](knowledge.md)).

## Platform support

| Platform | Current support |
| --- | --- |
| Windows x64 | Native Studio UI, desktop wallpaper layering, hardware video playback, tray controls, and startup integration are implemented. CI builds and tests on Windows; desktop layering and playback still need manual acceptance testing on each supported Windows configuration. |
| macOS | CLI status and video-file validation only. The macOS wallpaper player and configuration UI are not implemented; `gui` currently falls back to status output. |

For detailed engineering guidelines, see [AGENTS.md](AGENTS.md) and [docs/MULTI_AGENT_WORKFLOW.md](docs/MULTI_AGENT_WORKFLOW.md).
The monochrome brand asset is [assets/auralis-logo.svg](assets/auralis-logo.svg); design and resource measurement notes are in [docs/UX_AND_PERFORMANCE.md](docs/UX_AND_PERFORMANCE.md).

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

# Inspect platform capabilities, power status, measured Working Set/Private RAM, and connected displays
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

Releases are tag-driven via [`.github/workflows/release.yml`](.github/workflows/release.yml). The workflow accepts `v*` tags, then verifies SemVer syntax, exact equality with `Cargo.toml`, and that the tagged commit is reachable from the default branch. It runs formatting, Clippy, tests, and a release build on Windows and macOS. The resulting release remains a draft:

1. Push the committed source to the default branch, then push its matching annotated SemVer tag (e.g., `v0.2.0-beta.1`):
   ```powershell
   git push -u origin main
   git tag -a v0.2.0-beta.1 -m "Auralis v0.2.0-beta.1 Beta Release"
   git push origin v0.2.0-beta.1
   ```
2. GitHub Actions creates a **draft** release with `auralis-v<version>-windows-x86_64.zip` and `auralis-v<version>-macos-<arch>.tar.gz` plus SHA-256 checksum files. The macOS binary is a single-architecture CLI artifact, not a universal app or a wallpaper app. Review and publish the draft manually only after platform acceptance testing.
