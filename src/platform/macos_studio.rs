//! macOS Studio model: typed actions and view captions.
//!
//! The macOS Studio window and menu bar are drawn by a small Objective-C bridge
//! (`macos_native.m`) that owns only views. Every user action is queued as an integer action
//! code with an optional integer and UTF-8 text payload. This module decodes those codes into
//! [`StudioAction`] and formats the text the bridge displays, so the logic is plain Rust that
//! is compiled and unit-tested on every platform, not only on macOS.

use crate::config::VideoAspectMode;
use crate::media::PlaybackState;
use crate::platform::{DisplayInfo, PowerStatus, ProcessResourceMetrics, VideoPlaybackTelemetry};
use std::path::{Path, PathBuf};

/// Action codes shared with `macos_native.m`. Keep both sides in sync.
pub mod code {
    /// Menu: set the primary display's wallpaper (text: file path).
    pub const SET_PRIMARY_WALLPAPER: i32 = 1;
    /// Menu: pause or resume every display.
    pub const TOGGLE_PAUSE_ALL: i32 = 2;
    /// Menu: mute or unmute every display.
    pub const TOGGLE_MUTE_ALL: i32 = 3;
    /// Menu: cycle the primary display to the next recent wallpaper.
    pub const NEXT_RECENT_PRIMARY: i32 = 4;
    /// Menu: stop every display.
    pub const STOP_ALL: i32 = 5;
    /// Menu or Cmd-Q: quit Auralis.
    pub const QUIT: i32 = 6;
    /// `NSApplicationDidChangeScreenParametersNotification`.
    pub const DISPLAYS_CHANGED: i32 = 7;
    /// `NSWorkspaceWillSleepNotification`.
    pub const SYSTEM_WILL_SLEEP: i32 = 8;
    /// `NSWorkspaceDidWakeNotification`.
    pub const SYSTEM_DID_WAKE: i32 = 9;
    /// Menu or Studio: launch at login (value: 0 or 1).
    pub const SET_LAUNCH_AT_LOGIN: i32 = 10;
    /// Menu or Cmd-,: open the Studio window.
    pub const OPEN_STUDIO: i32 = 11;
    /// The Studio window closed (its preview player is already released).
    pub const STUDIO_CLOSED: i32 = 12;
    /// Studio: select a display (value: zero-based popup index).
    pub const SELECT_DISPLAY: i32 = 13;
    /// Studio: start the preview (text: file path from the path field).
    pub const PREVIEW: i32 = 14;
    /// Studio: stop the preview.
    pub const STOP_PREVIEW: i32 = 15;
    /// Studio: set the selected display's wallpaper (text: file path from the path field).
    pub const APPLY: i32 = 16;
    /// Studio: pause or resume the selected display.
    pub const TOGGLE_PAUSE: i32 = 17;
    /// Studio: stop the selected display.
    pub const STOP: i32 = 18;
    /// Studio: mute checkbox (value: 0 or 1).
    pub const SET_MUTED: i32 = 19;
    /// Studio: loop checkbox (value: 0 or 1).
    pub const SET_LOOP: i32 = 20;
    /// Studio: volume step (value: -1 or 1).
    pub const STEP_VOLUME: i32 = 21;
    /// Studio: speed step (value: -1 or 1).
    pub const STEP_SPEED: i32 = 22;
    /// Studio: scale control (value: 0 fill, 1 fit).
    pub const SET_ASPECT: i32 = 23;
    /// Studio: pause while a full-screen app is open (value: 0 or 1).
    pub const SET_PAUSE_ON_FULLSCREEN: i32 = 24;
    /// Studio: pause on battery power (value: 0 or 1).
    pub const SET_PAUSE_ON_BATTERY: i32 = 25;
    /// Studio: reduce motion (value: 0 or 1).
    pub const SET_REDUCE_MOTION: i32 = 26;
    /// Studio: cycle the selected display to the next recent wallpaper.
    pub const NEXT_RECENT: i32 = 27;
}

/// Direction of a volume or speed step button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The minus button.
    Down,
    /// The plus button.
    Up,
}

