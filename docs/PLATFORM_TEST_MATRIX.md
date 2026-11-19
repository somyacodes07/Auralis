# Platform acceptance test matrix

GitHub Actions compile and test the Rust and native bridge code on native Windows and macOS runners. They do not verify visible wallpaper layering on an interactive user desktop. Run the relevant checks on real devices before calling a platform release-ready.

| Platform | Automated CI | Manual acceptance still required |
| --- | --- | --- |
| Windows x64 | `fmt`, Clippy, tests, release build; interactive runner may exercise host lifecycle | Launch Auralis, select a real local video, confirm desktop icons and ordinary windows remain above the wallpaper, verify multi-display placement, tray restore/quit, pause/mute/loop, sleep/wake, and display reconnect. |
| macOS | `fmt`, Clippy, tests, release build; `build.rs` compiles and links the AppKit/AVFoundation bridge | Launch the `.app`, choose a local video, verify the image sits behind desktop icons and normal windows across Spaces, confirm multi-display placement and removal, test recent cycling, audio, pause, sleep/wake, resolution change, Launch at Login toggle, and quit. |

Record OS version, device model, display layout, video codec/profile/resolution/frame rate, and any known failures with the test result. Do not infer desktop behavior from compilation alone.

## Recorded results

| Date | Platform | Build | Result | Notes |
| --- | --- | --- | --- | --- |
| 2026-11-19 | macOS 15 (Sequoia) or newer, Apple Silicon (exact version and model to be recorded) | `main` before the host-visibility fix (menu-bar app) | **FAIL** | User report: after selecting a video from the menu bar, the original desktop picture stays unchanged (no black window, no error). Cause found in code: the AppKit host window was created but never ordered on screen, and its level matched the system desktop picture. Fix pending user retest. |

## Current status

- Windows desktop layering and playback code are implemented. Per-release interactive acceptance remains required across supported Windows configurations.
- macOS now has a native menu-bar controller, AVFoundation player, AppKit host, and lifecycle hooks. The native path compiles in CI. The first real-Mac report (above) failed: the wallpaper never became visible. The host-visibility fix still needs a retest on that Mac.
- To diagnose a macOS host on a real Mac, run `/Applications/Auralis.app/Contents/MacOS/Auralis verify-desktop` in Terminal and include its `Host window:` line (window number, level, frame, visibility, active Space) in the test record.
- The macOS menu-bar app does not include the Windows Studio window, embedded preview, or measured process/GPU telemetry.
