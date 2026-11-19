<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/auralis-logo-dark.svg">
    <img src="assets/auralis-logo.svg" alt="Auralis" height="64">
  </picture>
</p>

<p align="center">
  <strong>Live video wallpapers for Windows and macOS.</strong><br>
  Native playback, behind your desktop icons, out of your way.
</p>

<p align="center">
  <a href="https://somyacodes07.github.io/Auralis/"><strong>Download</strong></a> ·
  <a href="https://github.com/somyacodes07/Auralis/releases">All releases</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="#known-limitations">Known limitations</a>
</p>

<p align="center">
  <a href="https://github.com/somyacodes07/Auralis/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/somyacodes07/Auralis/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/somyacodes07/Auralis/releases"><img alt="Latest release" src="https://img.shields.io/github/v/release/somyacodes07/Auralis?include_prereleases&sort=semver&label=release"></a>
  <a href="#license"><img alt="License: MIT or Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-informational"></a>
</p>

> **Status: `1.0.0-rc.1`, release candidate.** Feature-complete for 1.0. It becomes `1.0.0` once acceptance testing on real Windows PCs and Macs passes. Please [report anything that breaks](https://github.com/somyacodes07/Auralis/issues).

---

## What it does

- **Video behind your icons.** Pick a local video and it plays as your desktop wallpaper, below desktop icons and ordinary windows. Windows uses Media Foundation; macOS uses AVFoundation. No custom decoder, no browser engine.
- **One video per display.** Choose a display, set its wallpaper, repeat. Display changes, sleep and wake, and (on Windows) Explorer restarts are handled.
- **Playback controls.** Pause, mute, volume, loop, speed from 0.25× to 2×, and Fill or Fit scaling. Up to 10 recent videos one click away.
- **Gets out of the way.** Optionally pauses while a full-screen app or game is open and on battery power. Honors Reduce Motion.
- **Studio window.** A native configuration window on both systems with a preview, drag and drop, and live status. Closing it keeps the wallpaper playing. Auralis lives in the Windows system tray or the macOS menu bar.
- **Lock screen (macOS).** Opt-in **Match lock screen to wallpaper** sets a still frame of the video as the desktop picture, so the lock screen and Mission Control match. Your original picture comes back when you turn it off or quit.
- **Updates itself.** Once a day Auralis checks GitHub Releases, verifies the SHA-256 checksum, installs the update, and restarts with your wallpaper. You can turn it off.
- **Resilient.** If a new video fails, the previous wallpaper keeps playing. A corrupt settings file is backed up and recovered. Quitting always restores your normal desktop.

## Download and install

Get the right file for your system from the [download page](https://somyacodes07.github.io/Auralis/) or the [Releases page](https://github.com/somyacodes07/Auralis/releases).

| System | File | |
| --- | --- | --- |
| **Windows 10 / 11** (x64) | `auralis-<version>-windows-x86_64-setup.exe` | Recommended |
| Windows, portable | `auralis-<version>-windows-x86_64.zip` | No install, no automatic updates |
| **macOS 13 or later** (Apple Silicon and Intel) | `auralis-<version>-macos-universal.dmg` | |

### Windows

1. Run the `setup.exe`. It installs for your account only, into `%LOCALAPPDATA%\Programs\Auralis`, with no administrator prompt. You can add a desktop shortcut and choose **Start Auralis when I sign in**.
2. The installer is not code-signed yet. If SmartScreen says "Windows protected your PC", choose **More info**, then **Run anyway**.
3. **Uninstall:** Settings › Apps › Installed apps › **Auralis** › **Uninstall**. The uninstaller quits Auralis, restores your desktop, and removes the sign-in entry. It asks whether to delete your settings and logs (the default keeps them). Your videos are never touched.

**Portable zip:** unzip anywhere and run `auralis.exe`. To remove it, quit Auralis, turn off **Launch at startup** if you enabled it, and delete the folder.

### macOS

1. Open the `.dmg` and drag **Auralis** into **Applications**.
2. The app is not notarized yet. The first time, Control-click **Auralis** in Applications, choose **Open**, then **Open** again. If macOS still blocks it, go to System Settings › Privacy & Security and choose **Open Anyway**.
3. Auralis runs in the menu bar. The Studio opens on first launch and from **Open Auralis Studio…** (⌘,). A Dock icon shows only while the Studio is open.
4. **Uninstall:** quit Auralis from the menu bar, turn off **Launch at Login** if you enabled it, and move `Auralis.app` to the Trash. Settings live in `~/Library/Application Support/Auralis` and logs in `~/Library/Logs/Auralis`; delete them too if you like.

### Check a download

Every file has a `.sha256` beside it on the release page.

```powershell
Get-FileHash .\auralis-<version>-windows-x86_64-setup.exe -Algorithm SHA256   # Windows
```

```bash
shasum -a 256 auralis-<version>-macos-universal.dmg   # macOS
```

## Using Auralis

1. Open the Studio (it opens on first launch).
2. Choose a display, then **Browse** for a video or drag one onto the window.
3. **Preview** it if you like, then **Set as wallpaper**.
4. Close the Studio. The wallpaper keeps playing; reopen the Studio or use quick controls from the tray icon (Windows) or menu bar (macOS).

**Recommended videos:** MP4, MOV, or M4V with H.264 or HEVC, at or below your display's resolution. MKV and WebM play only if the operating system has the codec (usually not on macOS).

**Updates:** **Check for Updates** and **Update Automatically** are in the tray and menu-bar menus. Installed copies update in place: Windows runs the new installer silently, macOS replaces the app in Applications. The portable zip only tells you a new version exists. Release candidates and test builds receive prerelease updates; stable releases receive only stable ones.

## Privacy

Auralis collects no telemetry and has no account. Its only network traffic is the update check to `api.github.com` and downloads from this repository's GitHub Releases, over HTTPS, using the system's own `curl`. Turn off **Update Automatically** and it makes no network requests unless you choose **Check for Updates**. The diagnostics log stays on your computer and names videos by file name only.

## System requirements

| | Windows | macOS |
| --- | --- | --- |
| OS | Windows 10 (1803 or later) or Windows 11, x64 | macOS 13 Ventura or later |
| Hardware | Any GPU with hardware video decoding recommended | Apple Silicon or Intel |
| Rights | Standard user, no administrator needed | Standard user |

Memory and CPU use depend on the video's resolution, codec, and frame rate. See [docs/UX_AND_PERFORMANCE.md](docs/UX_AND_PERFORMANCE.md) for how Auralis measures them; the Studio shows live numbers.

## Known limitations

- **Unsigned builds.** Windows SmartScreen and macOS Gatekeeper ask once on first launch. Update checksums prove a file matches its GitHub release, not who built it.
- **Lock screen.** macOS does not allow video on the lock screen, so Auralis uses a still frame, for the current Space only. A dynamic or aerial wallpaper may come back as its static image. Windows has no lock-screen matching yet.
- **Full-screen pause.** On Windows it covers exclusive and borderless full-screen apps on the same monitor. On macOS it uses the on-screen window list and may also pause for borderless overlays that cover the display.
- **Testing coverage.** Manual testing so far covers one Windows 11 PC and one Apple Silicon Mac. Mixed-scale multi-monitor setups, Intel Macs, and Windows 10 are untested. Results are in [docs/PLATFORM_TEST_MATRIX.md](docs/PLATFORM_TEST_MATRIX.md).

## Troubleshooting

`auralis status` prints the diagnostics log location:

- Windows: `%LOCALAPPDATA%\Auralis\logs\auralis.log`
- macOS: `~/Library/Logs/Auralis/auralis.log`

It records startup, wallpaper and playback errors, settings recovery, and crashes, keeps about 1 MB (older lines move to `auralis.log.1`), and names videos by file name only. Attach it when you report a bug.

| Problem | Try |
| --- | --- |
| The desktop picture didn't change (macOS) | Run `/Applications/Auralis.app/Contents/MacOS/Auralis verify-desktop` in Terminal and include its `Host window:` line in your report. |
| The wallpaper is a still image | It is paused. The Studio status says why, for example **Pause on battery power** or a full-screen app. |
| The wallpaper vanished after Explorer crashed (Windows) | It should return within a few seconds. If not, open the Studio and choose **Set as wallpaper** again. |
| A video won't play | Run `auralis validate <file>`, or convert it to H.264 MP4. |

## Command line

The same executable has a CLI (`auralis.exe` on Windows, `Auralis.app/Contents/MacOS/Auralis` on macOS):

```text
auralis                         Open the desktop app (Studio / tray / menu bar)
auralis gui --minimized         Start in the tray without opening the Studio (Windows)
auralis apply <video> [DISPLAY] Set a display's wallpaper; a running Studio reloads it
auralis play <video> <seconds>  Play a wallpaper for a few seconds, then restore
auralis validate <video>        Check a video file
auralis status                  Platform, displays, power, memory, and log location
auralis verify-desktop          Check that the native wallpaper host can be created and removed
auralis update [--check]        Install the newest release (--check only reports it)
auralis quit                    Quit a running Studio and restore the desktop
```

## Build from source

Requires the stable Rust toolchain (pinned in `rust-toolchain.toml`). On macOS, also the Xcode command-line tools; on Windows, the MSVC build tools (the Windows SDK's `rc.exe` embeds the icon and version information).

```bash
cargo run --release
```

Before pushing, run the local quality gates (formatting, Clippy with `-D warnings`, tests, release build, and `git diff --check`):

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1   # Windows
```

```bash
bash scripts/verify.sh   # macOS
```

If no resource compiler is found, a Windows build still succeeds with a warning but `auralis.exe` has no icon; GNU toolchains need a 64-bit `windres` such as MSYS2's `x86_64-w64-mingw32-windres`. Brand assets are generated by [scripts/generate_brand_assets.py](scripts/generate_brand_assets.py); see [docs/BRAND.md](docs/BRAND.md).

## Releases and CI

- Every pull request runs formatting, patch hygiene, `cargo deny`, and native Windows and macOS Clippy, tests, release builds, and packaging.
- Every merge to `main` publishes a numbered **prerelease** such as `v1.0.0-rc.1-build.42`, with the Windows installer, the portable zip, the macOS DMG, and checksums.
- Maintainer releases come from a plain `v<version>` tag that matches `Cargo.toml`. That builds a **draft** release, which is published by hand after acceptance testing.

Details are in [docs/CI_CD_SETUP.md](docs/CI_CD_SETUP.md).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md) for the engineering rules (no fake functionality, verified platform behavior, required local gates), and [SECURITY.md](SECURITY.md) to report a vulnerability. Manual test steps for each platform are in [docs/PLATFORM_TEST_MATRIX.md](docs/PLATFORM_TEST_MATRIX.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Dependency licenses are listed in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
