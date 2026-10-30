# Auralis UX and performance notes

## Interaction model

Auralis keeps the main workflow short: choose a display, open or drop a local video, preview it, then apply it. Recent wallpapers and the system tray make repeated use quick; loop, mute, volume, speed, aspect, full-screen pause, and battery pause remain close to the active wallpaper controls.

The refreshed Windows Studio uses a restrained grayscale palette, larger action targets, grouped cards, a live preview, and a small playback indicator. The idle preview shows a static play mark and keeps its decoder stopped until the user requests playback. The playback indicator animates only while playback is active. **Reduce motion** persists in the configuration and disables that animation; Windows' client-animation accessibility setting also disables it. The animation does not touch or re-render wallpaper frames.

Context labels and dividers group the controls, and hover feedback makes the custom actions easier to scan. Preview and apply actions stay disabled until a video path is entered; wallpaper playback and tuning controls stay disabled until a display has an active wallpaper. This keeps first-run actions clear and avoids controls that can only do nothing.

Lively Wallpaper is a useful workflow reference for drag-and-drop import, a recent/library-oriented workflow, and pause policies for full-screen use. Auralis borrows those interaction patterns while keeping this release focused on local video playback and native desktop integration. See the [Lively performance guide](https://github.com/rocksdanister/lively/wiki/Performance/797eb97ba115bc49fd9057e2ffa835da9e9904bc) and [wallpaper file guide](https://github.com/rocksdanister/lively/wiki/Lively-Wallpaper-File/aa607ecbea1ff8a1484c60db0f7354d971b46b72).

## Resource behavior and image quality

- Windows playback uses the Media Foundation EVR video path and keeps the source video resolution. Auralis does not lower resolution or frame rate to create a smaller memory number.
- Applying a wallpaper stops the in-app preview player first, avoiding concurrent preview and desktop decoders.
- Full-screen and battery pause policies stop playback when configured. The wallpaper is also paused during sleep and resumed after wake.
- The process telemetry reports Working Set (resident physical pages), Private Bytes (committed private memory), peak Working Set, and CPU time. These values do not include dedicated GPU memory and should not be conflated: `K32EmptyWorkingSet` can trim resident pages without reducing Private Bytes.
- Hardware decoding depends on the codec, profile, driver, and device. The application currently has no user-facing decoder or quality preset and does not claim that every supported file is hardware-decoded.

## Playback backend follow-up

The current Windows player uses Media Foundation's MFPlay (`IMFPMediaPlayer`). Microsoft classifies MFPlay as a legacy API, superseded by MediaPlayer and `IMFMediaEngine`, and recommends those APIs for new Windows playback code. Auralis should keep MFPlay isolated behind its platform player boundary while a small `IMFMediaEngine` proof of concept checks native HWND presentation, loop behavior, display changes, image quality, frame timing, and Working Set, Private Bytes, and GPU memory on the supported Windows versions. Do not treat an API migration as a memory optimization until matched measurements show that it is one. See Microsoft's [MFPlay guidance](https://learn.microsoft.com/en-us/windows/win32/medfound/getting-started-with-mfplay).

## Manual measurement procedure

For a repeatable Windows comparison, record the Windows build, GPU and driver, power mode, display resolution and refresh rate, video codec/profile/resolution/frame rate/bitrate, and whether the video is HDR. Use the same clip and display for each run.

1. Start Auralis with no wallpaper and record Working Set, Private Bytes, CPU time, and GPU memory.
2. Preview the clip by itself; record the same values after playback settles.
3. Apply it as the desktop wallpaper. Confirm preview playback stopped, then record the values again after one and fifteen minutes.
4. In Task Manager, check the process GPU engine for Video Decode and note dedicated/shared GPU memory separately. Watch for dropped frames, visible scaling artifacts, and correct pause/resume behavior.
5. Repeat on battery and in a full-screen application with each pause option enabled.

Do not compare a trimmed Working Set alone or claim an advantage over another wallpaper app without matching the video, display, hardware, power state, observation interval, and measurement method. Windows desktop layering and media behavior still require manual acceptance tests on supported Windows configurations. macOS currently provides CLI diagnostics and validation only; the native wallpaper host and Studio are not implemented.
