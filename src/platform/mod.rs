//! Platform Abstractions & Desktop Layering Interface
//!
//! Provides typed separation between shared orchestration logic and native OS wallpaper layers.
//! Shared code does not depend on AppKit, Objective-C, or Windows-specific raw pointers.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "windows")]
pub mod windows_gui;
#[cfg(target_os = "windows")]
pub mod windows_player;

/// Information about a detected display monitor, including virtual desktop coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayInfo {
    /// Unique identifier for the display.
    pub id: String,
    /// Human-readable display device name.
    pub name: String,
    /// Left coordinate in virtual screen space (can be negative on secondary displays).
    pub left: i32,
    /// Top coordinate in virtual screen space (can be negative on secondary displays).
    pub top: i32,
    /// Horizontal resolution in pixels.
    pub width: u32,
    /// Vertical resolution in pixels.
    pub height: u32,
    /// Whether this display is the primary monitor.
    pub is_primary: bool,
}

/// System power state used for battery-aware wallpaper playback policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerStatus {
    /// System is connected to AC mains power.
    OnAcPower,
    /// System is discharging on battery power.
    OnBattery {
        /// Remaining battery charge percentage (0..=100), if reported by the OS.
        percent: Option<u8>,
        /// Whether OS battery saver / low-power mode is active.
        battery_saver: bool,
    },
    /// Power state could not be determined.
    Unknown,
}

impl PowerStatus {
    /// Returns true if the system is currently running on battery power or battery saver.
    #[must_use]
    pub const fn should_conserve_power(&self) -> bool {
        matches!(self, Self::OnBattery { .. })
    }
}

/// Errors occurring within platform-specific wallpaper layering or display management.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformError {
    /// Could not locate the desktop Progman or desktop window handle.
    DesktopWindowNotFound,
    /// Could not spawn or locate the WorkerW window behind desktop icons.
    WorkerWCreationFailed(String),
    /// Failed to create or destroy a native wallpaper host window.
    WindowCreationFailed(String),
    /// Failed to attach wallpaper window behind desktop icons.
    AttachmentFailed(String),
    /// Failed to enumerate active system displays.
    DisplayEnumerationFailed(String),
    /// Failed to query system power state.
    PowerQueryFailed(String),
    /// Native hardware media decoder or renderer failed.
    MediaPlaybackFailed(String),
    /// Operating system is not supported for desktop wallpaper layering.
    UnsupportedPlatform,
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DesktopWindowNotFound => write!(f, "Desktop Progman window was not found"),
            Self::WorkerWCreationFailed(msg) => {
                write!(f, "Failed to create or locate WorkerW window: {msg}")
            }
            Self::WindowCreationFailed(msg) => {
                write!(f, "Wallpaper host window operation failed: {msg}")
            }
            Self::AttachmentFailed(msg) => {
                write!(f, "Failed to attach window behind desktop icons: {msg}")
            }
            Self::DisplayEnumerationFailed(msg) => write!(f, "Display enumeration failed: {msg}"),
            Self::PowerQueryFailed(msg) => write!(f, "System power status query failed: {msg}"),
            Self::MediaPlaybackFailed(msg) => write!(f, "Native video playback failed: {msg}"),
            Self::UnsupportedPlatform => {
                write!(
                    f,
                    "Current platform does not support desktop wallpaper layering"
                )
            }
        }
    }
}

impl std::error::Error for PlatformError {}

/// Abstract platform adapter interface for wallpaper window lifecycle, display management,
/// native video playback, and system state inspection.
pub trait PlatformAdapter: Send + Sync {
    /// Enumerate all currently connected and active displays.
    fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, PlatformError>;

    /// Query the current system power status (AC vs. battery).
    fn query_power_status(&self) -> Result<PowerStatus, PlatformError>;

    /// Check whether the specified display is currently occluded by a full-screen application.
    fn is_display_occluded_by_fullscreen(&self, display: &DisplayInfo) -> bool;

    /// Create a borderless native wallpaper host window sized to the display and attach it
    /// behind desktop icons, returning its opaque window handle.
    fn create_wallpaper_host_window(
        &mut self,
        display: &DisplayInfo,
    ) -> Result<isize, PlatformError>;

    /// Cleanly detach and destroy a previously created native wallpaper host window.
    fn destroy_wallpaper_host_window(&mut self, window_hwnd: isize) -> Result<(), PlatformError>;