/// A typed user or system action from the macOS menu bar, Studio window, or notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StudioAction {
    /// Set the primary display's wallpaper from the menu bar file picker.
    SetPrimaryWallpaper(PathBuf),
    /// Pause or resume every display.
    TogglePauseAll,
    /// Mute or unmute every display.
    ToggleMuteAll,
    /// Cycle the primary display to the next recent wallpaper.
    NextRecentPrimary,
    /// Stop every display.
    StopAll,
    /// Quit Auralis.
    Quit,
    /// The display configuration changed.
    DisplaysChanged,
    /// The system is about to sleep.
    SystemWillSleep,
    /// The system woke.
    SystemDidWake,
    /// Register or remove the login LaunchAgent.
    SetLaunchAtLogin(bool),
    /// Open (or focus) the Studio window.
    OpenStudio,
    /// The Studio window closed.
    StudioClosed,
    /// Select the display at this popup index.
    SelectDisplay(usize),
    /// Start the preview. `None` means the path field was empty.
    Preview(Option<PathBuf>),
    /// Stop the preview.
    StopPreview,
    /// Set the selected display's wallpaper. `None` means the path field was empty.
    Apply(Option<PathBuf>),
    /// Pause or resume the selected display.
    TogglePause,
    /// Stop the selected display.
    Stop,
    /// Mute or unmute the selected display.
    SetMuted(bool),
    /// Turn looping on or off for the selected display.
    SetLoop(bool),
    /// Step the selected display's volume.
    StepVolume(Step),
    /// Step the selected display's playback speed.
    StepSpeed(Step),
    /// Set the selected display's scaling mode.
    SetAspect(VideoAspectMode),
    /// Pause while a full-screen app is open.
    SetPauseOnFullscreen(bool),
    /// Pause on battery power.
    SetPauseOnBattery(bool),
    /// Reduce nonessential motion.
    SetReduceMotion(bool),
    /// Cycle the selected display to the next recent wallpaper.
    NextRecent,
}

