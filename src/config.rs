//! Auralis Configuration & Media Validation
//!
//! Handles validated, atomic configuration persistence adhering to AGENTS.md data safety principles.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Allowed video media extensions for desktop wallpaper playback.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "webm", "m4v"];

/// Errors that can occur during media validation or configuration operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// File does not exist on disk.
    FileNotFound(PathBuf),
    /// Path points to a directory or non-regular file.
    NotAFile(PathBuf),
    /// File extension is not a supported video format.
    UnsupportedFormat(String),
    /// File is empty (0 bytes).
    EmptyFile(PathBuf),
    /// I/O error occurred during reading or writing.
    IoError(String),
    /// Serialization or deserialization failed.
    ParseError(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileNotFound(p) => write!(f, "Media file not found: {}", p.display()),
            Self::NotAFile(p) => write!(f, "Path is not a regular file: {}", p.display()),
            Self::UnsupportedFormat(ext) => write!(f, "Unsupported video format: .{ext}"),
            Self::EmptyFile(p) => write!(f, "Media file is empty (0 bytes): {}", p.display()),
            Self::IoError(msg) => write!(f, "I/O error: {msg}"),
            Self::ParseError(msg) => write!(f, "Configuration parsing error: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Per-display wallpaper playback configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayWallpaperConfig {
    /// Absolute path to the local video file.
    pub media_path: PathBuf,
    /// Whether playback should seamlessly loop.
    pub loop_playback: bool,
    /// Mute audio.
    pub muted: bool,
    /// Playback volume (0 to 100).
    pub volume: u8,
}

impl DisplayWallpaperConfig {
    /// Creates a validated wallpaper configuration.
    pub fn new(media_path: PathBuf) -> Result<Self, ConfigError> {
        validate_media_file(&media_path)?;
        Ok(Self {
            media_path,
            loop_playback: true,
            muted: true,
            volume: 0,
        })
    }
}

/// Root application configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// Default wallpaper configuration applied to unassigned displays.
    pub default_wallpaper: Option<DisplayWallpaperConfig>,
    /// Display-specific wallpaper configurations mapped by display identifier.
    pub per_display: HashMap<String, DisplayWallpaperConfig>,
    /// Automatically pause playback when full-screen applications are focused.
    pub pause_on_fullscreen: bool,
    /// Automatically pause playback on battery power (laptops).
    pub pause_on_battery: bool,
    /// Automatically launch Auralis at user login.
    pub launch_at_startup: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_wallpaper: None,
            per_display: HashMap::new(),
            pause_on_fullscreen: true,
            pause_on_battery: true,
            launch_at_startup: false,
        }
    }
}

impl AppConfig {
    /// Load configuration from a JSON file. If the file does not exist, returns default configuration.
    pub fn load_from_file(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path).map_err(|e| ConfigError::IoError(e.to_string()))?;
        serde_json::from_str(&content).map_err(|e| ConfigError::ParseError(e.to_string()))
    }

    /// Atomically save configuration to disk via a temporary file to avoid corrupting user settings.
    pub fn save_to_file(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| ConfigError::IoError(e.to_string()))?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;

        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, json).map_err(|e| ConfigError::IoError(e.to_string()))?;
        fs::rename(&temp_path, path).map_err(|e| ConfigError::IoError(e.to_string()))?;

        Ok(())
    }
}

/// Validates that a media file exists, is a regular file, has a valid extension, and is non-empty.
pub fn validate_media_file(path: &Path) -> Result<(), ConfigError> {
    if !path.exists() {
        return Err(ConfigError::FileNotFound(path.to_path_buf()));
    }

    let metadata = fs::metadata(path).map_err(|e| ConfigError::IoError(e.to_string()))?;
    if !metadata.is_file() {
        return Err(ConfigError::NotAFile(path.to_path_buf()));
    }

    if metadata.len() == 0 {
        return Err(ConfigError::EmptyFile(path.to_path_buf()));
    }

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(ConfigError::UnsupportedFormat(ext));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert!(config.pause_on_fullscreen);
        assert!(config.pause_on_battery);
        assert!(!config.launch_at_startup);
        assert!(config.default_wallpaper.is_none());
        assert!(config.per_display.is_empty());
    }

    #[test]
    fn test_atomic_save_and_load_roundtrip() {
        let temp_dir = std::env::temp_dir().join("auralis_test_config");
        let config_file = temp_dir.join("test_config.json");

        let config = AppConfig {
            launch_at_startup: true,
            pause_on_battery: false,
            ..Default::default()
        };

        assert!(config.save_to_file(&config_file).is_ok());
        assert!(config_file.exists());

        let loaded = AppConfig::load_from_file(&config_file);
        assert!(loaded.is_ok());
        let loaded = loaded.unwrap();
        assert!(loaded.launch_at_startup);
        assert!(!loaded.pause_on_battery);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_load_nonexistent_returns_default() {
        let non_existent = Path::new("non_existent_config_12345.json");
        let result = AppConfig::load_from_file(non_existent);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), AppConfig::default());
    }

    #[test]
    fn test_media_validation() {
        let temp_dir = std::env::temp_dir().join("auralis_test_media");
        let _ = fs::create_dir_all(&temp_dir);

        // Non-existent file
        let missing = temp_dir.join("missing.mp4");
        assert!(matches!(
            validate_media_file(&missing),
            Err(ConfigError::FileNotFound(_))
        ));

        // Directory instead of file
        assert!(matches!(
            validate_media_file(&temp_dir),
            Err(ConfigError::NotAFile(_))
        ));

        // Empty file
        let empty = temp_dir.join("empty.mp4");
        let _ = fs::File::create(&empty);
        assert!(matches!(
            validate_media_file(&empty),
            Err(ConfigError::EmptyFile(_))
        ));

        // Unsupported extension
        let unsupported = temp_dir.join("image.png");
        let mut f = fs::File::create(&unsupported).unwrap();
        f.write_all(b"fake image data").unwrap();
        assert!(matches!(
            validate_media_file(&unsupported),
            Err(ConfigError::UnsupportedFormat(_))
        ));

        // Valid mock video file
        let valid = temp_dir.join("sample.mp4");
        let mut f2 = fs::File::create(&valid).unwrap();
        f2.write_all(b"fake video data content").unwrap();
        assert!(validate_media_file(&valid).is_ok());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
