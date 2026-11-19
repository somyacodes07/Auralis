//! macOS Native Platform Adapter (`CoreGraphics` + `pmset` + `LaunchAgents`)
//!
//! Provides native macOS multi-display enumeration via `CoreGraphics` (`CGGetActiveDisplayList`,
//! `CGDisplayBounds`, `CGMainDisplayID`), battery/AC power inspection via `pmset -g batt`,
//! native AVFoundation wallpaper playback, and login startup registration via
//! `~/Library/LaunchAgents/com.auralis.wallpaper.plist`.

use super::{DisplayInfo, PlatformAdapter, PlatformError, PowerStatus, ProcessResourceMetrics};
use crate::config::VideoAspectMode;
use crate::engine::WallpaperEngine;
use crate::media::PlaybackState;
use crate::{AppConfig, PlatformEvent};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::c_char;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

type CGDirectDisplayID = u32;
type CGError = i32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut CGDirectDisplayID,
        display_count: *mut u32,
    ) -> CGError;
    fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    fn CGMainDisplayID() -> CGDirectDisplayID;
    fn CGDisplayPixelsWide(display: CGDirectDisplayID) -> usize;
    fn CGDisplayPixelsHigh(display: CGDirectDisplayID) -> usize;
}

extern "C" {
    fn auralis_macos_begin_menu(launch_at_login: u8);
    fn auralis_macos_end_menu();
    fn auralis_macos_set_launch_menu_state(enabled: u8);
    fn auralis_macos_pump_events();
    fn auralis_macos_poll_action(path: *mut c_char, path_capacity: usize) -> i32;
    fn auralis_macos_show_message(title: *const c_char, message: *const c_char);
    fn auralis_macos_host_create(display_id: u32) -> *mut std::ffi::c_void;
    fn auralis_macos_host_attach(handle: *mut std::ffi::c_void) -> i32;
    fn auralis_macos_host_resize(handle: *mut std::ffi::c_void) -> i32;
    fn auralis_macos_host_start(
        handle: *mut std::ffi::c_void,
        path: *const c_char,
        muted: u8,
        volume: u8,
        speed_percent: u16,
        fit_aspect: u8,
    ) -> i32;
    fn auralis_macos_host_pause(handle: *mut std::ffi::c_void) -> i32;
    fn auralis_macos_host_resume(handle: *mut std::ffi::c_void) -> i32;
    fn auralis_macos_host_stop(handle: *mut std::ffi::c_void) -> i32;
    fn auralis_macos_host_set_audio(handle: *mut std::ffi::c_void, muted: u8, volume: u8) -> i32;
    fn auralis_macos_host_set_rate(handle: *mut std::ffi::c_void, speed_percent: u16) -> i32;
    fn auralis_macos_host_set_aspect(handle: *mut std::ffi::c_void, fit_aspect: u8) -> i32;
    fn auralis_macos_host_tick(handle: *mut std::ffi::c_void, loop_playback: u8) -> i32;
    fn auralis_macos_host_last_error(handle: *mut std::ffi::c_void) -> *const c_char;
    fn auralis_macos_host_detach(handle: *mut std::ffi::c_void) -> i32;
    fn auralis_macos_host_destroy(handle: *mut std::ffi::c_void) -> i32;
}

/// Native macOS platform adapter.
#[derive(Debug, Default)]
pub struct MacOsPlatformAdapter {
    display_ids: Mutex<HashMap<String, CGDirectDisplayID>>,
}

