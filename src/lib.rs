//! # Auralis Core Library
//!
//! Shared architecture, configuration, platform abstractions, and orchestration logic
//! for the Auralis live wallpaper engine.
//!
//! In accordance with `AGENTS.md`:
//! - Shared code does not depend directly on AppKit, Objective-C, or Windows-specific types.
//! - Platform adapters communicate via typed commands, events, and capabilities.
//! - No fake functionality or simulated playback.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod config;
pub mod engine;
pub mod media;
pub mod platform;

pub use config::{validate_media_file, AppConfig, ConfigError, DisplayWallpaperConfig};
pub use engine::WallpaperEngine;
pub use media::{MediaController, PlaybackState};
pub use platform::{
    create_platform_adapter, DisplayInfo, PlatformAdapter, PlatformError, PowerStatus,
};

/// Supported platform capabilities detected at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformCapabilities {
    /// Whether video wallpaper behind desktop icons is supported.
    pub wallpaper_layering: bool,
    /// Whether hardware-accelerated video decoding is available.
    pub hardware_decoding: bool,
    /// Whether multi-display independent wallpaper playback is supported.
    pub multi_display: bool,
    /// Operating system name.
    pub os_name: &'static str,
}

impl PlatformCapabilities {
    /// Detect current platform compile-time defaults.
    #[must_use]
    pub const fn detect() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self {
                wallpaper_layering: true,
                hardware_decoding: true,
                multi_display: true,
                os_name: "Windows",
            }
        }
        #[cfg(target_os = "macos")]
        {
            Self {
                wallpaper_layering: true,
                hardware_decoding: true,
                multi_display: true,
                os_name: "macOS",
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Self {
                wallpaper_layering: false,
                hardware_decoding: false,
                multi_display: false,
                os_name: "Unsupported",
            }
        }
    }
}

/// Typed platform commands dispatched from shared logic to platform adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformCommand {
    /// Attach the wallpaper window behind desktop icons for the specified display.
    AttachToDesktop { display_id: String },
    /// Detach the wallpaper window from the desktop.
    DetachFromDesktop { display_id: String },
    /// Pause playback (e.g., when full-screen apps or power-saving modes are active).
    PausePlayback { display_id: String },
    /// Resume playback.
    ResumePlayback { display_id: String },
}

/// Typed platform events received from native adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformEvent {
    /// Display configuration changed (resolution change, display added/removed).
    DisplayConfigurationChanged,
    /// System entered sleep or low-power state.
    SystemSleepEntered,
    /// System woke from sleep.
    SystemWoke,
    /// A native error occurred during playback or desktop layering.
    PlatformError { display_id: String, message: String },
}

/// Core wallpaper configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallpaperConfig {
    /// Path to the video media file.
    pub media_path: std::path::PathBuf,
    /// Whether playback should loop.
    pub loop_playback: bool,
    /// Mute audio.
    pub muted: bool,
    /// Playback volume (0 to 100).
    pub volume: u8,
}

impl WallpaperConfig {
    /// Create a new validated wallpaper configuration.
    ///
    /// # Errors
    /// Returns an error if the media path is empty.
    pub fn new(media_path: std::path::PathBuf) -> Result<Self, &'static str> {
        if media_path.as_os_str().is_empty() {
            return Err("Media path cannot be empty");
        }
        Ok(Self {
            media_path,
            loop_playback: true,
            muted: true,
            volume: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_version_not_empty() {
        assert!(!VERSION.is_empty());
        assert_eq!(VERSION, "0.1.0-beta.1");
    }

    #[test]
    fn test_platform_capabilities_detection() {
        let caps = PlatformCapabilities::detect();
        #[cfg(target_os = "windows")]
        {
            assert_eq!(caps.os_name, "Windows");
            assert!(caps.wallpaper_layering);
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(caps.os_name, "macOS");
            assert!(caps.wallpaper_layering);
        }
    }

    #[test]
    fn test_wallpaper_config_validation() {
        let valid = WallpaperConfig::new(PathBuf::from("test.mp4"));
        assert!(valid.is_ok());
        let config = valid.unwrap();
        assert!(config.loop_playback);
        assert!(config.muted);

        let invalid = WallpaperConfig::new(PathBuf::from(""));
        assert!(invalid.is_err());
    }
}
