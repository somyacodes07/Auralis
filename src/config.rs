//! Auralis Configuration & Media Validation
//!
//! Handles validated, atomic configuration persistence, schema versioning, and
//! corrupt-file backup recovery adhering to `AGENTS.md` data safety principles.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Allowed video media extensions for desktop wallpaper playback.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "webm", "m4v"];

/// Current configuration schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Maximum number of recent wallpaper paths stored in `AppConfig::recent_wallpapers`.
pub const MAX_RECENT_WALLPAPERS: usize = 10;

const fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

const fn default_speed_percent() -> u16 {
    100
}

/// Video aspect-ratio scaling mode inside the wallpaper host window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VideoAspectMode {
    /// Stretch/fill the entire monitor surface (`MFP_ASPECT_RATIO_NONE`).
    #[default]
    Fill,
    /// Preserve original video aspect ratio with black letterbox/pillarbox bars (`MFP_ASPECT_RATIO_PRESERVE`).
    Fit,
}

impl VideoAspectMode {
    /// Human-readable label for UI display.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fill => "Fill",
            Self::Fit => "Fit",
        }
    }

    /// Cycle between `Fill` and `Fit`.
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Fill => Self::Fit,
            Self::Fit => Self::Fill,
        }
    }
}

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
    /// Playback speed percentage (`25..=200`, where `100` is `1.00x`).
    #[serde(default = "default_speed_percent")]
    pub speed_percent: u16,
    /// Aspect ratio / scaling mode (`Fill` vs `Fit`).
    #[serde(default)]
    pub aspect_mode: VideoAspectMode,
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
            speed_percent: 100,
            aspect_mode: VideoAspectMode::Fill,
        })
    }
}

/// Root application configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// Schema version for safe forward/backward configuration migrations.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Default wallpaper configuration applied to unassigned displays.
    pub default_wallpaper: Option<DisplayWallpaperConfig>,
    /// Display-specific wallpaper configurations mapped by stable display identifier.
    pub per_display: HashMap<String, DisplayWallpaperConfig>,
    /// Recently applied video wallpaper paths (newest first, up to `MAX_RECENT_WALLPAPERS`).
    #[serde(default)]
    pub recent_wallpapers: Vec<PathBuf>,
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
            schema_version: CURRENT_SCHEMA_VERSION,
            default_wallpaper: None,
            per_display: HashMap::new(),
            recent_wallpapers: Vec::new(),
            pause_on_fullscreen: true,
            pause_on_battery: true,
            launch_at_startup: false,
        }
    }
}

impl AppConfig {
    /// Record a validated wallpaper path at the front of `recent_wallpapers`, deduplicating
    /// existing entries and capping the list at `MAX_RECENT_WALLPAPERS`.
    pub fn record_recent_wallpaper(&mut self, path: PathBuf) {
        self.recent_wallpapers.retain(|p| p != &path);
        self.recent_wallpapers.insert(0, path);
        if self.recent_wallpapers.len() > MAX_RECENT_WALLPAPERS {
            self.recent_wallpapers.truncate(MAX_RECENT_WALLPAPERS);
        }
    }

