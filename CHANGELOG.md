# Changelog

All notable changes to Project Auralis will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- A download website at <https://somyacodes07.github.io/Auralis/> (source in `site/`, deployed by `.github/workflows/pages.yml`). It detects Windows or macOS and links straight to the matching installer from the newest published build, lists every file with its size and SHA-256 checksum, and states what is and isn't verified on each platform.
- Windows builds now include a per-user installer (`auralis-<version>-windows-x86_64-setup.exe`) that needs no administrator rights. It installs to `%LOCALAPPDATA%\Programs\Auralis`, adds a Start menu entry, an optional desktop shortcut, and an optional "Start Auralis when I sign in" entry that matches the in-app Launch at startup toggle, and registers an uninstaller in Settings, Apps. Installing, upgrading, and uninstalling close a running Auralis first. Uninstalling removes the sign-in entry and asks before deleting settings and logs (the default keeps them); a silent uninstall always keeps them. The portable `.zip` is still published.
- CI now runs `cargo deny` (RustSec advisories, a license allow-list, duplicate-version warnings, and crates.io-only sources) on every pull request and push to `main`, and silently installs and uninstalls the Windows installer as a smoke test.
- The desktop app and `auralis play` now write a diagnostics log with startup, wallpaper apply/restore failures, playback errors, configuration recovery, and crash details (panic message and location). It lives in `%LOCALAPPDATA%\Auralis\logs` on Windows and `~/Library/Logs/Auralis` on macOS, rotates at 1 MB, and records media by file name only. `auralis status` shows its path.
- `auralis.exe` now has the Auralis icon in Explorer and version details (product name, description, version, copyright) in its file properties. CI checks that release builds include them.
- `auralis quit` asks a running Windows Studio to quit and restore the desktop, waits up to 5 seconds, and exits 0 when Auralis quit or was not running (for installers and uninstallers).
- The Windows Studio's Wallpaper card shows what the selected display is playing: file name (never the full path), video resolution, and state.
- The Windows Studio can be used from the keyboard: Tab and Shift+Tab move through the controls in visual order, and Space or Enter activates the focused control.
- Added an opt-in macOS setting, **Match lock screen to wallpaper**. It sets a still frame from the wallpaper video as the desktop picture, so the lock screen and Mission Control match it, and restores the original picture when turned off, when the wallpaper stops, or on Quit. macOS does not allow video on the lock screen. Windows lock screen matching is not available yet.
- Added a macOS Studio window, opened from **Open Auralis Studio…** (Cmd-,) in the menu bar and on launch when no wallpaper is running. It mirrors the Windows Studio: display selection, Browse, drag and drop, Next recent, an on-demand preview, Set as wallpaper, pause/resume, stop, mute, loop, volume, speed, Fill/Fit, behavior settings, and live status. Closing it keeps the wallpaper running. Not yet tested on a real Mac.
- macOS now pauses for full-screen apps (on-screen window list) and reads battery state through IOKit, and reports process memory, CPU time, and wallpaper resolution/position.
- macOS builds now ship as one universal (Apple Silicon and Intel) `.dmg` with drag-to-Applications install, replacing the single-architecture `.app.zip` and CLI `.tar.gz`. The app is ad-hoc signed and requires macOS 13 or later.
- Every merge to `main` is now built on Windows and macOS and published as a numbered GitHub prerelease tagged `v<version>-build.<N>`. The build number appears in the Studio title, tray tooltip, `auralis status`, and the macOS bundle version.
- Added a matching monochrome Auralis wordmark and square mark, plus an app-owned Windows icon for the Studio and system tray.
- Added a macOS menu-bar app backed by AppKit and AVFoundation, including wallpaper selection, playback/audio controls, recent cycling, Launch at Login, display changes, and sleep/wake handling.
- Added an unsigned `.app` bundle to macOS tag-release artifacts.
- Added MIT and Apache-2.0 license texts, a dependency license inventory, contribution/security guidance, and a native platform acceptance matrix.

