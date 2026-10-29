# Changelog

All notable changes to Project Auralis will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Added the Auralis SVG wordmark, live preview empty state, and a saved Reduce Motion option that follows Windows client-animation accessibility settings.

### Changed
- Redesigned the Windows Studio as a monochrome, card-based layout with custom native controls and a subtle grayscale playback indicator.
- Reorganized the wallpaper, preview, playback, behavior, and resource telemetry panels and clarified first-run guidance.
- Added restrained panel context labels, dividers, hover feedback, and state-based action availability so unavailable controls remain disabled until a source or wallpaper is ready.
- Replaced unmeasured low-memory wording with separate Working Set and Private Bytes explanations and a repeatable resource measurement guide.

## [0.2.0-beta.1] - 2026-10-13

### Added
- **Media Playback Lifecycle & Resource Telemetry (`src/platform/windows_player.rs`, `src/platform/windows.rs`, `src/platform/windows_gui.rs`)**:
  - `MfVideoPlayer::load_and_play` stops and clears the previous media item before loading a replacement, and restores the previous working wallpaper if the replacement fails (`AGENTS.md` Principle 9). Actual decoder memory release depends on Media Foundation and the device; no fixed RAM reduction is claimed.
  - Applying a wallpaper or cycling to a recent wallpaper stops the in-app preview player first, avoiding an extra preview decoder during the transition.
  - `K32EmptyWorkingSet` asks Windows to trim resident process pages after media setup, when hiding the Studio, and periodically during idle background playback. This does not reduce Private Bytes or guarantee that the same pages remain nonresident.
  - **Expanded Process Memory Telemetry**: Reports both OS **Working Set** (`WorkingSetSize`) and **Private Bytes** (`PagefileUsage`) in `auralis status` and the live Studio telemetry readout.
- **Live Playback Speed & Aspect Ratio Scaling Controls (`src/config.rs`, `src/engine.rs`, `src/platform/windows_player.rs`, `src/platform/windows_gui.rs`)**:
  - Live hardware playback speed adjustment (`0.25x` to `2.00x` in `0.25x` steps via `IMFPMediaPlayer::SetRate`) with persistence per display.
  - Aspect ratio scaling toggle between **Fill (Stretch to Full Screen)** (`MFP_ASPECT_RATIO_NONE`) and **Fit (Preserve Aspect Ratio / Letterbox)** (`MFP_ASPECT_RATIO_PRESERVE_ASPECT_RATIO`) with immediate `UpdateVideo()` repaint.
- **Recent Wallpapers History & One-Click Cycling (`src/config.rs`, `src/engine.rs`, `src/platform/windows_gui.rs`)**:
  - Persists up to 10 recently validated and applied wallpapers in `AppConfig::recent_wallpapers` (Schema v2, backward-compatible with v1 configs).
  - Added **Next recent** button in the Studio UI and **Next Recent Wallpaper** in the System Tray context menu to cycle through recent wallpapers with one click.
- **Native macOS Platform Adapter (`src/platform/macos.rs`)**:
  - Native `CoreGraphics` display enumeration (`CGGetActiveDisplayList`, `CGDisplayBounds`, `CGMainDisplayID`, `CGDisplayPixelsWide`, `CGDisplayPixelsHigh`).
  - Native macOS battery and AC power telemetry (`pmset -g batt`), process memory/CPU metrics (`ps`), and LaunchAgent login startup management (`~/Library/LaunchAgents/com.auralis.wallpaper.plist`).

### Fixed
- Eliminated `RefCell` borrow re-entrancy crashes in `src/platform/windows_gui.rs` when clicking `Apply & Play Wallpaper` or changing wallpapers while the `WM_TIMER` loop or Media Foundation `SetMediaItem` inner message pump is active.

## [0.1.0-beta.1] - 2026-10-13

### Added
- **Hardware-Accelerated Windows Media Foundation Video Player (`src/platform/windows_player.rs`)**:
  - Native `IMFPMediaPlayer` (`mfplay.dll`) hardware video decoding and Enhanced Video Renderer (EVR) presentation directly inside `WorkerW` wallpaper host windows behind desktop icons.
  - Synchronous media stream validation (`CreateMediaItemFromURL`) that rejects corrupt or unsupported media files while preserving the previous working wallpaper (`AGENTS.md` Principle 9).
  - Pre-EOS seamless loop seeking (`SetPosition` at `duration - 150ms` while keeping the presentation clock running), eliminating stop/start flicker during loop transitions.
  - Real-time audio mute/unmute, volume control (`0..=100%`), native video resolution inspection (`GetNativeVideoSize`), and live playback position/duration telemetry.