    /// Return the platform-standard per-user configuration file path
    /// (e.g., `%APPDATA%\Auralis\auralis_config.json` on Windows, or
    /// `~/Library/Application Support/Auralis/auralis_config.json` on macOS).
    #[must_use]
    pub fn default_config_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                if !appdata.is_empty() {
                    return PathBuf::from(appdata)
                        .join("Auralis")
                        .join("auralis_config.json");
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(home) = std::env::var_os("HOME") {
                if !home.is_empty() {
                    return PathBuf::from(home)
                        .join("Library")
                        .join("Application Support")
                        .join("Auralis")
                        .join("auralis_config.json");
                }
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
                if !xdg.is_empty() {
                    return PathBuf::from(xdg)
                        .join("auralis")
                        .join("auralis_config.json");
                }
            }
            if let Some(home) = std::env::var_os("HOME") {
                if !home.is_empty() {
                    return PathBuf::from(home)
                        .join(".config")
                        .join("auralis")
                        .join("auralis_config.json");
                }
            }
        }
        PathBuf::from("auralis_config.json")
    }

    /// Normalize legacy enumeration-indexed display keys (e.g. `DISPLAY_1` -> `DISPLAY1`)
    /// and clamp playback speeds to safe bounds (`25..=200`).
    pub fn migrate_legacy_keys(&mut self) {
        let keys: Vec<String> = self.per_display.keys().cloned().collect();
        for key in keys {
            if let Some(suffix) = key.strip_prefix("DISPLAY_") {
                if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
                    let normalized = format!("DISPLAY{suffix}");
                    if let Some(val) = self.per_display.remove(&key) {
                        self.per_display.entry(normalized).or_insert(val);
                    }
                }
            }
        }
        for cfg in self.per_display.values_mut() {
            if cfg.speed_percent == 0 {
                cfg.speed_percent = 100;
            } else {
                cfg.speed_percent = cfg.speed_percent.clamp(25, 200);
            }
        }
        if let Some(default_cfg) = &mut self.default_wallpaper {
            if default_cfg.speed_percent == 0 {
                default_cfg.speed_percent = 100;
            } else {
                default_cfg.speed_percent = default_cfg.speed_percent.clamp(25, 200);
            }
        }
        if self.schema_version < CURRENT_SCHEMA_VERSION {
            self.schema_version = CURRENT_SCHEMA_VERSION;
        }
    }

    /// Load configuration from a JSON file. If the file does not exist, returns default configuration.
    pub fn load_from_file(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path).map_err(|e| ConfigError::IoError(e.to_string()))?;
        let mut cfg: Self =
            serde_json::from_str(&content).map_err(|e| ConfigError::ParseError(e.to_string()))?;
        cfg.migrate_legacy_keys();
        Ok(cfg)
    }

    /// Load configuration from `path`, or if the file is corrupt, back it up to
    /// `<path>.corrupt.bak` before returning safe defaults and a diagnostic warning message.
    #[must_use]
    pub fn load_or_recover(path: &Path) -> (Self, Option<String>) {
        match Self::load_from_file(path) {
            Ok(cfg) => (cfg, None),
            Err(err) => {
                let backup_path = path.with_extension("json.corrupt.bak");
                let backup_note = if path.exists() {
                    match fs::copy(path, &backup_path) {
                        Ok(_) => format!(" (backed up corrupt file to {})", backup_path.display()),
                        Err(copy_err) => format!(" (backup failed: {copy_err})"),
                    }
                } else {
                    String::new()
                };
                let warning = format!(
                    "Configuration at '{}' could not be loaded: {err}{backup_note}. Using safe defaults.",
                    path.display()
                );
                (Self::default(), Some(warning))
            }
        }
    }

    /// Atomically save configuration to disk via a temporary file to avoid corrupting user settings.
    pub fn save_to_file(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| ConfigError::IoError(e.to_string()))?;
            }
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
        assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
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
        assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(loaded.launch_at_startup);
        assert!(!loaded.pause_on_battery);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_corrupt_config_backup_and_recovery() {
        let temp_dir = std::env::temp_dir().join("auralis_test_corrupt_config");
        let _ = fs::create_dir_all(&temp_dir);
        let config_file = temp_dir.join("auralis_config.json");
        fs::write(&config_file, b"{ invalid json payload ").unwrap();

        let (recovered, warning) = AppConfig::load_or_recover(&config_file);
        assert_eq!(recovered, AppConfig::default());
        assert!(warning.is_some());

        let backup_file = config_file.with_extension("json.corrupt.bak");
        assert!(backup_file.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_legacy_display_id_migration() {
        let temp_dir = std::env::temp_dir().join("auralis_test_migration");
        let _ = fs::create_dir_all(&temp_dir);
        let config_file = temp_dir.join("legacy_config.json");

        let raw_json = r#"{
            "default_wallpaper": null,
            "per_display": {
                "DISPLAY_1": {
                    "media_path": "sample.mp4",
                    "loop_playback": true,
                    "muted": true,
                    "volume": 0
                }
            },
            "pause_on_fullscreen": true,
            "pause_on_battery": true,
            "launch_at_startup": false
        }"#;
        fs::write(&config_file, raw_json).unwrap();

        let loaded = AppConfig::load_from_file(&config_file).unwrap();
        assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(loaded.per_display.contains_key("DISPLAY1"));
        assert!(!loaded.per_display.contains_key("DISPLAY_1"));

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

    #[test]
    fn test_recent_wallpapers_and_aspect_mode() {
        let mut config = AppConfig::default();
        for i in 0..15 {
            config.record_recent_wallpaper(PathBuf::from(format!("video_{i}.mp4")));
        }
        assert_eq!(config.recent_wallpapers.len(), MAX_RECENT_WALLPAPERS);
        assert_eq!(config.recent_wallpapers[0], PathBuf::from("video_14.mp4"));

        // Recording an existing path moves it to the front without duplicating
        config.record_recent_wallpaper(PathBuf::from("video_10.mp4"));
        assert_eq!(config.recent_wallpapers.len(), MAX_RECENT_WALLPAPERS);
        assert_eq!(config.recent_wallpapers[0], PathBuf::from("video_10.mp4"));

        assert_eq!(VideoAspectMode::Fill.toggle(), VideoAspectMode::Fit);
        assert_eq!(VideoAspectMode::Fit.toggle(), VideoAspectMode::Fill);
    }
}
