//! Platform Abstractions & Desktop Layering Interface
//!
//! Provides typed separation between shared orchestration logic and native OS wallpaper layers.
//! Shared code does not depend on AppKit, Objective-C, or Windows-specific raw pointers.

use serde::{Deserialize, Serialize};

#[cfg(target_os = "windows")]
pub mod windows;

/// Information about a detected display monitor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayInfo {
    /// Unique identifier for the display.
    pub id: String,
    /// Human-readable display device name.
    pub name: String,
    /// Horizontal resolution in pixels.
    pub width: u32,
    /// Vertical resolution in pixels.
    pub height: u32,
    /// Whether this display is the primary monitor.
    pub is_primary: bool,
}

/// Errors occurring within platform-specific wallpaper layering or display management.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformError {
    /// Could not locate the desktop Progman or desktop window handle.
    DesktopWindowNotFound,
    /// Could not spawn or locate the WorkerW window behind desktop icons.
    WorkerWCreationFailed(String),
    /// Failed to attach wallpaper window behind desktop icons.
    AttachmentFailed(String),
    /// Failed to enumerate active system displays.
    DisplayEnumerationFailed(String),
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
            Self::AttachmentFailed(msg) => {
                write!(f, "Failed to attach window behind desktop icons: {msg}")
            }
            Self::DisplayEnumerationFailed(msg) => write!(f, "Display enumeration failed: {msg}"),
            Self::UnsupportedPlatform => write!(
                f,
                "Current platform does not support desktop wallpaper layering"
            ),
        }
    }
}

impl std::error::Error for PlatformError {}

/// Abstract platform adapter interface for wallpaper window lifecycle and display management.
pub trait PlatformAdapter: Send + Sync {
    /// Enumerate all currently connected and active displays.
    fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, PlatformError>;

    /// Attach a native window handle behind desktop icons for the specified display.
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
            width: 1920,
            height: 1080,
            is_primary: true,
        }])
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
            id: "DISPLAY1".to_string(),
            name: "Generic PnP Monitor".to_string(),
            width: 2560,
            height: 1440,
            is_primary: true,
        };

        let json = serde_json::to_string(&display).unwrap();
        let deserialized: DisplayInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(display, deserialized);
    }

    #[test]
    fn test_create_platform_adapter() {
        let adapter = create_platform_adapter();
        let displays = adapter.enumerate_displays();
        assert!(displays.is_ok());
    }
}