impl MacOsPlatformAdapter {
    /// Create a new macOS platform adapter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn launch_agent_plist_path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        if home.is_empty() {
            return None;
        }
        Some(
            PathBuf::from(home)
                .join("Library")
                .join("LaunchAgents")
                .join("com.auralis.wallpaper.plist"),
        )
    }

    fn native_display_id(&self, display_id: &str) -> Option<CGDirectDisplayID> {
        self.display_ids
            .lock()
            .ok()
            .and_then(|displays| displays.get(display_id).copied())
    }

    /// Parse `pmset -g batt` output into a typed `PowerStatus`.
    pub(crate) fn parse_pmset_output(output: &str) -> PowerStatus {
        let lower = output.to_ascii_lowercase();
        let on_battery = lower.contains("battery power");
        let on_ac = lower.contains("ac power");

        let mut percent: Option<u8> = None;
        for token in output.split_whitespace() {
            let trimmed = token.trim_end_matches(';').trim_end_matches('%');
            if token.contains('%') {
                if let Ok(val) = trimmed.parse::<u8>() {
                    if val <= 100 {
                        percent = Some(val);
                        break;
                    }
                }
            }
        }

        let low_power = lower.contains("lowpowermode: 1") || lower.contains("low power mode: on");

        if on_battery {
            PowerStatus::OnBattery {
                percent,
                battery_saver: low_power,
            }
        } else if on_ac {
            if low_power {
                PowerStatus::OnBattery {
                    percent,
                    battery_saver: true,
                }
            } else {
                PowerStatus::OnAcPower
            }
        } else {
            PowerStatus::Unknown
        }
    }
}

impl PlatformAdapter for MacOsPlatformAdapter {
    fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        const MAX_DISPLAYS: u32 = 32;
        let mut display_ids = [0u32; MAX_DISPLAYS as usize];
        let mut count: u32 = 0;

        // SAFETY: CGGetActiveDisplayList writes up to MAX_DISPLAYS IDs into `display_ids` and sets `count`.
        let err =
            unsafe { CGGetActiveDisplayList(MAX_DISPLAYS, display_ids.as_mut_ptr(), &mut count) };
        // SAFETY: CGMainDisplayID returns the display ID of the primary macOS screen.
        let main_id = unsafe { CGMainDisplayID() };

        if (err != 0 || count == 0) && main_id != 0 {
            display_ids[0] = main_id;
            count = 1;
        } else if err != 0 || count == 0 {
            return Err(PlatformError::DisplayEnumerationFailed(format!(
                "CGGetActiveDisplayList returned CGError {err} (count={count})"
            )));
        }

        let mut displays = Vec::with_capacity(count as usize);
        let mut native_ids = HashMap::with_capacity(count as usize);

        for (idx, &cg_id) in display_ids[..count as usize].iter().enumerate() {
            // SAFETY: CGDisplayBounds, CGDisplayPixelsWide, and CGDisplayPixelsHigh query valid active display IDs.
            let (bounds, px_w, px_h) = unsafe {
                (
                    CGDisplayBounds(cg_id),
                    CGDisplayPixelsWide(cg_id),
                    CGDisplayPixelsHigh(cg_id),
                )
            };

            let width = if px_w > 0 {
                px_w as u32
            } else {
                bounds.size.width.max(1.0).round() as u32
            };
            let height = if px_h > 0 {
                px_h as u32
            } else {
                bounds.size.height.max(1.0).round() as u32
            };

            let id = format!("CGDISPLAY{cg_id}");
            native_ids.insert(id.clone(), cg_id);
            displays.push(DisplayInfo {
                id,
                name: format!("macOS Display #{cg_id}"),
                left: bounds.origin.x.round() as i32,
                top: bounds.origin.y.round() as i32,
                width,
                height,
                is_primary: cg_id == main_id || idx == 0,
            });
        }