fn decode_bool(value: i32) -> Option<bool> {
    match value {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

fn decode_step(value: i32) -> Option<Step> {
    match value {
        -1 => Some(Step::Down),
        1 => Some(Step::Up),
        _ => None,
    }
}

fn decode_path(text: &str) -> Option<PathBuf> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

impl StudioAction {
    /// Decode a queued bridge action. Returns `None` for unknown codes or malformed payloads,
    /// which the caller ignores.
    #[must_use]
    pub fn decode(action: i32, value: i32, text: &str) -> Option<Self> {
        Some(match action {
            code::SET_PRIMARY_WALLPAPER => Self::SetPrimaryWallpaper(decode_path(text)?),
            code::TOGGLE_PAUSE_ALL => Self::TogglePauseAll,
            code::TOGGLE_MUTE_ALL => Self::ToggleMuteAll,
            code::NEXT_RECENT_PRIMARY => Self::NextRecentPrimary,
            code::STOP_ALL => Self::StopAll,
            code::QUIT => Self::Quit,
            code::DISPLAYS_CHANGED => Self::DisplaysChanged,
            code::SYSTEM_WILL_SLEEP => Self::SystemWillSleep,
            code::SYSTEM_DID_WAKE => Self::SystemDidWake,
            code::SET_LAUNCH_AT_LOGIN => Self::SetLaunchAtLogin(decode_bool(value)?),
            code::OPEN_STUDIO => Self::OpenStudio,
            code::STUDIO_CLOSED => Self::StudioClosed,
            code::SELECT_DISPLAY => Self::SelectDisplay(usize::try_from(value).ok()?),
            code::PREVIEW => Self::Preview(decode_path(text)),
            code::STOP_PREVIEW => Self::StopPreview,
            code::APPLY => Self::Apply(decode_path(text)),
            code::TOGGLE_PAUSE => Self::TogglePause,
            code::STOP => Self::Stop,
            code::SET_MUTED => Self::SetMuted(decode_bool(value)?),
            code::SET_LOOP => Self::SetLoop(decode_bool(value)?),
            code::STEP_VOLUME => Self::StepVolume(decode_step(value)?),
            code::STEP_SPEED => Self::StepSpeed(decode_step(value)?),
            code::SET_ASPECT => Self::SetAspect(match value {
                0 => VideoAspectMode::Fill,
                1 => VideoAspectMode::Fit,
                _ => return None,
            }),
            code::SET_PAUSE_ON_FULLSCREEN => Self::SetPauseOnFullscreen(decode_bool(value)?),
            code::SET_PAUSE_ON_BATTERY => Self::SetPauseOnBattery(decode_bool(value)?),
            code::SET_REDUCE_MOTION => Self::SetReduceMotion(decode_bool(value)?),
            code::NEXT_RECENT => Self::NextRecent,
            _ => return None,
        })
    }

    /// Whether this action changes what the Studio window shows, so its state should be
    /// pushed again immediately instead of on the next periodic refresh.
    #[must_use]
    pub const fn refreshes_studio(&self) -> bool {
        !matches!(
            self,
            Self::SystemWillSleep | Self::SystemDidWake | Self::Quit | Self::StudioClosed
        )
    }
}

/// Volume after a step button: 10-point steps clamped to `0..=100`.
#[must_use]
pub fn stepped_volume(current: u8, step: Step) -> u8 {
    match step {
        Step::Down => current.min(100).saturating_sub(10),
        Step::Up => current.saturating_add(10).min(100),
    }
}

/// Whether audio stays muted after a volume step. Raising the volume above zero unmutes,
/// matching the Windows Studio.
#[must_use]
pub const fn muted_after_volume_step(muted: bool, step: Step, new_volume: u8) -> bool {
    match step {
        Step::Up if new_volume > 0 => false,
        _ => muted,
    }
}

/// Playback speed percentage after a step button: 25-point steps clamped to `25..=200`.
#[must_use]
pub fn stepped_speed(current: u16, step: Step) -> u16 {
    match step {
        Step::Down => current.saturating_sub(25).max(25),
        Step::Up => current.saturating_add(25).min(200),
    }
}

fn file_name_or_default(path: &Path) -> String {
    path.file_name().map_or_else(
        || "Wallpaper".to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// One-line playback caption. Shows the file name only, never the full path.
#[must_use]
pub fn playback_caption(state: Option<&PlaybackState>) -> String {
    match state {
        Some(PlaybackState::Playing { path, .. }) => {
            format!("Playing \u{b7} {}", file_name_or_default(path))
        }
        Some(PlaybackState::Paused { path, .. }) => {
            format!("Paused \u{b7} {}", file_name_or_default(path))
        }
        Some(PlaybackState::Loading { path }) => {
            format!("Loading \u{b7} {}", file_name_or_default(path))
        }
        Some(PlaybackState::Stopped { .. }) => "Wallpaper stopped".to_string(),
        Some(PlaybackState::Failed { path, .. }) => {
            format!("Could not play {}", file_name_or_default(path))
        }
        Some(PlaybackState::Unloaded) | None => "No wallpaper running".to_string(),
    }
}

/// Label for the pause/resume button.
#[must_use]
pub const fn pause_button_label(state: Option<&PlaybackState>) -> &'static str {
    match state {
        Some(PlaybackState::Paused { .. }) => "Resume wallpaper",
        _ => "Pause wallpaper",
    }
}

/// Whether pause/stop and the per-wallpaper controls apply to the current state.
#[must_use]
pub const fn wallpaper_controls_enabled(state: Option<&PlaybackState>) -> bool {
    matches!(
        state,
        Some(
            PlaybackState::Playing { .. }
                | PlaybackState::Paused { .. }
                | PlaybackState::Loading { .. }
        )
    )
}

/// Format milliseconds as `mm:ss`.
#[must_use]
pub fn format_mm_ss(ms: u64) -> String {
    let total_secs = ms / 1000;
    format!("{:02}:{:02}", total_secs / 60, total_secs % 60)
}

/// Resolution and position caption from live AVFoundation telemetry.
#[must_use]
pub fn telemetry_caption(telemetry: Option<&VideoPlaybackTelemetry>) -> String {
    let Some(telemetry) = telemetry else {
        return "Video inactive".to_string();
    };
    let resolution = telemetry
        .native_resolution
        .map(|(width, height)| format!("{width} \u{d7} {height}"));
    let time = match (telemetry.position_ms, telemetry.duration_ms) {
        (Some(position), Some(duration)) if duration > 0 => Some(format!(
            "{} / {}",
            format_mm_ss(position.min(duration)),
            format_mm_ss(duration)
        )),
        _ => None,
    };
    match (resolution, time) {
        (Some(resolution), Some(time)) => format!("{resolution} \u{b7} {time}"),
        (Some(resolution), None) => resolution,
        (None, Some(time)) => time,
        (None, None) => "Loading video details".to_string(),
    }
}

/// Power source caption.
#[must_use]
pub fn power_caption(power: Option<&PowerStatus>) -> String {
    match power {
        Some(PowerStatus::OnAcPower) => "Power adapter".to_string(),
        Some(PowerStatus::OnBattery {
            percent,
            battery_saver,
        }) => {
            let level = percent.map_or_else(|| "Battery".to_string(), |p| format!("Battery {p}%"));
            if *battery_saver {
                format!("{level} \u{b7} Low Power Mode")
            } else {
                level
            }
        }
        Some(PowerStatus::Unknown) | None => "Power source unknown".to_string(),
    }
}

/// Process resource caption. On macOS `working_set_bytes` is the resident size and
/// `private_bytes` is the physical footprint that Activity Monitor reports as Memory.
#[must_use]
pub fn metrics_caption(metrics: Option<&ProcessResourceMetrics>) -> String {
    let Some(metrics) = metrics else {
        return "Resource use unavailable".to_string();
    };
    let mib = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
    format!(
        "Memory {:.1} MB footprint \u{b7} {:.1} MB resident \u{b7} CPU time {}s",
        mib(metrics.private_bytes),
        mib(metrics.working_set_bytes),
        metrics.cpu_time_ms / 1000
    )
}

/// Volume readout, for example `Volume 40%`.
#[must_use]
pub fn volume_label(volume: u8) -> String {
    format!("Volume {}%", volume.min(100))
}

/// Speed readout, for example `Speed 1.25×`.
#[must_use]
pub fn speed_label(speed_percent: u16) -> String {
    let percent = speed_percent.clamp(25, 200);
    format!("Speed {}.{:02}\u{d7}", percent / 100, percent % 100)
}

/// Display popup entry, for example `Display 1, 2560 × 1440, primary`.
#[must_use]
pub fn display_label(index: usize, display: &DisplayInfo) -> String {
    let mut label = format!(
        "Display {}, {} \u{d7} {}",
        index + 1,
        display.width,
        display.height
    );
    if display.is_primary {
        label.push_str(", primary");
    }
    label
}

/// Everything the Studio window shows, pushed to the bridge only when it changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StudioView {
    /// Playback caption for the selected display.
    pub status: String,
    /// Resolution/time and preview state, or the most recent error.
    pub details: String,
    /// Memory, CPU time, and power source.
    pub resources: String,
    /// Volume readout.
    pub volume_label: String,
    /// Speed readout.
    pub speed_label: String,
    /// Pause/resume button label.
    pub pause_label: String,
    /// Whether pause, stop, and the per-wallpaper controls are enabled.
    pub controls_enabled: bool,
    /// Mute checkbox.
    pub muted: bool,
    /// Loop checkbox.
    pub loop_playback: bool,
    /// Scale control (`true` for Fit).
    pub fit_aspect: bool,
    /// Pause while a full-screen app is open.
    pub pause_on_fullscreen: bool,
    /// Pause on battery power.
    pub pause_on_battery: bool,
    /// Launch at login.
    pub launch_at_login: bool,
    /// Reduce motion.
    pub reduce_motion: bool,
    /// Whether the preview player is running.
    pub preview_active: bool,
}

/// Saved behavior settings shown as checkboxes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct BehaviorSettings {
    /// Pause while a full-screen app is open.
    pub pause_on_fullscreen: bool,
    /// Pause on battery power.
    pub pause_on_battery: bool,
    /// Launch at login.
    pub launch_at_login: bool,
    /// Reduce motion.
    pub reduce_motion: bool,
}

/// Inputs for [`StudioView::build`], gathered from the engine by the macOS adapter.
#[derive(Debug, Clone, Copy)]
pub struct StudioSnapshot<'a> {
    /// Selected display's playback state.
    pub state: Option<&'a PlaybackState>,
    /// Selected display's mute flag.
    pub muted: bool,
    /// Selected display's volume.
    pub volume: u8,
    /// Selected display's loop flag.
    pub loop_playback: bool,
    /// Selected display's speed percentage.
    pub speed_percent: u16,
    /// Selected display's scaling mode.
    pub aspect_mode: VideoAspectMode,
    /// Live wallpaper telemetry for the selected display.
    pub telemetry: Option<&'a VideoPlaybackTelemetry>,
    /// Current power status.
    pub power: Option<&'a PowerStatus>,
    /// Current process metrics.
    pub metrics: Option<&'a ProcessResourceMetrics>,
    /// File being previewed, if any.
    pub preview: Option<&'a Path>,
    /// Most recent error, shown in place of the details line.
    pub last_error: Option<&'a str>,
    /// Saved behavior settings.
    pub behavior: BehaviorSettings,
}