- **Native Win32 Configuration Studio UI (`src/platform/windows_gui.rs`)**:
  - Accessible, DPI-scaled Win32 configuration window (`auralis` or `auralis gui`) with display selector (`CBN_SELCHANGE`), native file browser dialog (`GetOpenFileNameW`), Drag-and-Drop video file support (`DragAcceptFiles` / `WM_DROPFILES`), and an embedded **In-App 16:9 Live Video Preview Panel**.
  - Persistent **Windows System Tray Icon** (`Shell_NotifyIconW`) with double-click restore and right-click context menu (`Open Auralis Studio`, `Pause / Resume Wallpaper`, `Mute / Unmute Audio`, `Stop Wallpaper`, `Quit Auralis`).
  - Windows Login Startup integration (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`) with `auralis gui --minimized` background tray startup mode.
  - Live display/power event handling (`WM_DISPLAYCHANGE` auto-resizing and `WM_POWERBROADCAST` suspend/resume/battery notifications) wired directly to `WallpaperEngine::handle_event`.
  - Resilient background persistence (`AGENTS.md` Principle 9): closing or minimizing the configuration window keeps the live wallpaper active in the system tray, re-launching `auralis` brings the studio window back to the foreground, and `auralis apply` reloads the running session live via `WM_AURALIS_RELOAD_CONFIG`.
- **Native Windows Platform Adapter (`src/platform/windows.rs`)**:
  - Desktop layering behind desktop icons via `Progman`/`WorkerW` (`0x052C`) architecture with verified `SetParent` + `IsChild` attachment, virtual-desktop-to-`WorkerW` coordinate translation, and automatic host window resize on `WM_DISPLAYCHANGE`.
  - Per-Monitor V2 DPI awareness (`DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`) and physical display mode resolution (`EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS)`) so wallpapers always span the full physical monitor resolution (e.g., `1920x1080` or `3840x2160`) regardless of 125%/150% Windows scaling.
  - Custom flicker-free wallpaper host window class (`AuralisWallpaperHostWindowClass`) with null background brush and `WM_ERASEBKGND` suppression so GDI never paints over EVR hardware frames.
  - Measured process resource telemetry (`K32GetProcessMemoryInfo` and `GetProcessTimes`) reporting real RAM working set and CPU time (`AGENTS.md` Principle 7).
  - Stable OS device-based display identifiers (`DISPLAY1`, `DISPLAY2`) and desktop shell filtering (`Progman`, `WorkerW`, `Shell_TrayWnd`) in full-screen occlusion detection.
- **Multi-Display Wallpaper Engine (`src/engine.rs`)**:
  - Coordinates per-display `MediaController` state machines, native host window handles, automatic monitor resize/reconnect handling, user pause state, sleep/wake events, battery conservation, and full-screen pause policies.
- **Safe Configuration & Media Validation (`src/config.rs`, `src/media/mod.rs`)**:
  - Platform-standard configuration path (`%APPDATA%\Auralis\auralis_config.json` on Windows) with `schema_version`, legacy display key migration (`DISPLAY_1` -> `DISPLAY1`), corrupt file backup (`.corrupt.bak`) and recovery warning, and atomic `.tmp`-file-and-rename writes.
- **CLI & Verification Pipeline**:
  - Subcommands: `auralis [gui] [file] [--minimized]`, `auralis play <file> [seconds]`, `auralis apply <file> [display_id]`, `auralis validate <file>`, `auralis status`, and `auralis verify-desktop`.
  - Local 5-gate verification scripts (`scripts/verify.ps1`, `scripts/verify.sh`), GitHub Actions CI matrix (`.github/workflows/ci.yml`), tag-driven draft release workflow (`.github/workflows/release.yml`), and multi-agent engineering workflow with **Antigravity** and **Freebuff** (`knowledge.md`, `docs/MULTI_AGENT_WORKFLOW.md`).

### Known Limitations
- Windows native desktop layering (`WorkerW`), hardware video playback (`mfplay.dll`), Per-Monitor V2 full-screen scaling, In-App Video Preview, System Tray integration, and the Win32 Configuration Studio UI are implemented and verified end-to-end on Windows.
- macOS native `AVFoundation`/`NSWindow` desktop wallpaper layering is not yet implemented (`PlatformCapabilities::detect()` reports `false` on macOS until the native adapter is added in the next milestone).