### Fixed
- After Windows Explorer restarts, the Studio re-adds its tray icon and re-creates the desktop wallpaper, which Explorer destroyed. On current Windows 11 builds a restarted Explorer also needed a different request to create the wallpaper layer.
- The Windows Studio re-lays out its controls and fonts when moved to a monitor with a different scale factor, instead of keeping the size it opened with.
- Only one Windows Studio runs per user, even when two launches race. Opening a video with `auralis.exe` while the Studio runs sets it as the primary display's wallpaper in the running Studio instead of starting a second copy.
- The Windows Studio saves settings and removes its wallpaper when Windows signs out or shuts down.
- When sign-in startup is on, the Studio updates the sign-in entry to point at its current location if Auralis was moved or reinstalled elsewhere.
- Fixed the Studio sometimes showing blank cards when restored from the tray.
- Fixed a black macOS desktop when a wallpaper was paused (for example by Pause on battery power or Low Power Mode) right after it was applied. The video now always shows its first frame, and the Studio says why playback is paused and how to change it.
- Fixed the macOS Studio layout overflowing its right edge with a long video path. The path field now shows the file name, status lines wrap instead of truncating, the display menu no longer looks disabled with one display, and controls use a neutral graphite fill instead of the system accent color.
- Fixed the macOS wallpaper never appearing after a video was selected: the AppKit host window was created but never ordered on screen. It is now shown when created, placed one level below the Finder desktop icons (above the system desktop picture), and positioned correctly on secondary displays. If macOS does not show it, Auralis now reports an error instead of failing silently. `auralis verify-desktop` on macOS prints the host window's level, frame, and visibility. Not yet retested on a real Mac.
- Fixed the `&` in "Quit & restore desktop" rendering as an underline mnemonic and the header live label being clipped.
- The Windows Studio now closes the stray console window Windows opens when `auralis.exe` is launched from Explorer, the Start menu, or the sign-in entry. Selecting text in that console could suspend log output and stall the Studio. CLI commands run from a terminal still print normally.

### Changed
- Redesigned the Windows Studio as a monochrome, card-based layout with custom native controls and a subtle grayscale playback indicator.
- Reorganized the wallpaper, preview, playback, behavior, and resource telemetry panels and clarified first-run guidance.
- Added restrained panel context labels, dividers, hover feedback, and state-based action availability so unavailable controls remain disabled until a source or wallpaper is ready.
- Tightened the Windows Studio layout onto a consistent spacing grid, sized the window from its client area, and centered it inside the monitor work area so it no longer extends under the taskbar on 1080p displays at 125% scaling.
- Replaced all-caps panel captions with sentence-case headings and secondary-tone field labels, limited type to regular and semibold weights, and showed the live display name in the header.
- Replaced dotted focus rectangles with solid focus rings and painted compact minus/plus glyphs on volume and speed step buttons while keeping descriptive accessible names.
- Display selector entries now read like `DISPLAY1, 1920 × 1080, primary`, and the video path field scrolls to show the file name.
- Replaced unmeasured low-memory wording with separate Working Set and Private Bytes explanations and a repeatable resource measurement guide.
- Kept the previous macOS video visible until AVFoundation reports the replacement item ready to play.
- Propagated native playback tick failures into recoverable engine state and surfaced the error once instead of silently ignoring it.
- Updated platform documentation and runtime capability reporting for the implemented macOS native host while leaving hardware-decoding claims disabled.

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
- macOS visual desktop layering, playback, display/sleep lifecycle, and startup behavior still need manual acceptance testing on an interactive Mac; CI compilation does not verify them.
- macOS does not report GPU memory and is not signed or notarized. The macOS Studio window has not been tested on a real Mac.
- Windows native layering and playback also require manual acceptance on supported Windows configurations; CI compilation is not proof of visual desktop behavior.