impl StudioView {
    /// Build the view from a snapshot of engine state.
    #[must_use]
    pub fn build(snapshot: &StudioSnapshot<'_>) -> Self {
        let preview = snapshot.preview.map_or_else(
            || "Preview stopped".to_string(),
            |path| format!("Previewing {}", file_name_or_default(path)),
        );
        let details = snapshot.last_error.map_or_else(
            || format!("{} \u{b7} {preview}", telemetry_caption(snapshot.telemetry)),
            ToString::to_string,
        );
        Self {
            status: playback_caption(snapshot.state),
            details,
            resources: format!(
                "{} \u{b7} {}",
                metrics_caption(snapshot.metrics),
                power_caption(snapshot.power)
            ),
            volume_label: volume_label(snapshot.volume),
            speed_label: speed_label(snapshot.speed_percent),
            pause_label: pause_button_label(snapshot.state).to_string(),
            controls_enabled: wallpaper_controls_enabled(snapshot.state),
            muted: snapshot.muted,
            loop_playback: snapshot.loop_playback,
            fit_aspect: matches!(snapshot.aspect_mode, VideoAspectMode::Fit),
            pause_on_fullscreen: snapshot.behavior.pause_on_fullscreen,
            pause_on_battery: snapshot.behavior.pause_on_battery,
            launch_at_login: snapshot.behavior.launch_at_login,
            reduce_motion: snapshot.behavior.reduce_motion,
            preview_active: snapshot.preview.is_some(),
        }
    }
}