        if let Ok(mut display_ids) = self.display_ids.lock() {
            *display_ids = native_ids;
        }
        Ok(displays)
    }

    fn query_power_status(&self) -> Result<PowerStatus, PlatformError> {
        match Command::new("pmset").args(["-g", "batt"]).output() {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                Ok(Self::parse_pmset_output(&text))
            }
            _ => Ok(PowerStatus::Unknown),
        }
    }

    fn is_display_occluded_by_fullscreen(&self, _display: &DisplayInfo) -> bool {
        false
    }

    fn create_wallpaper_host_window(
        &mut self,
        display: &DisplayInfo,
    ) -> Result<isize, PlatformError> {
        let display_id = self.native_display_id(&display.id).ok_or_else(|| {
            PlatformError::WindowCreationFailed(format!(
                "Native display ID for {} is unavailable",
                display.id
            ))
        })?;
        // SAFETY: The native bridge creates an AppKit host for an active display on the main thread.
        let handle = unsafe { auralis_macos_host_create(display_id) };
        if handle.is_null() {
            return Err(PlatformError::WindowCreationFailed(format!(
                "Could not create an AppKit wallpaper host for {}",
                display.id
            )));
        }
        Ok(handle as isize)
    }

    fn resize_wallpaper_host_window(
        &mut self,
        window_hwnd: isize,
        _display: &DisplayInfo,
    ) -> Result<(), PlatformError> {
        // SAFETY: `window_hwnd` is an opaque host handle created by this adapter.
        if unsafe { auralis_macos_host_resize(native_handle(window_hwnd)) } != 0 {
            Ok(())
        } else {
            Err(PlatformError::WindowCreationFailed(
                "Could not resize the AppKit wallpaper host".into(),
            ))
        }
    }

    fn destroy_wallpaper_host_window(&mut self, window_hwnd: isize) -> Result<(), PlatformError> {
        // SAFETY: `window_hwnd` is an opaque host handle created by this adapter and is released once.
        if unsafe { auralis_macos_host_destroy(native_handle(window_hwnd)) } != 0 {
            Ok(())
        } else {
            Err(PlatformError::WindowCreationFailed(
                "Could not destroy the AppKit wallpaper host".into(),
            ))
        }
    }

    fn attach_to_desktop(
        &mut self,
        display_id: &str,
        window_hwnd: isize,
    ) -> Result<(), PlatformError> {
        if self.native_display_id(display_id).is_none() {
            return Err(PlatformError::AttachmentFailed(format!(
                "Display {display_id} is no longer active"
            )));
        }
        // SAFETY: The opaque handle refers to a live AppKit window created for `display_id`.
        if unsafe { auralis_macos_host_attach(native_handle(window_hwnd)) } != 0 {
            Ok(())
        } else {
            Err(PlatformError::AttachmentFailed(format!(
                "Could not place the wallpaper window on {display_id}"
            )))
        }
    }

    fn detach_from_desktop(
        &mut self,
        _display_id: &str,
        window_hwnd: isize,
    ) -> Result<(), PlatformError> {
        // SAFETY: The opaque handle refers to a live AppKit window created by this adapter.
        if unsafe { auralis_macos_host_detach(native_handle(window_hwnd)) } != 0 {
            Ok(())
        } else {
            Err(PlatformError::AttachmentFailed(
                "Could not hide the AppKit wallpaper window".into(),
            ))
        }
    }

    fn start_video_playback(
        &mut self,
        window_hwnd: isize,
        path: &std::path::Path,
        muted: bool,
        volume: u8,
    ) -> Result<(), PlatformError> {
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            PlatformError::MediaPlaybackFailed("Media path contains an embedded NUL byte".into())
        })?;
        // SAFETY: All pointers are valid for the duration of the native call; the host handle is live.
        let started = unsafe {
            auralis_macos_host_start(
                native_handle(window_hwnd),
                path.as_ptr(),
                u8::from(muted),
                volume.min(100),
                100,
                0,
            )
        };
        if started != 0 {
            Ok(())
        } else {
            Err(PlatformError::MediaPlaybackFailed(
                "AVFoundation could not start the selected video".into(),
            ))
        }
    }

    fn pause_video_playback(&mut self, window_hwnd: isize) -> Result<(), PlatformError> {
        // SAFETY: The opaque handle refers to an active host created by this adapter.
        if unsafe { auralis_macos_host_pause(native_handle(window_hwnd)) } != 0 {
            Ok(())
        } else {
            Err(PlatformError::MediaPlaybackFailed(
                "AVFoundation has no active player to pause".into(),
            ))
        }
    }

    fn resume_video_playback(&mut self, window_hwnd: isize) -> Result<(), PlatformError> {
        // SAFETY: The opaque handle refers to an active host created by this adapter.
        if unsafe { auralis_macos_host_resume(native_handle(window_hwnd)) } != 0 {
            Ok(())
        } else {
            Err(PlatformError::MediaPlaybackFailed(
                "AVFoundation has no active player to resume".into(),
            ))
        }
    }

    fn stop_video_playback(&mut self, window_hwnd: isize) -> Result<(), PlatformError> {
        // SAFETY: The opaque handle refers to an active host created by this adapter.
        if unsafe { auralis_macos_host_stop(native_handle(window_hwnd)) } != 0 {
            Ok(())
        } else {
            Err(PlatformError::MediaPlaybackFailed(
                "Could not stop AVFoundation playback".into(),
            ))
        }
    }

    fn set_video_audio(
        &mut self,
        window_hwnd: isize,
        muted: bool,
        volume: u8,
    ) -> Result<(), PlatformError> {
        // SAFETY: The opaque handle refers to an active host created by this adapter.
        if unsafe {
            auralis_macos_host_set_audio(
                native_handle(window_hwnd),
                u8::from(muted),
                volume.min(100),
            )
        } != 0
        {
            Ok(())
        } else {
            Err(PlatformError::MediaPlaybackFailed(
                "Could not update AVFoundation audio settings".into(),
            ))
        }
    }

    fn set_video_playback_rate(
        &mut self,
        window_hwnd: isize,
        speed_percent: u16,
    ) -> Result<(), PlatformError> {
        // SAFETY: The opaque handle refers to an active host created by this adapter.
        if unsafe { auralis_macos_host_set_rate(native_handle(window_hwnd), speed_percent) } != 0 {
            Ok(())
        } else {
            Err(PlatformError::MediaPlaybackFailed(
                "Could not update AVFoundation playback speed".into(),
            ))
        }
    }

    fn set_video_aspect_mode(
        &mut self,
        window_hwnd: isize,
        aspect_mode: VideoAspectMode,
    ) -> Result<(), PlatformError> {
        let fit = u8::from(matches!(aspect_mode, VideoAspectMode::Fit));
        // SAFETY: The opaque handle refers to an active host created by this adapter.
        if unsafe { auralis_macos_host_set_aspect(native_handle(window_hwnd), fit) } != 0 {
            Ok(())
        } else {
            Err(PlatformError::MediaPlaybackFailed(
                "Could not update AVFoundation video scaling".into(),
            ))
        }
    }

    fn tick_video_playback(
        &mut self,
        window_hwnd: isize,
        loop_playback: bool,
    ) -> Result<(), PlatformError> {
        // SAFETY: The opaque handle refers to an active host created by this adapter.
        let result =
            unsafe { auralis_macos_host_tick(native_handle(window_hwnd), u8::from(loop_playback)) };
        if result >= 0 {
            return Ok(());
        }
        // SAFETY: The bridge returns a NUL-terminated error buffer owned by the host.
        let error = unsafe { auralis_macos_host_last_error(native_handle(window_hwnd)) };
        let detail = if error.is_null() {
            "AVFoundation playback failed".to_string()
        } else {
            unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned()
        };
        Err(PlatformError::MediaPlaybackFailed(detail))
    }

    fn query_process_metrics(&self) -> Option<ProcessResourceMetrics> {
        // `ps` reports RSS only; it cannot supply distinct private/peak values, so do not label
        // that single metric as three different measurements.
        None
    }

    fn set_launch_at_startup(&mut self, enabled: bool) -> Result<(), PlatformError> {
        let plist_path = Self::launch_agent_plist_path().ok_or_else(|| {
            PlatformError::WindowCreationFailed("HOME directory not set for LaunchAgents".into())
        })?;

        if enabled {
            if let Some(parent) = plist_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    PlatformError::WindowCreationFailed(format!(
                        "Failed to create LaunchAgents directory: {e}"
                    ))
                })?;
            }
            let exe = std::env::current_exe().map_err(|e| {
                PlatformError::WindowCreationFailed(format!("Failed to resolve executable: {e}"))
            })?;
            let plist_content = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.auralis.wallpaper</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>gui</string>
        <string>--minimized</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
                xml_escape(&exe.display().to_string())
            );
            fs::write(&plist_path, plist_content).map_err(|e| {
                PlatformError::WindowCreationFailed(format!(
                    "Failed to write LaunchAgent plist: {e}"
                ))
            })?;
        } else if plist_path.exists() {
            fs::remove_file(&plist_path).map_err(|error| {
                PlatformError::WindowCreationFailed(format!(
                    "Failed to remove LaunchAgent plist: {error}"
                ))
            })?;
        }
        Ok(())
    }

    fn is_launch_at_startup_enabled(&self) -> bool {
        Self::launch_agent_plist_path().is_some_and(|p| p.exists())
    }
}