    /// Attach an existing native window handle behind desktop icons for the specified display.
    fn attach_to_desktop(
        &mut self,
        display_id: &str,
        window_hwnd: isize,
    ) -> Result<(), PlatformError>;

    /// Detach a native window handle from behind desktop icons.
    fn detach_from_desktop(
        &mut self,
        display_id: &str,
        window_hwnd: isize,
    ) -> Result<(), PlatformError>;

    /// Start hardware-accelerated video playback inside the specified wallpaper host window.
    fn start_video_playback(
        &mut self,
        _window_hwnd: isize,
        _path: &Path,
        _muted: bool,
        _volume: u8,
    ) -> Result<(), PlatformError> {
        Ok(())
    }

    /// Pause native video playback on the specified wallpaper host window.
    fn pause_video_playback(&mut self, _window_hwnd: isize) -> Result<(), PlatformError> {
        Ok(())
    }

    /// Resume native video playback on the specified wallpaper host window.
    fn resume_video_playback(&mut self, _window_hwnd: isize) -> Result<(), PlatformError> {
        Ok(())
    }

    /// Stop and unload native video playback on the specified wallpaper host window.
    fn stop_video_playback(&mut self, _window_hwnd: isize) -> Result<(), PlatformError> {
        Ok(())
    }

    /// Update audio mute and volume (`0..=100`) on the specified wallpaper host window.
    fn set_video_audio(
        &mut self,
        _window_hwnd: isize,
        _muted: bool,
        _volume: u8,
    ) -> Result<(), PlatformError> {
        Ok(())
    }

    /// Periodic playback tick to handle seamless looping and video surface updates.
    fn tick_video_playback(
        &mut self,
        _window_hwnd: isize,
        _loop_playback: bool,
    ) -> Result<(), PlatformError> {
        Ok(())
    }
}

/// Factory function to create the appropriate platform adapter for the current host OS.
#[must_use]
pub fn create_platform_adapter() -> Box<dyn PlatformAdapter> {
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsPlatformAdapter::new())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Box::new(FallbackPlatformAdapter)
    }
}

/// Fallback adapter for non-Windows / unverified platforms.
#[cfg(not(target_os = "windows"))]
struct FallbackPlatformAdapter;

#[cfg(not(target_os = "windows"))]
impl PlatformAdapter for FallbackPlatformAdapter {
    fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        Ok(vec![DisplayInfo {
            id: "default".to_string(),
            name: "Default Display".to_string(),
            left: 0,
            top: 0,
            width: 1920,
            height: 1080,
            is_primary: true,
        }])
    }

    fn query_power_status(&self) -> Result<PowerStatus, PlatformError> {
        Ok(PowerStatus::OnAcPower)
    }

    fn is_display_occluded_by_fullscreen(&self, _display: &DisplayInfo) -> bool {
        false
    }

    fn create_wallpaper_host_window(
        &mut self,
        _display: &DisplayInfo,
    ) -> Result<isize, PlatformError> {
        Err(PlatformError::UnsupportedPlatform)
    }

    fn destroy_wallpaper_host_window(&mut self, _window_hwnd: isize) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedPlatform)
    }

    fn attach_to_desktop(
        &mut self,
        _display_id: &str,
        _window_hwnd: isize,
    ) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedPlatform)
    }

    fn detach_from_desktop(
        &mut self,
        _display_id: &str,
        _window_hwnd: isize,
    ) -> Result<(), PlatformError> {
        Err(PlatformError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_info_serialization() {
        let display = DisplayInfo {
            id: "DISPLAY_1".to_string(),
            name: "Generic PnP Monitor".to_string(),
            left: 0,
            top: 0,
            width: 2560,
            height: 1440,
            is_primary: true,
        };

        let json = serde_json::to_string(&display).unwrap();
        let deserialized: DisplayInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(display, deserialized);
    }

    #[test]
    fn test_power_status_conserve_check() {
        assert!(!PowerStatus::OnAcPower.should_conserve_power());
        assert!(!PowerStatus::Unknown.should_conserve_power());
        assert!(PowerStatus::OnBattery {
            percent: Some(80),
            battery_saver: false
        }
        .should_conserve_power());
    }

    #[test]
    fn test_create_platform_adapter() {
        let adapter = create_platform_adapter();
        let displays = adapter.enumerate_displays();
        assert!(displays.is_ok());
        let power = adapter.query_power_status();
        assert!(power.is_ok());
    }
}
