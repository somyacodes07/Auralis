//! Media Playback State Machine & Controller
//!
//! Provides resilient playback orchestration, volume/mute management, and error preservation
//! adhering to AGENTS.md principle 9 (preserve working wallpaper when replacement fails).

use crate::config::{validate_media_file, ConfigError};
use std::path::PathBuf;

/// Status of the video playback pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackState {
    /// No media is currently assigned.
    Unloaded,
    /// Media is validating and preparing for playback.
    Loading { path: PathBuf },
    /// Active playback running.
    Playing { path: PathBuf, position_secs: f64 },
    /// Playback paused (e.g., full-screen app, battery power mode, or user request).
    Paused { path: PathBuf, position_secs: f64 },
    /// Playback stopped.
    Stopped { path: PathBuf },
    /// Recoverable playback error (holds failed path and error message).
    Failed { path: PathBuf, error: String },
}

/// Controller managing media playback lifecycle and audio parameters.
#[derive(Debug, Clone)]
pub struct MediaController {
    /// Current state of the player.
    pub state: PlaybackState,
    /// Last known successfully playing media file (used for resilient fallback).
    pub last_working_media: Option<PathBuf>,
    /// Whether playback should loop seamlessly.
    pub loop_playback: bool,
    /// Whether audio is muted.
    pub muted: bool,
    /// Playback volume (clamped 0 to 100).
    pub volume: u8,
}

impl Default for MediaController {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaController {
    /// Create a new media controller with safe initial defaults (muted, looping).
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: PlaybackState::Unloaded,
            last_working_media: None,
            loop_playback: true,
            muted: true,
            volume: 0,
        }
    }

    /// Load and validate a new media file.
    /// If validation fails, preserves the previous working media state instead of crashing.
    pub fn load_media(&mut self, path: PathBuf) -> Result<(), ConfigError> {
        if let Err(e) = validate_media_file(&path) {
            self.state = PlaybackState::Failed {
                path: path.clone(),
                error: e.to_string(),
            };
            return Err(e);
        }

        self.last_working_media = Some(path.clone());
        self.state = PlaybackState::Playing {
            path,
            position_secs: 0.0,
        };
        Ok(())
    }

    /// Pause the active wallpaper playback.
    pub fn pause(&mut self) {
        if let PlaybackState::Playing {
            path,
            position_secs,
        } = &self.state
        {
            self.state = PlaybackState::Paused {
                path: path.clone(),
                position_secs: *position_secs,
            };
        }
    }

    /// Resume wallpaper playback if paused.
    pub fn resume(&mut self) {
        if let PlaybackState::Paused {
            path,
            position_secs,
        } = &self.state
        {
            self.state = PlaybackState::Playing {
                path: path.clone(),
                position_secs: *position_secs,
            };
        }
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        match &self.state {
            PlaybackState::Playing { path, .. } | PlaybackState::Paused { path, .. } => {
                self.state = PlaybackState::Stopped { path: path.clone() };
            }
            _ => {}
        }
    }

    /// Set volume (automatically clamped between 0 and 100).
    pub fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
    }

    /// Set mute status.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Set looping preference.
    pub fn set_loop(&mut self, loop_playback: bool) {
        self.loop_playback = loop_playback;
    }

    /// Attempt to recover from a failure by reverting to the last known working media.
    pub fn recover_last_working(&mut self) -> bool {
        if let Some(media) = self.last_working_media.clone() {
            if media.exists() {
                self.state = PlaybackState::Playing {
                    path: media,
                    position_secs: 0.0,
                };
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn create_temp_video(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"mock video data").unwrap();
        path
    }

    #[test]
    fn test_initial_state() {
        let player = MediaController::new();
        assert_eq!(player.state, PlaybackState::Unloaded);
        assert!(player.muted);
        assert!(player.loop_playback);
        assert_eq!(player.volume, 0);
    }

    #[test]
    fn test_load_valid_media_and_lifecycle() {
        let media_path = create_temp_video("test_video_1.mp4");

        let mut player = MediaController::new();
        assert!(player.load_media(media_path.clone()).is_ok());

        assert!(matches!(player.state, PlaybackState::Playing { .. }));
        assert_eq!(player.last_working_media, Some(media_path.clone()));

        player.pause();
        assert!(matches!(player.state, PlaybackState::Paused { .. }));

        player.resume();
        assert!(matches!(player.state, PlaybackState::Playing { .. }));

        player.stop();
        assert!(matches!(player.state, PlaybackState::Stopped { .. }));

        let _ = std::fs::remove_file(media_path);
    }

    #[test]
    fn test_resilient_error_recovery() {
        let good_media = create_temp_video("good_video.mp4");
        let mut player = MediaController::new();

        assert!(player.load_media(good_media.clone()).is_ok());
        assert_eq!(player.last_working_media, Some(good_media.clone()));

        // Now attempt to load a missing video
        let bad_media = PathBuf::from("does_not_exist_9999.mp4");
        let load_err = player.load_media(bad_media);
        assert!(load_err.is_err());
        assert!(matches!(player.state, PlaybackState::Failed { .. }));

        // Recover last working media
        assert!(player.recover_last_working());
        assert!(matches!(player.state, PlaybackState::Playing { .. }));

        let _ = std::fs::remove_file(good_media);
    }

    #[test]
    fn test_volume_clamping() {
        let mut player = MediaController::new();
        player.set_volume(50);
        assert_eq!(player.volume, 50);

        player.set_volume(255);
        assert_eq!(player.volume, 100);
    }
}