fn native_handle(handle: isize) -> *mut std::ffi::c_void {
    handle as *mut std::ffi::c_void
}

/// Pump AppKit events for command-line playback on the process's main thread.
pub fn pump_native_events() {
    // SAFETY: Callers run from the process main thread while using the AppKit adapter.
    unsafe { auralis_macos_pump_events() };
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn show_native_message(title: &str, message: &str) {
    let Ok(title) = CString::new(title.replace('\0', "�")) else {
        return;
    };
    let Ok(message) = CString::new(message.replace('\0', "�")) else {
        return;
    };
    // SAFETY: Both UTF-8 strings are NUL-terminated and remain alive for the native call.
    unsafe { auralis_macos_show_message(title.as_ptr(), message.as_ptr()) };
}

fn primary_display_id(engine: &WallpaperEngine) -> Option<String> {
    engine
        .displays()
        .iter()
        .find(|display| display.is_primary)
        .or_else(|| engine.displays().first())
        .map(|display| display.id.clone())
}

fn apply_primary_wallpaper(
    engine: &mut WallpaperEngine,
    config_path: &std::path::Path,
    path: PathBuf,
) -> Result<(), String> {
    let display_id =
        primary_display_id(engine).ok_or_else(|| "No active display found".to_string())?;
    engine.apply_wallpaper(&display_id, path)?;
    engine
        .config
        .save_to_file(config_path)
        .map_err(|error| format!("Could not save wallpaper settings: {error}"))
}

fn handle_menu_action(
    engine: &mut WallpaperEngine,
    config_path: &std::path::Path,
    action: i32,
    selected_value: Option<String>,
) -> bool {
    match action {
        1 => {
            if let Some(path) = selected_value.map(PathBuf::from) {
                if let Err(error) = apply_primary_wallpaper(engine, config_path, path) {
                    show_native_message("Could not apply wallpaper", &error);
                }
            }
        }
        2 => {
            let displays: Vec<_> = engine
                .displays()
                .iter()
                .filter_map(|display| {
                    let state = &engine.controller(&display.id)?.state;
                    match state {
                        PlaybackState::Playing { .. } => Some((display.id.clone(), true)),
                        PlaybackState::Paused { .. } => Some((display.id.clone(), false)),
                        _ => None,
                    }
                })
                .collect();
            for (display_id, is_playing) in displays {
                let result = if is_playing {
                    engine.pause_display(&display_id)
                } else {
                    engine.resume_display(&display_id)
                };
                if let Err(error) = result {
                    show_native_message("Playback control failed", &error.to_string());
                }
            }
        }
        3 => {
            let audio: Vec<_> = engine
                .displays()
                .iter()
                .filter_map(|display| {
                    let controller = engine.controller(&display.id)?;
                    Some((display.id.clone(), !controller.muted, controller.volume))
                })
                .collect();
            for (display_id, muted, volume) in audio {
                if let Err(error) = engine.set_display_audio(&display_id, muted, volume) {
                    show_native_message("Audio control failed", &error.to_string());
                }
            }
            let _ = engine.config.save_to_file(config_path);
        }
        4 => {
            if let Some(display_id) = primary_display_id(engine) {
                match engine.cycle_next_recent_wallpaper(&display_id) {
                    Ok(_) => {
                        let _ = engine.config.save_to_file(config_path);
                    }
                    Err(error) => show_native_message("Could not change wallpaper", &error),
                }
            }
        }
        5 => {
            let display_ids: Vec<_> = engine
                .displays()
                .iter()
                .map(|display| display.id.clone())
                .collect();
            for display_id in display_ids {
                if let Err(error) = engine.stop_display(&display_id) {
                    show_native_message("Could not stop wallpaper", &error.to_string());
                }
            }
        }
        6 => return true,
        10 => {
            if let Some(enabled) = selected_value.and_then(|value| match value.as_str() {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            }) {
                let previous = engine.is_launch_at_startup_enabled();
                if let Err(error) = engine.set_launch_at_startup(enabled) {
                    show_native_message("Could not update login startup", &error.to_string());
                    // SAFETY: Updates the native menu checkmark on the AppKit main thread.
                    unsafe { auralis_macos_set_launch_menu_state(u8::from(previous)) };
                } else if let Err(error) = engine.config.save_to_file(config_path) {
                    let _ = engine.set_launch_at_startup(previous);
                    engine.config.launch_at_startup = previous;
                    show_native_message("Could not save login startup setting", &error.to_string());
                    // SAFETY: Restores the menu checkmark to the previously registered state.
                    unsafe { auralis_macos_set_launch_menu_state(u8::from(previous)) };
                }
            }
        }
        7 => {
            if let Err(error) = engine.handle_event(PlatformEvent::DisplayConfigurationChanged) {
                show_native_message("Display update failed", &error.to_string());
            }
        }
        8 => {
            let _ = engine.handle_event(PlatformEvent::SystemSleepEntered);
        }
        9 => {
            let _ = engine.handle_event(PlatformEvent::SystemWoke);
        }
        _ => {}
    }
    false
}

/// Run the native macOS menu-bar app and its wallpaper event loop.
///
/// The menu bar exposes file selection, pause/resume, mute, recent cycling, stop, and quit.
pub fn run_macos_gui(initial_video: Option<PathBuf>, _start_minimized: bool) -> Result<(), String> {
    let config_path = AppConfig::default_config_path();
    let (config, load_warning) = AppConfig::load_or_recover(&config_path);
    let mut engine = WallpaperEngine::new(config, Box::new(MacOsPlatformAdapter::new()))
        .map_err(|error| error.to_string())?;
    engine.config.launch_at_startup = engine.is_launch_at_startup_enabled();

    if let Some(path) = initial_video {
        apply_primary_wallpaper(&mut engine, &config_path, path)?;
    } else {
        engine.restore_saved_wallpapers();
    }

    // SAFETY: Startup, event processing, and shutdown run on this process's main thread.
    unsafe { auralis_macos_begin_menu(u8::from(engine.config.launch_at_startup)) };
    if let Some(warning) = load_warning {
        show_native_message("Auralis recovered its settings", &warning);
    }

    let mut should_quit = false;
    while !should_quit {
        // SAFETY: Pumps AppKit events on the same main thread that owns all AppKit objects.
        pump_native_events();
        let mut selected_path = [0i8; 8192];
        // SAFETY: The native bridge writes no more than the provided buffer capacity.
        let action =
            unsafe { auralis_macos_poll_action(selected_path.as_mut_ptr(), selected_path.len()) };
        let selected_value = if matches!(action, 1 | 10) && selected_path[0] != 0 {
            // SAFETY: The bridge writes a NUL-terminated UTF-8 path into `selected_path`.
            Some(
                unsafe { CStr::from_ptr(selected_path.as_ptr()) }
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            None
        };
        should_quit = handle_menu_action(&mut engine, &config_path, action, selected_value);
        if let Err(error) = engine.tick() {
            show_native_message("Wallpaper playback stopped", &error.to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // SAFETY: Removes the status item and notifications before AppKit state is released.
    unsafe { auralis_macos_end_menu() };
    engine
        .config
        .save_to_file(&config_path)
        .map_err(|error| format!("Could not save settings during shutdown: {error}"))?;
    engine.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pmset_output() {
        let ac_sample = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=123) 98%; charged; 0:00 remaining present: true";
        assert_eq!(
            MacOsPlatformAdapter::parse_pmset_output(ac_sample),
            PowerStatus::OnAcPower
        );

        let batt_sample = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=123) 74%; discharging; 4:12 remaining present: true";
        assert_eq!(
            MacOsPlatformAdapter::parse_pmset_output(batt_sample),
            PowerStatus::OnBattery {
                percent: Some(74),
                battery_saver: false,
            }
        );
    }

    #[test]
    fn test_macos_adapter_queries() {
        let adapter = MacOsPlatformAdapter::new();
        assert!(adapter.query_power_status().is_ok());
        let _metrics = adapter.query_process_metrics();
    }

    #[test]
    fn launch_agent_paths_are_xml_escaped() {
        assert_eq!(
            xml_escape("/Applications/Auralis & Video.app"),
            "/Applications/Auralis &amp; Video.app"
        );
        assert_eq!(xml_escape("<Auralis>\"'"), "&lt;Auralis&gt;&quot;&apos;");
    }
}
