# Platform acceptance test matrix

GitHub Actions compile and test the Rust and native bridge code on native Windows and macOS runners. They do not verify visible wallpaper layering on an interactive user desktop. Run the relevant checks on real devices before calling a platform release-ready.

| Platform | Automated CI | Manual acceptance still required |
| --- | --- | --- |
| Windows x64 | `fmt`, Clippy, tests, release build; interactive runner may exercise host lifecycle | Launch Auralis, select a real local video, confirm desktop icons and ordinary windows remain above the wallpaper, verify multi-display placement, tray restore/quit, pause/mute/loop, sleep/wake, and display reconnect. Restart Explorer (`taskkill /f /im explorer.exe` then `start explorer.exe`) and confirm the wallpaper and tray icon return; check Tab/Shift+Tab/Space/Enter in the Studio; move the Studio to a monitor with a different scale factor; run `auralis quit`. |
| Windows installer | Inno Setup compile; silent per-user install, `auralis.exe status`, and silent uninstall smoke test (files, Start menu shortcut, Run value removed; settings kept) | Run the setup wizard as a standard user (no UAC prompt), confirm SmartScreen guidance, shortcut icon, "Start Auralis when I sign in" matches the in-app toggle, upgrade over a running Studio (wallpaper restored, files replaced), and an interactive uninstall answering both No and Yes to deleting settings and logs. |
| macOS | `fmt`, Clippy, tests, release build; `build.rs` compiles and links the AppKit/AVFoundation bridge | Launch the `.app`, choose a local video, verify the image sits behind desktop icons and normal windows across Spaces, confirm multi-display placement and removal, test recent cycling, audio, pause, sleep/wake, resolution change, Launch at Login toggle, and quit. |

Record OS version, device model, display layout, video codec/profile/resolution/frame rate, and any known failures with the test result. Do not infer desktop behavior from compilation alone.

## Recorded results

| Date | Platform | Build | Result | Notes |
| --- | --- | --- | --- | --- |
| 2026-11-19 | macOS 15 (Sequoia) or newer, Apple Silicon | `main` before the host-visibility fix (menu-bar app) | **FAIL** (since fixed) | User report: after selecting a video, the original desktop picture stayed unchanged. Cause: the AppKit host window was created but never ordered on screen, at the same level as the system desktop picture. Fixed in `feat/macos-studio` (commit a5a62f4). |
| 2026-11-19 | macOS 15 or newer, Apple Silicon, built-in display 1710 × 1112 | PR #13 integration build (host-visibility fix and Studio) | **PASS** | User report: the wallpaper is visible and plays behind desktop icons. A black desktop seen in one run came from Pause on battery power (Low Power Mode on); later builds show the paused wallpaper's first frame and name the pause reason. |

## Current status

- Windows desktop layering and playback code are implemented. Per-release interactive acceptance remains required across supported Windows configurations.
- macOS has a native menu-bar controller, AVFoundation player, AppKit host, and lifecycle hooks. After the host-visibility fix, the wallpaper was confirmed visible and playing on one Apple Silicon Mac (macOS 15+). Other Macs, multi-display layouts, and sleep/wake still need acceptance testing.
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
11. Lock screen: turn on **Match lock screen to wallpaper** with a wallpaper running. Within a second, the desktop picture (visible in Mission Control and on the lock screen, Ctrl-Cmd-Q) shows a still frame of the video, filled or fitted like the wallpaper. A JPEG appears in `~/Library/Application Support/Auralis/lock-screen/`.
12. Change the wallpaper or Fill/Fit: the still updates and the previous JPEG is deleted.
13. Restore: turning the setting off, **Stop wallpaper**, and **Quit Auralis** each bring back the exact original desktop picture (including its scaling and fill color), and remove the JPEG. After a force-quit the still stays; relaunching with the setting off restores the original.
14. Known limits to note: `setDesktopImageURL` changes the picture for the current Space only, and a dynamic or aerial wallpaper may come back as its static image.
