//! macOS Native Platform Adapter (`CoreGraphics` + IOKit + AppKit + `LaunchAgents`)
//!
//! Provides native macOS multi-display enumeration via `CoreGraphics` (`CGGetActiveDisplayList`,
//! `CGDisplayBounds`, `CGMainDisplayID`), battery/AC power inspection via IOKit power sources
//! (with `pmset -g batt` as a fallback), full-screen detection via `CGWindowListCopyWindowInfo`,
//! native AVFoundation wallpaper playback, the menu-bar app and Studio window, and login startup
//! registration via `~/Library/LaunchAgents/com.auralis.wallpaper.plist`.

use super::lock_screen::{DesktopPictureBackend, LockScreenSync, WallpaperTarget};
use super::macos_studio::{
    cg_display_number, display_label, muted_after_volume_step, stepped_speed, stepped_volume,
    window_covers_display, BehaviorSettings, ScreenRect, StudioAction, StudioSnapshot, StudioView,
};
use super::{
    DisplayInfo, PlatformAdapter, PlatformError, PowerStatus, ProcessResourceMetrics,
    VideoPlaybackTelemetry,
};
use crate::config::{validate_media_file, DesktopPicture, VideoAspectMode};
use crate::engine::WallpaperEngine;
use crate::media::{MediaController, PlaybackState};
use crate::{AppConfig, PlatformEvent};
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::c_char;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a power-source reading is reused. `evaluate_policy` runs every 50 ms tick.
const POWER_CACHE_TTL: Duration = Duration::from_secs(2);
/// How long the on-screen window list is reused for full-screen detection.
const WINDOW_LIST_CACHE_TTL: Duration = Duration::from_secs(1);
/// Upper bound on on-screen windows inspected for full-screen detection.
const MAX_WINDOW_RECTS: usize = 256;