/// Window rectangle in global display coordinates (top-left origin, as `CGWindowListCopyWindowInfo`
/// and `CGDisplayBounds` report them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// Whether a normal-layer window from another app covers the whole display, which is how a
/// full-screen app (native full screen or a borderless full-screen game) appears in the
/// on-screen window list. A one-point tolerance absorbs fractional scaling.
#[must_use]
pub fn window_covers_display(window: ScreenRect, display: ScreenRect) -> bool {
    const TOLERANCE: f64 = 1.0;
    display.width > 0.0
        && display.height > 0.0
        && window.x <= display.x + TOLERANCE
        && window.y <= display.y + TOLERANCE
        && window.x + window.width >= display.x + display.width - TOLERANCE
        && window.y + window.height >= display.y + display.height - TOLERANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(path: &str) -> PlaybackState {
        PlaybackState::Playing {
            path: PathBuf::from(path),
            position_secs: 0.0,
        }
    }

    #[test]
    fn decodes_every_action_code() {
        assert_eq!(
            StudioAction::decode(code::SET_PRIMARY_WALLPAPER, 0, " /Movies/a.mp4 "),
            Some(StudioAction::SetPrimaryWallpaper(PathBuf::from(
                "/Movies/a.mp4"
            )))
        );
        assert_eq!(
            StudioAction::decode(code::SET_PRIMARY_WALLPAPER, 0, ""),
            None
        );
        assert_eq!(
            StudioAction::decode(code::TOGGLE_PAUSE_ALL, 0, ""),
            Some(StudioAction::TogglePauseAll)
        );
        assert_eq!(
            StudioAction::decode(code::QUIT, 0, ""),
            Some(StudioAction::Quit)
        );
        assert_eq!(
            StudioAction::decode(code::SET_LAUNCH_AT_LOGIN, 1, ""),
            Some(StudioAction::SetLaunchAtLogin(true))
        );
        assert_eq!(StudioAction::decode(code::SET_LAUNCH_AT_LOGIN, 7, ""), None);
        assert_eq!(
            StudioAction::decode(code::SELECT_DISPLAY, 2, ""),
            Some(StudioAction::SelectDisplay(2))
        );
        assert_eq!(StudioAction::decode(code::SELECT_DISPLAY, -1, ""), None);
        assert_eq!(
            StudioAction::decode(code::PREVIEW, 0, "  "),
            Some(StudioAction::Preview(None))
        );
        assert_eq!(
            StudioAction::decode(code::APPLY, 0, "/v.mov"),
            Some(StudioAction::Apply(Some(PathBuf::from("/v.mov"))))
        );
        assert_eq!(
            StudioAction::decode(code::STEP_VOLUME, -1, ""),
            Some(StudioAction::StepVolume(Step::Down))
        );
        assert_eq!(
            StudioAction::decode(code::STEP_SPEED, 1, ""),
            Some(StudioAction::StepSpeed(Step::Up))
        );
        assert_eq!(StudioAction::decode(code::STEP_SPEED, 0, ""), None);
        assert_eq!(
            StudioAction::decode(code::SET_ASPECT, 1, ""),
            Some(StudioAction::SetAspect(VideoAspectMode::Fit))
        );
        assert_eq!(StudioAction::decode(code::SET_ASPECT, 3, ""), None);
        assert_eq!(
            StudioAction::decode(code::SET_REDUCE_MOTION, 0, ""),
            Some(StudioAction::SetReduceMotion(false))
        );
        assert_eq!(
            StudioAction::decode(code::NEXT_RECENT, 0, ""),
            Some(StudioAction::NextRecent)
        );
        assert_eq!(StudioAction::decode(0, 0, ""), None);
        assert_eq!(StudioAction::decode(999, 0, ""), None);
    }

    #[test]
    fn steps_clamp_like_the_windows_studio() {
        assert_eq!(stepped_volume(0, Step::Down), 0);
        assert_eq!(stepped_volume(35, Step::Down), 25);
        assert_eq!(stepped_volume(95, Step::Up), 100);
        assert_eq!(stepped_volume(250, Step::Up), 100);
        assert!(!muted_after_volume_step(true, Step::Up, 10));
        assert!(muted_after_volume_step(true, Step::Down, 0));
        assert!(!muted_after_volume_step(false, Step::Down, 0));
        assert_eq!(stepped_speed(25, Step::Down), 25);
        assert_eq!(stepped_speed(100, Step::Up), 125);
        assert_eq!(stepped_speed(200, Step::Up), 200);
    }

    #[test]
    fn captions_show_file_name_but_never_the_full_path() {
        let state = playing("/Users/me/Private Folder/ocean.mp4");
        let caption = playback_caption(Some(&state));
        assert_eq!(caption, "Playing \u{b7} ocean.mp4");
        assert!(!caption.contains("Private Folder"));
        assert_eq!(playback_caption(None), "No wallpaper running");
        assert_eq!(
            playback_caption(Some(&PlaybackState::Failed {
                path: PathBuf::from("/x/broken.mov"),
                error: "decoder".into(),
            })),
            "Could not play broken.mov"
        );
        assert_eq!(pause_button_label(Some(&state)), "Pause wallpaper");
        assert_eq!(
            pause_button_label(Some(&PlaybackState::Paused {
                path: PathBuf::from("/a.mp4"),
                position_secs: 1.0,
            })),
            "Resume wallpaper"
        );
        assert!(wallpaper_controls_enabled(Some(&state)));
        assert!(!wallpaper_controls_enabled(Some(&PlaybackState::Unloaded)));
    }

    #[test]
    fn readouts_use_sentence_case_and_units() {
        assert_eq!(volume_label(40), "Volume 40%");
        assert_eq!(speed_label(100), "Speed 1.00\u{d7}");
        assert_eq!(speed_label(125), "Speed 1.25\u{d7}");
        assert_eq!(speed_label(5), "Speed 0.25\u{d7}");
        assert_eq!(format_mm_ss(83_999), "01:23");
        assert_eq!(
            telemetry_caption(Some(&VideoPlaybackTelemetry {
                native_resolution: Some((3840, 2160)),
                position_ms: Some(12_000),
                duration_ms: Some(30_000),
            })),
            "3840 \u{d7} 2160 \u{b7} 00:12 / 00:30"
        );
        assert_eq!(telemetry_caption(None), "Video inactive");
        assert_eq!(
            power_caption(Some(&PowerStatus::OnBattery {
                percent: Some(64),
                battery_saver: true,
            })),
            "Battery 64% \u{b7} Low Power Mode"
        );
        assert_eq!(
            power_caption(Some(&PowerStatus::OnAcPower)),
            "Power adapter"
        );
        assert_eq!(metrics_caption(None), "Resource use unavailable");
        assert_eq!(
            metrics_caption(Some(&ProcessResourceMetrics {
                working_set_bytes: 100 * 1024 * 1024,
                private_bytes: 50 * 1024 * 1024,
                peak_working_set_bytes: 0,
                cpu_time_ms: 4_500,
            })),
            "Memory 50.0 MB footprint \u{b7} 100.0 MB resident \u{b7} CPU time 4s"
        );
        let display = DisplayInfo {
            id: "CGDISPLAY1".into(),
            name: "macOS Display #1".into(),
            left: 0,
            top: 0,
            width: 2560,
            height: 1440,
            is_primary: true,
        };
        assert_eq!(
            display_label(0, &display),
            "Display 1, 2560 \u{d7} 1440, primary"
        );
    }

    #[test]
    fn view_prefers_errors_and_tracks_preview() {
        let state = playing("/a/b/clip.mov");
        let preview = PathBuf::from("/a/b/next.mp4");
        let mut snapshot = StudioSnapshot {
            state: Some(&state),
            muted: true,
            volume: 0,
            loop_playback: true,
            speed_percent: 100,
            aspect_mode: VideoAspectMode::Fit,
            telemetry: None,
            power: None,
            metrics: None,
            preview: Some(&preview),
            last_error: None,
            behavior: BehaviorSettings {
                pause_on_fullscreen: true,
                ..BehaviorSettings::default()
            },
        };
        let view = StudioView::build(&snapshot);
        assert_eq!(view.status, "Playing \u{b7} clip.mov");
        assert_eq!(view.details, "Video inactive \u{b7} Previewing next.mp4");
        assert!(view.preview_active && view.fit_aspect && view.controls_enabled);
        assert!(view.pause_on_fullscreen && !view.pause_on_battery);

        snapshot.last_error = Some("Choose a video file first.");
        snapshot.preview = None;
        let view = StudioView::build(&snapshot);
        assert_eq!(view.details, "Choose a video file first.");
        assert!(!view.preview_active);
    }

    #[test]
    fn full_screen_detection_requires_covering_the_display() {
        let display = ScreenRect {
            x: 1440.0,
            y: 0.0,
            width: 2560.0,
            height: 1440.0,
        };
        assert!(window_covers_display(display, display));
        assert!(window_covers_display(
            ScreenRect {
                x: 1440.4,
                y: -0.3,
                width: 2559.5,
                height: 1440.6,
            },
            display
        ));
        // A zoomed window leaves the menu bar visible.
        assert!(!window_covers_display(
            ScreenRect {
                x: 1440.0,
                y: 25.0,
                width: 2560.0,
                height: 1415.0,
            },
            display
        ));
        // A full-screen window on another display.
        assert!(!window_covers_display(
            ScreenRect {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
            },
            display
        ));
    }
}
