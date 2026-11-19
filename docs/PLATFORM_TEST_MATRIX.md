# Platform acceptance test matrix

GitHub Actions compile and test the Rust and native bridge code on native Windows and macOS runners. They do not verify visible wallpaper layering on an interactive user desktop. Run the relevant checks on real devices before calling a platform release-ready.

| Platform | Automated CI | Manual acceptance still required |
| --- | --- | --- |
| Windows x64 | `fmt`, Clippy, tests, release build; interactive runner may exercise host lifecycle | Launch Auralis, select a real local video, confirm desktop icons and ordinary windows remain above the wallpaper, verify multi-display placement, tray restore/quit, pause/mute/loop, sleep/wake, and display reconnect. |
| Windows installer | Inno Setup compile; silent per-user install, `auralis.exe status`, and silent uninstall smoke test (files, Start menu shortcut, Run value removed; settings kept) | Run the setup wizard as a standard user (no UAC prompt), confirm SmartScreen guidance, shortcut icon, "Start Auralis when I sign in" matches the in-app toggle, upgrade over a running Studio (wallpaper restored, files replaced), and an interactive uninstall answering both No and Yes to deleting settings and logs. |
| macOS | `fmt`, Clippy, tests, release build; `build.rs` compiles and links the AppKit/AVFoundation bridge | Launch the `.app`, choose a local video, verify the image sits behind desktop icons and normal windows across Spaces, confirm multi-display placement and removal, test recent cycling, audio, pause, sleep/wake, resolution change, Launch at Login toggle, and quit. |

Record OS version, device model, display layout, video codec/profile/resolution/frame rate, and any known failures with the test result. Do not infer desktop behavior from compilation alone.

## Current status

- Windows desktop layering and playback code are implemented. Per-release interactive acceptance remains required across supported Windows configurations.
- macOS now has a native menu-bar controller, AVFoundation player, AppKit host, and lifecycle hooks. The native path compiles in CI, but visual layering and interactive playback have not yet been accepted on a real Mac.
- The macOS menu-bar app does not include the Windows Studio window, embedded preview, or measured process/GPU telemetry.