/// Window rectangle written by the bridge (global, top-left origin).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NativeRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Studio state pushed to the bridge. Must match `AuralisStudioState` in `macos_native.m`.
#[repr(C)]
struct NativeStudioState {
    status: *const c_char,
    details: *const c_char,
    resources: *const c_char,
    volume_label: *const c_char,
    speed_label: *const c_char,
    pause_label: *const c_char,
    controls_enabled: u8,
    muted: u8,
    loop_playback: u8,
    fit_aspect: u8,
    pause_on_fullscreen: u8,
    pause_on_battery: u8,
    launch_at_login: u8,
    reduce_motion: u8,
    preview_active: u8,
    match_lock_screen: u8,
}

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
    fn auralis_macos_poll_action(text: *mut c_char, text_capacity: usize, value: *mut i32) -> i32;
    fn auralis_macos_copy_window_rects(rects: *mut NativeRect, capacity: usize) -> usize;
    fn auralis_macos_power_source(percent: *mut i32, low_power: *mut u8) -> i32;
    fn auralis_macos_process_metrics(
        resident: *mut u64,
        footprint: *mut u64,
        resident_peak: *mut u64,
        cpu_ms: *mut u64,
    ) -> i32;
    fn auralis_macos_host_telemetry(
        handle: *mut std::ffi::c_void,
        width: *mut u32,
        height: *mut u32,
        position_ms: *mut i64,
        duration_ms: *mut i64,
    ) -> i32;
    fn auralis_macos_set_reduce_motion(enabled: u8);
    fn auralis_macos_studio_open();
    fn auralis_macos_studio_set_displays(labels: *const c_char, selected: i32);
    fn auralis_macos_studio_set_path(path: *const c_char);
    fn auralis_macos_studio_update(state: *const NativeStudioState);
    fn auralis_macos_studio_start_preview(path: *const c_char, fit_aspect: u8) -> i32;
    fn auralis_macos_studio_stop_preview();
    fn auralis_macos_studio_preview_error(message: *mut c_char, capacity: usize) -> i32;
    fn auralis_macos_desktop_picture_get(
        display_id: u32,
        url: *mut c_char,
        url_capacity: usize,
        scaling: *mut i32,
        allow_clipping: *mut i32,
        rgba: *mut u16,
        has_color: *mut u8,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn auralis_macos_desktop_picture_set(
        display_id: u32,
        url: *const c_char,
        scaling: i32,
        allow_clipping: i32,
        rgba: *const u16,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
    fn auralis_macos_extract_still(
        display_id: u32,
        video_path: *const c_char,
        output_path: *const c_char,
        error: *mut c_char,
        error_capacity: usize,
    ) -> i32;
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
    fn auralis_macos_host_describe(
        handle: *mut std::ffi::c_void,
        buffer: *mut c_char,
        capacity: usize,
    ) -> i32;
    fn auralis_macos_host_detach(handle: *mut std::ffi::c_void) -> i32;
    fn auralis_macos_host_destroy(handle: *mut std::ffi::c_void) -> i32;
}

/// Native macOS platform adapter.
#[derive(Debug, Default)]
pub struct MacOsPlatformAdapter {
    display_ids: Mutex<HashMap<String, CGDirectDisplayID>>,
    power_cache: Mutex<Option<(Instant, PowerStatus)>>,
    window_cache: Mutex<Option<(Instant, Vec<ScreenRect>)>>,
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

    /// Read the power source through IOKit (`IOPSCopyPowerSourcesInfo`) and Low Power Mode
    /// through `NSProcessInfo`, falling back to `pmset -g batt` when IOKit reports nothing.
    fn read_power_status() -> PowerStatus {
        let mut percent: i32 = -1;
        let mut low_power: u8 = 0;
        // SAFETY: The bridge writes one i32 and one u8 through the provided valid pointers.
        let source = unsafe { auralis_macos_power_source(&mut percent, &mut low_power) };
        let percent = u8::try_from(percent).ok().filter(|p| *p <= 100);
        match source {
            // Low Power Mode on AC is still treated as "conserve power", as before.
            1 if low_power != 0 => PowerStatus::OnBattery {
                percent,
                battery_saver: true,
            },
            1 => PowerStatus::OnAcPower,
            2 => PowerStatus::OnBattery {
                percent,
                battery_saver: low_power != 0,
            },
            _ => match Command::new("pmset").args(["-g", "batt"]).output() {
                Ok(out) if out.status.success() => {
                    Self::parse_pmset_output(&String::from_utf8_lossy(&out.stdout))
                }
                _ => PowerStatus::Unknown,
            },
        }
    }

    /// On-screen normal-layer windows of other apps, refreshed at most once per second.
    fn other_app_window_rects(&self) -> Vec<ScreenRect> {
        let Ok(mut cache) = self.window_cache.lock() else {
            return Vec::new();
        };
        if let Some((at, rects)) = cache.as_ref() {
            if at.elapsed() < WINDOW_LIST_CACHE_TTL {
                return rects.clone();
            }
        }
        let mut native = vec![NativeRect::default(); MAX_WINDOW_RECTS];
        // SAFETY: The bridge writes at most `native.len()` rectangles and returns the count.
        let count = unsafe { auralis_macos_copy_window_rects(native.as_mut_ptr(), native.len()) };
        let rects: Vec<ScreenRect> = native[..count.min(native.len())]
            .iter()
            .map(|r| ScreenRect {
                x: r.x,
                y: r.y,
                width: r.width,
                height: r.height,
            })
            .collect();
        *cache = Some((Instant::now(), rects.clone()));
        rects
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
        let Ok(mut cache) = self.power_cache.lock() else {
            return Ok(Self::read_power_status());
        };
        if let Some((at, status)) = cache.as_ref() {
            if at.elapsed() < POWER_CACHE_TTL {
                return Ok(status.clone());
            }
        }
        let status = Self::read_power_status();
        *cache = Some((Instant::now(), status.clone()));
        Ok(status)
    }

    /// A display counts as occluded when another app has an on-screen, normal-layer window
    /// covering the whole display: a native full-screen Space or a borderless full-screen game.
    /// Window bounds and layers come from `CGWindowListCopyWindowInfo` and need no Screen
    /// Recording permission (window titles are not read).
    fn is_display_occluded_by_fullscreen(&self, display: &DisplayInfo) -> bool {
        // The window list uses points, as CGDisplayBounds does; `display.width/height` hold
        // CGDisplayPixels values, so prefer CGDisplayBounds when the native ID is known.
        let bounds = self.native_display_id(&display.id).map_or(
            ScreenRect {
                x: f64::from(display.left),
                y: f64::from(display.top),
                width: f64::from(display.width),
                height: f64::from(display.height),
            },
            |id| {
                // SAFETY: CGDisplayBounds accepts any display ID; an inactive one yields an empty rect.
                let rect = unsafe { CGDisplayBounds(id) };
                ScreenRect {
                    x: rect.origin.x,
                    y: rect.origin.y,
                    width: rect.size.width,
                    height: rect.size.height,
                }
            },
        );
        self.other_app_window_rects()
            .into_iter()
            .any(|window| window_covers_display(window, bounds))
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
                "Could not create an AppKit wallpaper host for {} (no matching NSScreen)",
                display.id
            )));
        }
        // The engine expects `create_wallpaper_host_window` to return a host that is already
        // placed on the desktop (the Windows adapter parents it into WorkerW here). Without this
        // the AppKit window was created but never ordered on screen, so nothing was visible.
        // SAFETY: `handle` was just returned non-null by the bridge and is live.
        if unsafe { auralis_macos_host_attach(handle) } == 0 {
            let detail = host_last_error(handle as isize);
            // SAFETY: Releases the host created above exactly once.
            unsafe { auralis_macos_host_destroy(handle) };
            return Err(PlatformError::AttachmentFailed(format!(
                "Could not show the wallpaper window on {}: {detail}",
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
        Err(PlatformError::MediaPlaybackFailed(host_last_error(
            window_hwnd,
        )))
    }

    /// Resident size, physical footprint, and peak resident size from `task_info(TASK_VM_INFO)`,
    /// plus user and system CPU time from `getrusage`. `private_bytes` carries the physical
    /// footprint (the Memory column in Activity Monitor), not Windows Private Bytes.
    fn query_process_metrics(&self) -> Option<ProcessResourceMetrics> {
        let (mut resident, mut footprint, mut peak, mut cpu_ms) = (0u64, 0u64, 0u64, 0u64);
        // SAFETY: The bridge writes four u64 values through the provided valid pointers.
        let ok = unsafe {
            auralis_macos_process_metrics(&mut resident, &mut footprint, &mut peak, &mut cpu_ms)
        };
        (ok != 0).then_some(ProcessResourceMetrics {
            working_set_bytes: resident,
            private_bytes: footprint,
            peak_working_set_bytes: peak,
            cpu_time_ms: cpu_ms,
        })
    }

    fn query_video_telemetry(&self, window_hwnd: isize) -> Option<VideoPlaybackTelemetry> {
        let (mut width, mut height) = (0u32, 0u32);
        let (mut position_ms, mut duration_ms) = (-1i64, -1i64);
        // SAFETY: The opaque handle refers to a live host; the bridge writes four scalars.
        let ok = unsafe {
            auralis_macos_host_telemetry(
                native_handle(window_hwnd),
                &mut width,
                &mut height,
                &mut position_ms,
                &mut duration_ms,
            )
        };
        if ok == 0 {
            return None;
        }
        Some(VideoPlaybackTelemetry {
            native_resolution: (width > 0 && height > 0).then_some((width, height)),
            position_ms: u64::try_from(position_ms).ok(),
            duration_ms: u64::try_from(duration_ms).ok().filter(|d| *d > 0),
        })
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

/// Last error recorded by a host, or a generic message when none was recorded.
fn host_last_error(handle: isize) -> String {
    // SAFETY: `handle` is a live host; the bridge returns a NUL-terminated buffer it owns.
    let error = unsafe { auralis_macos_host_last_error(native_handle(handle)) };
    let detail = if error.is_null() {
        String::new()
    } else {
        // SAFETY: Non-null pointers from the bridge are NUL-terminated and valid for this call.
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    };
    if detail.is_empty() {
        "AVFoundation playback failed".to_string()
    } else {
        detail
    }
}

/// Describe a live host window (number, level, frame, visibility, Space) for diagnostics such
/// as `auralis verify-desktop`.
#[must_use]
pub fn describe_host_window(handle: isize) -> String {
    let mut buffer = [c_char::default(); 1024];
    // SAFETY: The bridge writes a NUL-terminated string of at most `buffer.len()` bytes.
    unsafe {
        auralis_macos_host_describe(native_handle(handle), buffer.as_mut_ptr(), buffer.len())
    };
    // SAFETY: The bridge always NUL-terminates `buffer` when capacity is non-zero.
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
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

/// Convert UI text to a C string, replacing any interior NUL so the conversion cannot fail.
fn c_text(text: &str) -> CString {
    CString::new(text.replace('\0', "\u{fffd}")).unwrap_or_default()
}

/// Convert a path to a C string using its exact file-system bytes.
fn c_path(path: &Path) -> Option<CString> {
    CString::new(path.as_os_str().as_bytes()).ok()
}

fn show_native_message(title: &str, message: &str) {
    let title = c_text(title);
    let message = c_text(message);
    // SAFETY: Both UTF-8 strings are NUL-terminated and remain alive for the native call.
    unsafe { auralis_macos_show_message(title.as_ptr(), message.as_ptr()) };
}

fn set_launch_menu_state(enabled: bool) {
    // SAFETY: Updates the native menu checkmark on the AppKit main thread.
    unsafe { auralis_macos_set_launch_menu_state(u8::from(enabled)) };
}

fn primary_display_id(engine: &WallpaperEngine) -> Option<String> {
    engine
        .displays()
        .iter()
        .find(|display| display.is_primary)
        .or_else(|| engine.displays().first())
        .map(|display| display.id.clone())
}

/// One queued bridge action: code, integer payload, and UTF-8 text payload.
fn poll_native_action() -> Option<(i32, i32, String)> {
    let mut text = [c_char::default(); 8192];
    let mut value: i32 = 0;
    // SAFETY: The bridge writes at most `text.len()` bytes (always NUL-terminated) and one i32.
    let action = unsafe { auralis_macos_poll_action(text.as_mut_ptr(), text.len(), &mut value) };
    if action == 0 {
        return None;
    }
    // SAFETY: The bridge zero-fills and NUL-terminates `text` before returning a non-zero code.
    let text = unsafe { CStr::from_ptr(text.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    Some((action, value, text))
}

fn native_error(buffer: &[c_char]) -> String {
    // SAFETY: Callers pass buffers the bridge NUL-terminated (it zeroes the first byte first).
    let text = unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    if text.is_empty() {
        "macOS reported no details".to_string()
    } else {
        text
    }
}

/// `NSWorkspace` desktop pictures and AVFoundation still frames for "Match lock screen to
/// wallpaper". Main thread only.
struct NativeDesktopPictures;

impl NativeDesktopPictures {
    fn screen(display_id: &str) -> Result<u32, String> {
        cg_display_number(display_id).ok_or_else(|| format!("Unknown display {display_id}"))
    }
}

impl DesktopPictureBackend for NativeDesktopPictures {
    fn current(&mut self, display_id: &str) -> Result<DesktopPicture, String> {
        let screen = Self::screen(display_id)?;
        let mut url = vec![c_char::default(); 4096];
        let mut error = [c_char::default(); 512];
        let (mut scaling, mut clipping, mut has_color) = (-1i32, -1i32, 0u8);
        let mut rgba = [0u16; 4];
        // SAFETY: Every pointer refers to a live buffer of the stated capacity or one scalar.
        let ok = unsafe {
            auralis_macos_desktop_picture_get(
                screen,
                url.as_mut_ptr(),
                url.len(),
                &mut scaling,
                &mut clipping,
                rgba.as_mut_ptr(),
                &mut has_color,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok == 0 {
            return Err(native_error(&error));
        }
        // SAFETY: On success the bridge NUL-terminated `url`.
        let url = unsafe { CStr::from_ptr(url.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        Ok(DesktopPicture {
            url,
            scaling: (scaling >= 0).then_some(scaling),
            allow_clipping: (clipping >= 0).then_some(clipping != 0),
            fill_rgba: (has_color != 0).then_some(rgba),
        })
    }

    fn set(&mut self, display_id: &str, picture: &DesktopPicture) -> Result<(), String> {
        let screen = Self::screen(display_id)?;
        let url = CString::new(picture.url.as_bytes())
            .map_err(|_| "The picture path contains a NUL byte".to_string())?;
        let rgba = picture.fill_rgba;
        let mut error = [c_char::default(); 512];
        // SAFETY: `url` is NUL-terminated; `rgba` is null or points at four u16 values.
        let ok = unsafe {
            auralis_macos_desktop_picture_set(
                screen,
                url.as_ptr(),
                picture.scaling.unwrap_or(-1),
                picture.allow_clipping.map_or(-1, i32::from),
                rgba.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok == 0 {
            Err(native_error(&error))
        } else {
            Ok(())
        }
    }

    fn extract_still(
        &mut self,
        display_id: &str,
        video: &Path,
        output: &Path,
    ) -> Result<(), String> {
        let screen = Self::screen(display_id)?;
        let video = c_path(video).ok_or_else(|| "The video path is not valid".to_string())?;
        let output = c_path(output).ok_or_else(|| "The still path is not valid".to_string())?;
        let mut error = [c_char::default(); 512];
        // SAFETY: Both paths are NUL-terminated and alive for the call; `error` is writable.
        let ok = unsafe {
            auralis_macos_extract_still(
                screen,
                video.as_ptr(),
                output.as_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if ok == 0 {
            Err(native_error(&error))
        } else {
            Ok(())
        }
    }
}

/// Rust-owned state of the macOS menu-bar app and Studio window. The Objective-C bridge only
/// draws views and queues actions; every decision is made here.
struct MacApp {
    engine: WallpaperEngine,
    config_path: PathBuf,
    studio_open: bool,
    selected_display: usize,
    preview: Option<PathBuf>,
    last_error: Option<String>,
    pushed_view: Option<StudioView>,
    pushed_displays: Option<String>,
    pushed_reduce_motion: Option<bool>,
    /// Displays the user paused, to tell a user pause from a battery or full-screen pause.
    user_paused: HashSet<String>,
    /// Still-frame desktop pictures for "Match lock screen to wallpaper".
    lock_screen: LockScreenSync,
    quit: bool,
}

impl MacApp {
    fn selected_display_id(&self) -> Option<String> {
        self.engine
            .displays()
            .get(self.selected_display)
            .map(|display| display.id.clone())
            .or_else(|| primary_display_id(&self.engine))
    }

    /// Displays with a loaded wallpaper, for lock screen matching.
    fn wallpaper_targets(&self) -> Vec<WallpaperTarget> {
        self.engine
            .displays()
            .iter()
            .filter_map(|display| {
                let controller = self.engine.controller(&display.id)?;
                let video = match &controller.state {
                    PlaybackState::Playing { path, .. }
                    | PlaybackState::Paused { path, .. }
                    | PlaybackState::Loading { path } => path.clone(),
                    _ => return None,
                };
                Some(WallpaperTarget {
                    display_id: display.id.clone(),
                    video,
                    aspect: controller.aspect_mode,
                })
            })
            .collect()
    }

    /// Keep desktop pictures in step with the wallpapers (or restore them when the setting is
    /// off). Native work happens only when a wallpaper or its scaling changes.
    fn sync_lock_screen(&mut self, restore_everything: bool) {
        let targets = self.wallpaper_targets();
        let enabled = self.engine.config.match_lock_screen;
        let report = if restore_everything {
            self.lock_screen.restore_all(
                &mut self.engine.config.lock_screen_originals,
                &mut NativeDesktopPictures,
            )
        } else {
            self.lock_screen.reconcile(
                enabled,
                &targets,
                &mut self.engine.config.lock_screen_originals,
                &mut NativeDesktopPictures,
            )
        };
        for error in &report.errors {
            eprintln!("Auralis lock screen: {error}");
        }
        if let Some(error) = report.errors.first() {
            self.report("Lock screen", error);
        }
        if report.config_changed {
            self.save_config();
        }
    }

    /// Show an error in the Studio status line when it is open, otherwise as an alert.
    fn report(&mut self, title: &str, message: &str) {
        if self.studio_open {
            self.last_error = Some(format!("{title}: {message}"));
        } else {
            show_native_message(title, message);
        }
    }

    fn save_config(&mut self) {
        if let Err(error) = self.engine.config.save_to_file(&self.config_path) {
            self.report("Could not save settings", &error.to_string());
        }
    }

    fn primary_index(&self) -> usize {
        self.engine
            .displays()
            .iter()
            .position(|display| display.is_primary)
            .unwrap_or(0)
    }

    fn configured_path(&self, display_id: &str) -> Option<PathBuf> {
        self.engine
            .config
            .per_display
            .get(display_id)
            .map(|cfg| cfg.media_path.clone())
    }

    fn push_path_field(&self, path: Option<&Path>) {
        let text = path.and_then(c_path).unwrap_or_default();
        // SAFETY: `text` is NUL-terminated and alive for the call; runs on the main thread.
        unsafe { auralis_macos_studio_set_path(text.as_ptr()) };
    }

    fn push_selected_path(&self) {
        let path = self
            .selected_display_id()
            .and_then(|id| self.configured_path(&id));
        self.push_path_field(path.as_deref());
    }

    fn push_displays(&mut self) {
        if self.selected_display >= self.engine.displays().len() {
            self.selected_display = self.primary_index();
        }
        let labels = self
            .engine
            .displays()
            .iter()
            .enumerate()
            .map(|(index, display)| display_label(index, display))
            .collect::<Vec<_>>()
            .join("\n");
        let key = format!("{labels}\u{1f}{}", self.selected_display);
        if self.pushed_displays.as_deref() == Some(key.as_str()) {
            return;
        }
        let selected = i32::try_from(self.selected_display).unwrap_or(0);
        let labels = c_text(&labels);
        // SAFETY: `labels` is a NUL-terminated, newline-separated UTF-8 list alive for the call.
        unsafe { auralis_macos_studio_set_displays(labels.as_ptr(), selected) };
        self.pushed_displays = Some(key);
    }

    fn push_reduce_motion(&mut self) {
        let enabled = self.engine.config.reduce_motion;
        if self.pushed_reduce_motion != Some(enabled) {
            // SAFETY: Stores a flag the bridge reads when swapping wallpaper layers.
            unsafe { auralis_macos_set_reduce_motion(u8::from(enabled)) };
            self.pushed_reduce_motion = Some(enabled);
        }
    }

    fn push_studio_view(&mut self) {
        if !self.studio_open {
            return;
        }
        self.push_displays();
        let display_id = self.selected_display_id().unwrap_or_default();
        let telemetry = self.engine.video_telemetry(&display_id);
        let power = self.engine.query_power_status().ok();
        let metrics = self.engine.process_metrics();
        let defaults = MediaController::default();
        let controller = self.engine.controller(&display_id).unwrap_or(&defaults);
        let view = StudioView::build(&StudioSnapshot {
            state: Some(&controller.state),
            muted: controller.muted,
            volume: controller.volume,
            loop_playback: controller.loop_playback,
            speed_percent: controller.speed_percent,
            aspect_mode: controller.aspect_mode,
            telemetry: telemetry.as_ref(),
            power: power.as_ref(),
            metrics: metrics.as_ref(),
            preview: self.preview.as_deref(),
            last_error: self.last_error.as_deref(),
            user_paused: self.user_paused.contains(&display_id),
            behavior: BehaviorSettings {
                pause_on_fullscreen: self.engine.config.pause_on_fullscreen,
                pause_on_battery: self.engine.config.pause_on_battery,
                launch_at_login: self.engine.config.launch_at_startup,
                reduce_motion: self.engine.config.reduce_motion,
                match_lock_screen: self.engine.config.match_lock_screen,
            },
        });
        if self.pushed_view.as_ref() == Some(&view) {
            return;
        }
        let status = c_text(&view.status);
        let details = c_text(&view.details);
        let resources = c_text(&view.resources);
        let volume = c_text(&view.volume_label);
        let speed = c_text(&view.speed_label);
        let pause = c_text(&view.pause_label);
        let native = NativeStudioState {
            status: status.as_ptr(),
            details: details.as_ptr(),
            resources: resources.as_ptr(),
            volume_label: volume.as_ptr(),
            speed_label: speed.as_ptr(),
            pause_label: pause.as_ptr(),
            controls_enabled: u8::from(view.controls_enabled),
            muted: u8::from(view.muted),
            loop_playback: u8::from(view.loop_playback),
            fit_aspect: u8::from(view.fit_aspect),
            pause_on_fullscreen: u8::from(view.pause_on_fullscreen),
            pause_on_battery: u8::from(view.pause_on_battery),
            launch_at_login: u8::from(view.launch_at_login),
            reduce_motion: u8::from(view.reduce_motion),
            preview_active: u8::from(view.preview_active),
            match_lock_screen: u8::from(view.match_lock_screen),
        };
        // SAFETY: Every string pointer refers to a CString alive until the end of this scope;
        // the bridge copies them into AppKit controls before returning.
        unsafe { auralis_macos_studio_update(&native) };
        self.pushed_view = Some(view);
    }

    fn open_studio(&mut self) {
        let newly_opened = !self.studio_open;
        if newly_opened {
            self.studio_open = true;
            self.pushed_view = None;
            self.pushed_displays = None;
            self.selected_display = self.primary_index();
        }
        // SAFETY: Creates or focuses the Studio window on the AppKit main thread.
        unsafe { auralis_macos_studio_open() };
        if newly_opened {
            self.push_selected_path();
        }
    }

    fn stop_preview(&mut self) {
        if self.preview.take().is_some() {
            // SAFETY: Releases the Studio preview player on the AppKit main thread.
            unsafe { auralis_macos_studio_stop_preview() };
        }
    }

    fn start_preview(&mut self, path: Option<PathBuf>) {
        let Some(path) = path else {
            self.last_error = Some("Choose a video file to preview.".into());
            return;
        };
        if let Err(error) = validate_media_file(&path) {
            self.last_error = Some(format!("Could not preview this file: {error}"));
            return;
        }
        let Some(native_path) = c_path(&path) else {
            self.last_error = Some("Could not preview this file: the path is not valid.".into());
            return;
        };
        let fit = self
            .selected_display_id()
            .and_then(|id| self.engine.controller(&id))
            .is_some_and(|c| matches!(c.aspect_mode, VideoAspectMode::Fit));
        // SAFETY: `native_path` is NUL-terminated and alive for the call; main thread only.
        let started =
            unsafe { auralis_macos_studio_start_preview(native_path.as_ptr(), u8::from(fit)) };
        if started == 0 {
            self.preview = None;
            self.last_error = Some("Could not start the preview.".into());
        } else {
            self.preview = Some(path);
            self.last_error = None;
        }
    }

    fn poll_preview_failure(&mut self) {
        if self.preview.is_none() {
            return;
        }
        let mut message = [c_char::default(); 512];
        // SAFETY: The bridge writes a NUL-terminated message of at most `message.len()` bytes.
        let failed =
            unsafe { auralis_macos_studio_preview_error(message.as_mut_ptr(), message.len()) };
        if failed != 0 {
            // SAFETY: The bridge NUL-terminated `message` before returning non-zero.
            let detail = unsafe { CStr::from_ptr(message.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            self.stop_preview();
            self.last_error = Some(format!("Could not play the preview: {detail}"));
        }
    }

    fn apply(&mut self, display_id: &str, path: PathBuf) {
        // One decoder at a time: release the preview before the wallpaper player loads.
        self.stop_preview();
        self.user_paused.remove(display_id);
        match self.engine.apply_wallpaper(display_id, path) {
            Ok(()) => {
                self.last_error = None;
                self.save_config();
            }
            Err(error) => self.report("Could not apply wallpaper", &error),
        }
    }

    fn set_launch_at_login(&mut self, enabled: bool) {
        let previous = self.engine.is_launch_at_startup_enabled();
        if let Err(error) = self.engine.set_launch_at_startup(enabled) {
            self.engine.config.launch_at_startup = previous;
            self.report("Could not update login startup", &error.to_string());
        } else if let Err(error) = self.engine.config.save_to_file(&self.config_path) {
            let _ = self.engine.set_launch_at_startup(previous);
            self.engine.config.launch_at_startup = previous;
            self.report("Could not save login startup setting", &error.to_string());
        }
        set_launch_menu_state(self.engine.config.launch_at_startup);
    }

    fn toggle_pause(&mut self, display_ids: Vec<String>) {
        for display_id in display_ids {
            let is_playing = match self.engine.controller(&display_id).map(|c| &c.state) {
                Some(PlaybackState::Playing { .. }) => true,
                Some(PlaybackState::Paused { .. }) => false,
                _ => continue,
            };
            let result = if is_playing {
                self.user_paused.insert(display_id.clone());
                self.engine.pause_display(&display_id)
            } else {
                self.user_paused.remove(&display_id);
                self.engine.resume_display(&display_id)
            };
            if let Err(error) = result {
                self.report("Playback control failed", &error.to_string());
            }
        }
    }

    fn set_audio(&mut self, display_id: &str, muted: bool, volume: u8) {
        if let Err(error) = self.engine.set_display_audio(display_id, muted, volume) {
            self.report("Audio control failed", &error.to_string());
        }
        self.save_config();
    }

    fn all_display_ids(&self) -> Vec<String> {
        self.engine
            .displays()
            .iter()
            .map(|display| display.id.clone())
            .collect()
    }

    fn next_recent(&mut self, display_id: Option<String>) {
        let Some(display_id) = display_id else {
            return;
        };
        self.stop_preview();
        self.user_paused.remove(&display_id);
        match self.engine.cycle_next_recent_wallpaper(&display_id) {
            Ok(path) => {
                self.last_error = None;
                self.save_config();
                if self.studio_open && self.selected_display_id().as_ref() == Some(&display_id) {
                    self.push_path_field(Some(&path));
                }
            }
            Err(error) => self.report("Could not change wallpaper", &error),
        }
    }

    fn handle_selected(&mut self, display_id: &str, action: StudioAction) {
        match action {
            StudioAction::TogglePause => self.toggle_pause(vec![display_id.to_string()]),
            StudioAction::Stop => {
                self.user_paused.remove(display_id);
                if let Err(error) = self.engine.stop_display(display_id) {
                    self.report("Could not stop wallpaper", &error.to_string());
                } else {
                    self.last_error = None;
                }
            }
            StudioAction::SetMuted(muted) => {
                let volume = self.engine.controller(display_id).map_or(0, |c| c.volume);
                self.set_audio(display_id, muted, volume);
            }
            StudioAction::StepVolume(step) => {
                let (muted, volume) = self
                    .engine
                    .controller(display_id)
                    .map_or((true, 0), |c| (c.muted, c.volume));
                let volume = stepped_volume(volume, step);
                let muted = muted_after_volume_step(muted, step, volume);
                self.set_audio(display_id, muted, volume);
            }
            StudioAction::SetLoop(loop_playback) => {
                self.engine.set_display_loop(display_id, loop_playback);
                self.save_config();
            }
            StudioAction::StepSpeed(step) => {
                let speed = self
                    .engine
                    .controller(display_id)
                    .map_or(100, |c| c.speed_percent);
                if let Err(error) = self
                    .engine
                    .set_display_playback_speed(display_id, stepped_speed(speed, step))
                {
                    self.report("Could not change speed", &error.to_string());
                }
                self.save_config();
            }
            StudioAction::SetAspect(mode) => {
                if let Err(error) = self.engine.set_display_aspect_mode(display_id, mode) {
                    self.report("Could not change scaling", &error.to_string());
                }
                self.save_config();
            }
            _ => {}
        }
    }

    fn handle(&mut self, action: StudioAction) {
        match action {
            StudioAction::SetPrimaryWallpaper(path) => match primary_display_id(&self.engine) {
                Some(display_id) => self.apply(&display_id, path),
                None => self.report("Could not apply wallpaper", "No active display found"),
            },
            StudioAction::TogglePauseAll => self.toggle_pause(self.all_display_ids()),
            StudioAction::ToggleMuteAll => {
                let audio: Vec<_> = self
                    .all_display_ids()
                    .into_iter()
                    .filter_map(|id| {
                        let controller = self.engine.controller(&id)?;
                        Some((id, !controller.muted, controller.volume))
                    })
                    .collect();
                for (display_id, muted, volume) in audio {
                    self.set_audio(&display_id, muted, volume);
                }
            }
            StudioAction::NextRecentPrimary => self.next_recent(primary_display_id(&self.engine)),
            StudioAction::NextRecent => self.next_recent(self.selected_display_id()),
            StudioAction::StopAll => {
                for display_id in self.all_display_ids() {
                    self.user_paused.remove(&display_id);
                    if let Err(error) = self.engine.stop_display(&display_id) {
                        self.report("Could not stop wallpaper", &error.to_string());
                    }
                }
            }
            StudioAction::Quit => self.quit = true,
            StudioAction::DisplaysChanged => {
                if let Err(error) = self
                    .engine
                    .handle_event(PlatformEvent::DisplayConfigurationChanged)
                {
                    self.report("Display update failed", &error.to_string());
                }
            }
            StudioAction::SystemWillSleep => {
                let _ = self.engine.handle_event(PlatformEvent::SystemSleepEntered);
            }
            StudioAction::SystemDidWake => {
                let _ = self.engine.handle_event(PlatformEvent::SystemWoke);
            }
            StudioAction::SetLaunchAtLogin(enabled) => self.set_launch_at_login(enabled),
            StudioAction::OpenStudio => self.open_studio(),
            StudioAction::StudioClosed => {
                // The bridge already released the preview player with the window.
                self.studio_open = false;
                self.preview = None;
                self.last_error = None;
                self.pushed_view = None;
            }
            StudioAction::SelectDisplay(index) => {
                if index < self.engine.displays().len() && index != self.selected_display {
                    self.selected_display = index;
                    self.last_error = None;
                    if self.preview.is_none() {
                        self.push_selected_path();
                    }
                }
            }
            StudioAction::Preview(path) => self.start_preview(path),
            StudioAction::StopPreview => {
                self.stop_preview();
                self.last_error = None;
            }
            StudioAction::Apply(path) => match (self.selected_display_id(), path) {
                (Some(display_id), Some(path)) => self.apply(&display_id, path),
                (None, _) => self.report("Could not apply wallpaper", "No active display found"),
                (_, None) => self.last_error = Some("Choose a video file first.".into()),
            },
            StudioAction::SetPauseOnFullscreen(enabled) => {
                self.engine.config.pause_on_fullscreen = enabled;
                self.save_config();
            }
            StudioAction::SetPauseOnBattery(enabled) => {
                self.engine.config.pause_on_battery = enabled;
                self.save_config();
            }
            StudioAction::SetMatchLockScreen(enabled) => {
                self.engine.config.match_lock_screen = enabled;
                self.save_config();
                self.sync_lock_screen(false);
            }
            StudioAction::SetReduceMotion(enabled) => {
                self.engine.config.reduce_motion = enabled;
                self.save_config();
                self.push_reduce_motion();
            }
            selected_action @ (StudioAction::TogglePause
            | StudioAction::Stop
            | StudioAction::SetMuted(_)
            | StudioAction::StepVolume(_)
            | StudioAction::SetLoop(_)
            | StudioAction::StepSpeed(_)
            | StudioAction::SetAspect(_)) => {
                if let Some(display_id) = self.selected_display_id() {
                    self.handle_selected(&display_id, selected_action);
                }
            }
        }
    }
}

/// `~/Library/Application Support/Auralis/lock-screen/`, next to the configuration file.
fn lock_screen_directory(config_path: &Path) -> PathBuf {
    config_path.parent().map_or_else(
        || PathBuf::from("lock-screen"),
        |dir| dir.join("lock-screen"),
    )
}

/// Run the native macOS menu-bar app, its Studio window, and the wallpaper event loop.
///
/// The Studio opens from the menu bar ("Open Auralis Studio…", Cmd-,) and automatically on
/// launch when no wallpaper is running, unless launched with `--minimized` (login startup).
/// Closing the Studio leaves the wallpaper running.
pub fn run_macos_gui(initial_video: Option<PathBuf>, start_minimized: bool) -> Result<(), String> {
    const TICK: Duration = Duration::from_millis(50);
    // Studio status refresh period in ticks (500 ms).
    const STUDIO_REFRESH_TICKS: u32 = 10;
    // Lock screen reconcile period in ticks (1 s).
    const LOCK_SCREEN_SYNC_TICKS: u32 = 20;

    let config_path = AppConfig::default_config_path();
    let (config, load_warning) = AppConfig::load_or_recover(&config_path);
    let mut engine = WallpaperEngine::new(config, Box::new(MacOsPlatformAdapter::new()))
        .map_err(|error| error.to_string())?;
    engine.config.launch_at_startup = engine.is_launch_at_startup_enabled();

    let mut app = MacApp {
        engine,
        config_path: config_path.clone(),
        studio_open: false,
        selected_display: 0,
        preview: None,
        last_error: None,
        pushed_view: None,
        pushed_displays: None,
        pushed_reduce_motion: None,
        user_paused: HashSet::new(),
        lock_screen: LockScreenSync::new(lock_screen_directory(&config_path)),
        quit: false,
    };
    app.push_reduce_motion();

    if let Some(path) = initial_video {
        let display_id =
            primary_display_id(&app.engine).ok_or_else(|| "No active display found".to_string())?;
        app.engine.apply_wallpaper(&display_id, path)?;
        app.engine
            .config
            .save_to_file(&app.config_path)
            .map_err(|error| format!("Could not save wallpaper settings: {error}"))?;
    } else {
        app.engine.restore_saved_wallpapers();
    }

    // SAFETY: Startup, event processing, and shutdown run on this process's main thread.
    unsafe { auralis_macos_begin_menu(u8::from(app.engine.config.launch_at_startup)) };
    if let Some(warning) = load_warning {
        show_native_message("Auralis recovered its settings", &warning);
    }
    if !start_minimized && !app.engine.has_active_wallpaper() {
        app.open_studio();
    }

    let mut tick_counter: u32 = 0;
    while !app.quit {
        pump_native_events();
        let mut changed = false;
        while let Some((code, value, text)) = poll_native_action() {
            if let Some(action) = StudioAction::decode(code, value, &text) {
                changed |= action.refreshes_studio();
                app.handle(action);
            }
            if app.quit {
                break;
            }
        }
        if app.quit {
            break;
        }
        if let Err(error) = app.engine.tick() {
            app.report("Wallpaper playback stopped", &error.to_string());
            changed = true;
        }
        tick_counter = tick_counter.wrapping_add(1);
        // Once a second (or right after an action) compare wallpapers with matched pictures.
        if changed || tick_counter.is_multiple_of(LOCK_SCREEN_SYNC_TICKS) {
            app.sync_lock_screen(false);
        }
        if app.studio_open {
            app.poll_preview_failure();
            if changed || tick_counter.is_multiple_of(STUDIO_REFRESH_TICKS) {
                app.push_studio_view();
            }
        }
        std::thread::sleep(TICK);
    }

    app.stop_preview();
    // Quit puts every original desktop picture back. A force-quit cannot; the originals stay
    // recorded in the configuration and are restored on the next Quit or when turned off.
    app.sync_lock_screen(true);
    // SAFETY: Closes the Studio, removes the status item and notifications on the main thread.
    unsafe { auralis_macos_end_menu() };
    app.engine
        .config
        .save_to_file(&app.config_path)
        .map_err(|error| format!("Could not save settings during shutdown: {error}"))?;
    app.engine.shutdown();
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
