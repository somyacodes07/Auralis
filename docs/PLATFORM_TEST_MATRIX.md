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
- macOS now has a Studio window (NOT VERIFIED on a real Mac). Full-screen detection uses the on-screen window list and may pause for any other app's window that covers the whole display, including borderless overlays.

## macOS Studio manual checklist

Run on a real Mac with the `.app` from the DMG. Record macOS version, chip, and display layout.

1. Launch with no wallpaper configured: the Studio opens and takes focus, and a Dock icon appears.
2. Close the Studio: the Dock icon disappears and the menu-bar item remains. Reopen it with **Open Auralis Studio…** and with Cmd-, while it is focused.
3. **Browse…** and drag and drop a `.mp4`/`.mov` onto the window: the path field fills, nothing plays yet. Paste a path with Cmd-V.
4. **Preview** plays muted and looping inside the 16:9 area; **Stop preview** clears it. Preview a non-video or missing file: an error appears in the status block.
5. **Set as wallpaper**: the preview stops, the video appears behind desktop icons and ordinary windows on the selected display. Repeat on a second display from the display popup.
6. Close the Studio while a wallpaper plays: the wallpaper keeps playing.
7. Pause/Resume and Stop wallpaper; Mute, Loop, volume and speed steps (labels update), Fill/Fit.
8. Behavior: with **Pause while a full-screen app is open**, put an app in full screen on the wallpaper's display and confirm the wallpaper pauses and resumes after leaving full screen. With **Pause on battery power**, unplug the charger and confirm a pause within a few seconds. **Launch at login** creates or removes `~/Library/LaunchAgents/com.auralis.wallpaper.plist` and matches the menu checkmark. **Reduce motion** removes the wallpaper fade-in.
9. The status block shows the file name only, resolution and position, memory footprint, CPU time, and power source.
10. VoiceOver reads every control, including the volume and speed step buttons.
