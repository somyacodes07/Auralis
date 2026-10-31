//! Wallpaper Engine Orchestrator
//!
//! Coordinates multi-display state, media playback controllers, power/fullscreen policies,
//! and native platform adapters without coupling to OS-specific types.

use crate::config::{AppConfig, DisplayWallpaperConfig, VideoAspectMode};
use crate::media::MediaController;
use crate::platform::{
    DisplayInfo, PlatformAdapter, PlatformError, PowerStatus, ProcessResourceMetrics,
    VideoPlaybackTelemetry,
};
use crate::PlatformEvent;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Core orchestrator managing displays, native host windows, and media controllers.
pub struct WallpaperEngine {
    /// Active application configuration.
    pub config: AppConfig,
    /// Platform-specific desktop layering and hardware status adapter.
    adapter: Box<dyn PlatformAdapter>,
    /// Currently connected displays.
    displays: Vec<DisplayInfo>,
    /// Per-display media playback controllers keyed by display ID.
    controllers: HashMap<String, MediaController>,
    /// Active native wallpaper host window handles keyed by display ID.
    host_windows: HashMap<String, isize>,
    /// Displays explicitly paused by the user (preventing automatic policy resume).
    user_paused: HashSet<String>,
    /// Whether the system is currently in OS sleep state.
    system_sleeping: bool,
}

impl WallpaperEngine {
    /// Initialize the wallpaper engine and discover active displays.
    pub fn new(
        config: AppConfig,
        adapter: Box<dyn PlatformAdapter>,
    ) -> Result<Self, PlatformError> {
        let mut engine = Self {
            config,
            adapter,
            displays: Vec::new(),
            controllers: HashMap::new(),
            host_windows: HashMap::new(),
            user_paused: HashSet::new(),
            system_sleeping: false,
        };
        engine.refresh_displays()?;
        Ok(engine)
    }

    /// Return the slice of currently connected displays.
    #[must_use]
    pub fn displays(&self) -> &[DisplayInfo] {
        &self.displays
    }

    /// Return the playback controller for a specific display.
    #[must_use]
    pub fn controller(&self, display_id: &str) -> Option<&MediaController> {
        self.controllers.get(display_id)
    }

    /// Return the native host window handle for a specific display, if attached.
    #[must_use]
    pub fn host_window(&self, display_id: &str) -> Option<isize> {
        self.host_windows.get(display_id).copied()
    }

    /// Re-enumerate connected displays, cleaning up removed displays, resizing changed displays,
    /// and initializing newly added displays.
    pub fn refresh_displays(&mut self) -> Result<&[DisplayInfo], PlatformError> {
        let detected = self.adapter.enumerate_displays()?;
        let active_ids: HashSet<String> = detected.iter().map(|d| d.id.clone()).collect();
        let previous_map: HashMap<String, DisplayInfo> = self
            .displays
            .iter()
            .map(|d| (d.id.clone(), d.clone()))
            .collect();

        // Clean up host windows and controllers for disconnected displays
        let removed_ids: Vec<String> = self
            .controllers
            .keys()
            .filter(|id| !active_ids.contains(*id))
            .cloned()
            .collect();

        for id in removed_ids {
            if let Some(mut controller) = self.controllers.remove(&id) {
                controller.stop();
            }
            if let Some(hwnd) = self.host_windows.remove(&id) {
                let _ = self.adapter.destroy_wallpaper_host_window(hwnd);
            }
            self.user_paused.remove(&id);
        }

        // Initialize controllers for newly connected displays and resize existing host windows
        // if a monitor's resolution or virtual desktop coordinates changed.
        for display in &detected {
            self.controllers.entry(display.id.clone()).or_default();
            if let (Some(&hwnd), Some(prev)) = (
                self.host_windows.get(&display.id),
                previous_map.get(&display.id),
            ) {
                if prev.left != display.left
                    || prev.top != display.top
                    || prev.width != display.width
                    || prev.height != display.height
                {
                    let _ = self.adapter.resize_wallpaper_host_window(hwnd, display);
                }
            }
        }

        self.displays = detected;
        Ok(&self.displays)
    }

    /// Apply a local video wallpaper to the specified display and start native hardware playback.
    ///
    /// Preserves the previous working wallpaper if validation, window creation, or native media
    /// loading fails (`AGENTS.md` Principle 9).
    pub fn apply_wallpaper(&mut self, display_id: &str, media_path: PathBuf) -> Result<(), String> {
        let display = self
            .displays
            .iter()
            .find(|d| d.id == display_id)
            .cloned()
            .ok_or_else(|| format!("Display '{display_id}' not found"))?;

        // Validate and load into media controller first
        let controller = self.controllers.entry(display.id.clone()).or_default();

        if let Err(err) = controller.load_media(media_path.clone()) {
            // Preserve previous working wallpaper on replacement failure (AGENTS.md Principle 9)
            controller.recover_last_working();
            return Err(err.to_string());
        }

        let muted = controller.muted;
        let volume = controller.volume;
        let loop_playback = controller.loop_playback;
        let speed_percent = controller.speed_percent;
        let aspect_mode = controller.aspect_mode;

        // Ensure native host window is created and layered behind desktop icons
        let hwnd = if let Some(&existing_hwnd) = self.host_windows.get(display_id) {
            existing_hwnd
        } else {
            match self.adapter.create_wallpaper_host_window(&display) {
                Ok(new_hwnd) => {
                    self.host_windows.insert(display.id.clone(), new_hwnd);
                    new_hwnd
                }
                Err(err) => {
                    if let Some(ctrl) = self.controllers.get_mut(&display.id) {
                        ctrl.recover_last_working();
                    }
                    return Err(err.to_string());
                }
            }
        };

        // Start hardware-accelerated video playback inside the host window
        if let Err(err) = self
            .adapter
            .start_video_playback(hwnd, &media_path, muted, volume)
        {
            if let Some(ctrl) = self.controllers.get_mut(&display.id) {
                ctrl.recover_last_working();
            }
            return Err(err.to_string());
        }

        let _ = self.adapter.set_video_aspect_mode(hwnd, aspect_mode);
        let _ = self.adapter.set_video_playback_rate(hwnd, speed_percent);
        self.adapter.trim_working_set();

        self.user_paused.remove(display_id);

        // Record validated configuration and recent wallpapers library
        if let Ok(mut display_cfg) = DisplayWallpaperConfig::new(media_path.clone()) {
            display_cfg.muted = muted;
            display_cfg.volume = volume;
            display_cfg.loop_playback = loop_playback;
            display_cfg.speed_percent = speed_percent;
            display_cfg.aspect_mode = aspect_mode;
            self.config
                .per_display
                .insert(display.id.clone(), display_cfg.clone());
            if display.is_primary {
                self.config.default_wallpaper = Some(display_cfg);
            }
            self.config.record_recent_wallpaper(media_path);
        }

        Ok(())
    }

    /// Cycle to the next valid wallpaper in `self.config.recent_wallpapers` for `display_id`.
    pub fn cycle_next_recent_wallpaper(&mut self, display_id: &str) -> Result<PathBuf, String> {
        let valid_recents: Vec<PathBuf> = self
            .config
            .recent_wallpapers
            .iter()
            .filter(|p| p.exists())
            .cloned()
            .collect();

        if valid_recents.is_empty() {
            return Err("No recent wallpapers available in history.".to_string());
        }

        let current_path = self
            .config
            .per_display
            .get(display_id)
            .or(self.config.default_wallpaper.as_ref())
            .map(|c| c.media_path.clone());

        let next_path = if valid_recents.len() == 1 {
            valid_recents[0].clone()
        } else if let Some(cur) = current_path {
            // Pick the first entry different from `cur` (and rotate `cur` to the back of `recent_wallpapers`)
            let candidate = valid_recents
                .iter()
                .find(|p| **p != cur)
                .cloned()
                .unwrap_or_else(|| valid_recents[0].clone());
            self.config.recent_wallpapers.retain(|p| p != &cur);
            self.config.recent_wallpapers.push(cur);
            candidate
        } else {
            valid_recents[0].clone()
        };

        self.apply_wallpaper(display_id, next_path.clone())?;
        Ok(next_path)
    }

    /// Restore any previously configured wallpapers from `self.config`.
    pub fn restore_saved_wallpapers(&mut self) {
        let display_ids: Vec<(String, bool)> = self
            .displays
            .iter()
            .map(|d| (d.id.clone(), d.is_primary))
            .collect();

        for (id, is_primary) in display_ids {
            let saved_cfg = self.config.per_display.get(&id).cloned().or_else(|| {
                if is_primary {
                    self.config.default_wallpaper.clone()
                } else {
                    None
                }
            });

            if let Some(cfg) = saved_cfg {
                if let Some(ctrl) = self.controllers.get_mut(&id) {
                    ctrl.set_muted(cfg.muted);
                    ctrl.set_volume(cfg.volume);
                    ctrl.set_loop(cfg.loop_playback);
                    ctrl.set_speed_percent(cfg.speed_percent);
                    ctrl.set_aspect_mode(cfg.aspect_mode);
                }
                let _ = self.apply_wallpaper(&id, cfg.media_path);
            }
        }
    }

    /// Explicitly pause wallpaper playback on the specified display.
    pub fn pause_display(&mut self, display_id: &str) -> Result<(), PlatformError> {
        self.user_paused.insert(display_id.to_string());
        if let Some(controller) = self.controllers.get_mut(display_id) {
            controller.pause();
        }
        if let Some(&hwnd) = self.host_windows.get(display_id) {
            self.adapter.pause_video_playback(hwnd)?;
        }
        Ok(())
    }

    /// Explicitly resume wallpaper playback on the specified display.
    pub fn resume_display(&mut self, display_id: &str) -> Result<(), PlatformError> {
        self.user_paused.remove(display_id);
        if let Some(controller) = self.controllers.get_mut(display_id) {
            controller.resume();
        }
        if let Some(&hwnd) = self.host_windows.get(display_id) {
            self.adapter.resume_video_playback(hwnd)?;
        }
        Ok(())
    }

    /// Stop wallpaper playback and clear the host window on the specified display.
    pub fn stop_display(&mut self, display_id: &str) -> Result<(), PlatformError> {
        self.user_paused.remove(display_id);
        if let Some(controller) = self.controllers.get_mut(display_id) {
            controller.stop();
        }
        if let Some(hwnd) = self.host_windows.remove(display_id) {
            self.adapter.destroy_wallpaper_host_window(hwnd)?;
        }
        self.adapter.trim_working_set();
        Ok(())
    }

    /// Configure audio mute and volume (`0..=100`) for a display.
    pub fn set_display_audio(
        &mut self,
        display_id: &str,
        muted: bool,
        volume: u8,
    ) -> Result<(), PlatformError> {
        if let Some(controller) = self.controllers.get_mut(display_id) {
            controller.set_muted(muted);
            controller.set_volume(volume);
        }
        if let Some(cfg) = self.config.per_display.get_mut(display_id) {
            cfg.muted = muted;
            cfg.volume = volume.min(100);
        }
        if let Some(default_cfg) = &mut self.config.default_wallpaper {
            default_cfg.muted = muted;
            default_cfg.volume = volume.min(100);
        }
        if let Some(&hwnd) = self.host_windows.get(display_id) {
            self.adapter.set_video_audio(hwnd, muted, volume.min(100))?;
        }
        Ok(())
    }

    /// Configure seamless looping for a display.
    pub fn set_display_loop(&mut self, display_id: &str, loop_playback: bool) {
        if let Some(controller) = self.controllers.get_mut(display_id) {
            controller.set_loop(loop_playback);
        }
        if let Some(cfg) = self.config.per_display.get_mut(display_id) {
            cfg.loop_playback = loop_playback;
        }
        if let Some(default_cfg) = &mut self.config.default_wallpaper {
            default_cfg.loop_playback = loop_playback;
        }
    }

    /// Configure video playback speed percentage (`25..=200`, where `100` is `1.00x`) for a display.
    pub fn set_display_playback_speed(
        &mut self,
        display_id: &str,
        speed_percent: u16,
    ) -> Result<(), PlatformError> {
        let clamped = speed_percent.clamp(25, 200);
        if let Some(controller) = self.controllers.get_mut(display_id) {
            controller.set_speed_percent(clamped);
        }
        if let Some(cfg) = self.config.per_display.get_mut(display_id) {
            cfg.speed_percent = clamped;
        }
        if let Some(default_cfg) = &mut self.config.default_wallpaper {
            default_cfg.speed_percent = clamped;
        }
        if let Some(&hwnd) = self.host_windows.get(display_id) {
            self.adapter.set_video_playback_rate(hwnd, clamped)?;
        }
        Ok(())
    }

    /// Configure video aspect-ratio scaling mode (`Fill` vs `Fit`) for a display.
    pub fn set_display_aspect_mode(
        &mut self,
        display_id: &str,
        aspect_mode: VideoAspectMode,
    ) -> Result<(), PlatformError> {
        if let Some(controller) = self.controllers.get_mut(display_id) {
            controller.set_aspect_mode(aspect_mode);
        }
        if let Some(cfg) = self.config.per_display.get_mut(display_id) {
            cfg.aspect_mode = aspect_mode;
        }
        if let Some(default_cfg) = &mut self.config.default_wallpaper {
            default_cfg.aspect_mode = aspect_mode;
        }
        if let Some(&hwnd) = self.host_windows.get(display_id) {
            self.adapter.set_video_aspect_mode(hwnd, aspect_mode)?;
        }
        Ok(())
    }

    /// Reclaim unused OS staging pages from the process working set.
    pub fn trim_working_set(&self) {
        self.adapter.trim_working_set();
    }

    /// Evaluate power and full-screen occlusion policies across all active displays.
    pub fn evaluate_policy(&mut self) -> Result<(), PlatformError> {
        if self.system_sleeping {
            for (id, controller) in &mut self.controllers {
                let was_playing = matches!(
                    controller.state,
                    crate::media::PlaybackState::Playing { .. }
                );
                controller.pause();
                if was_playing {
                    if let Some(&hwnd) = self.host_windows.get(id) {
                        let _ = self.adapter.pause_video_playback(hwnd);
                    }
                }
            }
            return Ok(());
        }

        let power = self.adapter.query_power_status()?;
        if self.config.pause_on_battery && power.should_conserve_power() {
            for (id, controller) in &mut self.controllers {
                let was_playing = matches!(
                    controller.state,
                    crate::media::PlaybackState::Playing { .. }
                );
                controller.pause();
                if was_playing {
                    if let Some(&hwnd) = self.host_windows.get(id) {
                        let _ = self.adapter.pause_video_playback(hwnd);
                    }
                }
            }
            return Ok(());
        }

        for display in &self.displays {
            let occluded = self.config.pause_on_fullscreen
                && self.adapter.is_display_occluded_by_fullscreen(display);
            if let Some(controller) = self.controllers.get_mut(&display.id) {
                if occluded || self.user_paused.contains(&display.id) {
                    let was_playing = matches!(
                        controller.state,
                        crate::media::PlaybackState::Playing { .. }
                    );
                    controller.pause();
                    if was_playing {
                        if let Some(&hwnd) = self.host_windows.get(&display.id) {
                            let _ = self.adapter.pause_video_playback(hwnd);
                        }
                    }
                } else {
                    let was_paused =
                        matches!(controller.state, crate::media::PlaybackState::Paused { .. });
                    controller.resume();
                    if was_paused {
                        if let Some(&hwnd) = self.host_windows.get(&display.id) {
                            let _ = self.adapter.resume_video_playback(hwnd);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Periodic runtime tick that evaluates power/fullscreen policies and keeps active
    /// video streams seamlessly looping.
    pub fn tick(&mut self) -> Result<(), PlatformError> {
        self.evaluate_policy()?;

        for (id, controller) in &self.controllers {
            if matches!(
                controller.state,
                crate::media::PlaybackState::Playing { .. }
            ) {
                if let Some(&hwnd) = self.host_windows.get(id) {
                    let _ = self
                        .adapter
                        .tick_video_playback(hwnd, controller.loop_playback);
                }
            }
        }
        Ok(())
    }

    /// Handle a platform lifecycle or display event.
    pub fn handle_event(&mut self, event: PlatformEvent) -> Result<(), PlatformError> {
        match event {
            PlatformEvent::DisplayConfigurationChanged => {
                self.refresh_displays()?;
                // Restore wallpapers for any newly connected displays that do not yet have a host window
                let missing_displays: Vec<(String, bool)> = self
                    .displays
                    .iter()
                    .filter(|d| !self.host_windows.contains_key(&d.id))
                    .map(|d| (d.id.clone(), d.is_primary))
                    .collect();
                for (id, is_primary) in missing_displays {
                    let saved_cfg = self.config.per_display.get(&id).cloned().or_else(|| {
                        if is_primary {
                            self.config.default_wallpaper.clone()
                        } else {
                            None
                        }
                    });
                    if let Some(cfg) = saved_cfg {
                        if let Some(ctrl) = self.controllers.get_mut(&id) {
                            ctrl.set_muted(cfg.muted);
                            ctrl.set_volume(cfg.volume);
                            ctrl.set_loop(cfg.loop_playback);
                            ctrl.set_speed_percent(cfg.speed_percent);
                            ctrl.set_aspect_mode(cfg.aspect_mode);
                        }
                        let _ = self.apply_wallpaper(&id, cfg.media_path);
                    }
                }
                self.evaluate_policy()?;
            }
            PlatformEvent::SystemSleepEntered => {
                self.system_sleeping = true;
                for (id, controller) in &mut self.controllers {
                    controller.pause();
                    if let Some(&hwnd) = self.host_windows.get(id) {
                        let _ = self.adapter.pause_video_playback(hwnd);
                    }
                }
                self.adapter.trim_working_set();
            }
            PlatformEvent::SystemWoke => {
                self.system_sleeping = false;
                self.evaluate_policy()?;
            }
            PlatformEvent::PlatformError { display_id, .. } => {
                if let Some(controller) = self.controllers.get_mut(&display_id) {
                    controller.recover_last_working();
                }
            }
        }
        Ok(())
    }

    /// Query current power status from the underlying platform adapter.
    pub fn query_power_status(&self) -> Result<PowerStatus, PlatformError> {
        self.adapter.query_power_status()
    }

    /// Query live hardware video playback telemetry (resolution, position, duration) for a display.
    #[must_use]
    pub fn video_telemetry(&self, display_id: &str) -> Option<VideoPlaybackTelemetry> {
        let hwnd = self.host_windows.get(display_id).copied()?;
        self.adapter.query_video_telemetry(hwnd)
    }

    /// Query measured physical RAM working set and CPU time of the current Auralis process.
    #[must_use]
    pub fn process_metrics(&self) -> Option<ProcessResourceMetrics> {
        self.adapter.query_process_metrics()
    }

    /// Configure OS login startup registration and synchronize `self.config.launch_at_startup`.
    pub fn set_launch_at_startup(&mut self, enabled: bool) -> Result<(), PlatformError> {
        self.adapter.set_launch_at_startup(enabled)?;
        self.config.launch_at_startup = enabled;
        Ok(())
    }

    /// Query whether OS login startup registration is currently active.
    #[must_use]
    pub fn is_launch_at_startup_enabled(&self) -> bool {
        self.adapter.is_launch_at_startup_enabled()
    }

    /// Return true if at least one display has an active wallpaper host window.
    #[must_use]
    pub fn has_active_wallpaper(&self) -> bool {
        !self.host_windows.is_empty()
    }

    /// Cleanly stop all playback controllers and destroy all native wallpaper host windows.
    pub fn shutdown(&mut self) {
        for controller in self.controllers.values_mut() {
            controller.stop();
        }
        let handles: Vec<isize> = self.host_windows.drain().map(|(_, hwnd)| hwnd).collect();
        for hwnd in handles {
            let _ = self.adapter.destroy_wallpaper_host_window(hwnd);
        }
        self.adapter.trim_working_set();
    }
}

impl Drop for WallpaperEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::PlaybackState;
    use std::io::Write;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };

    struct MockPlatformAdapter {
        on_battery: Arc<AtomicBool>,
        occluded: Arc<AtomicBool>,
        created_windows: Arc<AtomicUsize>,
        destroyed_windows: Arc<AtomicUsize>,
    }

    impl PlatformAdapter for MockPlatformAdapter {
        fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
            Ok(vec![
                DisplayInfo {
                    id: "DISPLAY_1".to_string(),
                    name: "Primary Monitor".to_string(),
                    left: 0,
                    top: 0,
                    width: 1920,
                    height: 1080,
                    is_primary: true,
                },
                DisplayInfo {
                    id: "DISPLAY_2".to_string(),
                    name: "Secondary Monitor".to_string(),
                    left: 1920,
                    top: 0,
                    width: 1920,
                    height: 1080,
                    is_primary: false,
                },
            ])
        }

        fn query_power_status(&self) -> Result<PowerStatus, PlatformError> {
            if self.on_battery.load(Ordering::SeqCst) {
                Ok(PowerStatus::OnBattery {
                    percent: Some(65),
                    battery_saver: false,
                })
            } else {
                Ok(PowerStatus::OnAcPower)
            }
        }

        fn is_display_occluded_by_fullscreen(&self, display: &DisplayInfo) -> bool {
            display.id == "DISPLAY_1" && self.occluded.load(Ordering::SeqCst)
        }

        fn create_wallpaper_host_window(
            &mut self,
            _display: &DisplayInfo,
        ) -> Result<isize, PlatformError> {
            let id = self.created_windows.fetch_add(1, Ordering::SeqCst) + 100;
            Ok(id as isize)
        }

        fn destroy_wallpaper_host_window(
            &mut self,
            _window_hwnd: isize,
        ) -> Result<(), PlatformError> {
            self.destroyed_windows.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn attach_to_desktop(
            &mut self,
            _display_id: &str,
            _window_hwnd: isize,
        ) -> Result<(), PlatformError> {
            Ok(())
        }

        fn detach_from_desktop(
            &mut self,
            _display_id: &str,
            _window_hwnd: isize,
        ) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    fn create_temp_video(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"mock mp4 payload").unwrap();
        path
    }

    #[test]
    fn test_engine_apply_and_fallback_recovery() {
        let on_battery = Arc::new(AtomicBool::new(false));
        let occluded = Arc::new(AtomicBool::new(false));
        let created = Arc::new(AtomicUsize::new(0));
        let destroyed = Arc::new(AtomicUsize::new(0));

        let adapter = Box::new(MockPlatformAdapter {
            on_battery,
            occluded,
            created_windows: created.clone(),
            destroyed_windows: destroyed.clone(),
        });

        let mut engine = WallpaperEngine::new(AppConfig::default(), adapter).unwrap();
        assert_eq!(engine.displays().len(), 2);

        let video = create_temp_video("engine_test_valid.mp4");
        assert!(engine.apply_wallpaper("DISPLAY_1", video.clone()).is_ok());
        assert_eq!(created.load(Ordering::SeqCst), 1);
        assert!(matches!(
            engine.controller("DISPLAY_1").unwrap().state,
            PlaybackState::Playing { .. }
        ));

        // Attempt replacement with invalid file; verify previous working wallpaper is preserved
        let invalid = PathBuf::from("non_existent_replacement.mp4");
        assert!(engine.apply_wallpaper("DISPLAY_1", invalid).is_err());
        assert!(matches!(
            engine.controller("DISPLAY_1").unwrap().state,
            PlaybackState::Playing { .. }
        ));

        engine.shutdown();
        assert_eq!(destroyed.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_file(video);
    }

    #[test]
    fn test_engine_sleep_wake_and_battery_policies() {
        let on_battery = Arc::new(AtomicBool::new(false));
        let occluded = Arc::new(AtomicBool::new(false));
        let created = Arc::new(AtomicUsize::new(0));
        let destroyed = Arc::new(AtomicUsize::new(0));

        let adapter = Box::new(MockPlatformAdapter {
            on_battery: on_battery.clone(),
            occluded: occluded.clone(),
            created_windows: created,
            destroyed_windows: destroyed,
        });

        let mut engine = WallpaperEngine::new(AppConfig::default(), adapter).unwrap();
        let video = create_temp_video("engine_policy_test.mp4");
        engine.apply_wallpaper("DISPLAY_1", video.clone()).unwrap();
        engine.apply_wallpaper("DISPLAY_2", video.clone()).unwrap();

        // Enter sleep -> both displays pause
        engine
            .handle_event(PlatformEvent::SystemSleepEntered)
            .unwrap();
        assert!(matches!(
            engine.controller("DISPLAY_1").unwrap().state,
            PlaybackState::Paused { .. }
        ));
        assert!(matches!(
            engine.controller("DISPLAY_2").unwrap().state,
            PlaybackState::Paused { .. }
        ));

        // Wake up on AC power -> both resume
        engine.handle_event(PlatformEvent::SystemWoke).unwrap();
        assert!(matches!(
            engine.controller("DISPLAY_1").unwrap().state,
            PlaybackState::Playing { .. }
        ));

        // Occlude DISPLAY_1 with full-screen app -> DISPLAY_1 pauses, DISPLAY_2 keeps playing
        occluded.store(true, Ordering::SeqCst);
        engine.evaluate_policy().unwrap();
        assert!(matches!(
            engine.controller("DISPLAY_1").unwrap().state,
            PlaybackState::Paused { .. }
        ));
        assert!(matches!(
            engine.controller("DISPLAY_2").unwrap().state,
            PlaybackState::Playing { .. }
        ));

        // Switch to battery power -> all displays pause
        occluded.store(false, Ordering::SeqCst);
        on_battery.store(true, Ordering::SeqCst);
        engine.evaluate_policy().unwrap();
        assert!(matches!(
            engine.controller("DISPLAY_1").unwrap().state,
            PlaybackState::Paused { .. }
        ));
        assert!(matches!(
            engine.controller("DISPLAY_2").unwrap().state,
            PlaybackState::Paused { .. }
        ));

        let _ = std::fs::remove_file(video);
    }

    #[test]
    fn test_engine_user_pause_and_audio_controls() {
        let on_battery = Arc::new(AtomicBool::new(false));
        let occluded = Arc::new(AtomicBool::new(false));
        let created = Arc::new(AtomicUsize::new(0));
        let destroyed = Arc::new(AtomicUsize::new(0));

        let adapter = Box::new(MockPlatformAdapter {
            on_battery,
            occluded,
            created_windows: created,
            destroyed_windows: destroyed.clone(),
        });

        let mut engine = WallpaperEngine::new(AppConfig::default(), adapter).unwrap();
        let video = create_temp_video("engine_controls_test.mp4");
        engine.apply_wallpaper("DISPLAY_1", video.clone()).unwrap();
        assert!(engine.has_active_wallpaper());

        // User explicitly pauses DISPLAY_1; policy tick should NOT auto-resume it
        engine.pause_display("DISPLAY_1").unwrap();
        engine.tick().unwrap();
        assert!(matches!(
            engine.controller("DISPLAY_1").unwrap().state,
            PlaybackState::Paused { .. }
        ));

        // User explicitly resumes DISPLAY_1
        engine.resume_display("DISPLAY_1").unwrap();
        assert!(matches!(
            engine.controller("DISPLAY_1").unwrap().state,
            PlaybackState::Playing { .. }
        ));

        // Adjust audio and looping settings
        engine.set_display_audio("DISPLAY_1", false, 75).unwrap();
        engine.set_display_loop("DISPLAY_1", false);
        let ctrl = engine.controller("DISPLAY_1").unwrap();
        assert!(!ctrl.muted);
        assert_eq!(ctrl.volume, 75);
        assert!(!ctrl.loop_playback);

        // Stop display destroys host window
        engine.stop_display("DISPLAY_1").unwrap();
        assert!(!engine.has_active_wallpaper());
        assert_eq!(destroyed.load(Ordering::SeqCst), 1);

        let _ = std::fs::remove_file(video);
    }

    #[test]
    fn test_engine_speed_aspect_and_recent_cycling() {
        let on_battery = Arc::new(AtomicBool::new(false));
        let occluded = Arc::new(AtomicBool::new(false));
        let created = Arc::new(AtomicUsize::new(0));
        let destroyed = Arc::new(AtomicUsize::new(0));

        let adapter = Box::new(MockPlatformAdapter {
            on_battery,
            occluded,
            created_windows: created,
            destroyed_windows: destroyed,
        });

        let mut engine = WallpaperEngine::new(AppConfig::default(), adapter).unwrap();
        let v1 = create_temp_video("engine_cycle_1.mp4");
        let v2 = create_temp_video("engine_cycle_2.mp4");

        engine.apply_wallpaper("DISPLAY_1", v1.clone()).unwrap();
        engine.apply_wallpaper("DISPLAY_1", v2.clone()).unwrap();

        assert_eq!(engine.config.recent_wallpapers.len(), 2);
        assert_eq!(engine.config.recent_wallpapers[0], v2);

        // Cycle to next recent wallpaper -> switches back to v1
        let cycled = engine.cycle_next_recent_wallpaper("DISPLAY_1").unwrap();
        assert_eq!(cycled, v1);

        // Set playback speed and aspect mode
        engine.set_display_playback_speed("DISPLAY_1", 150).unwrap();
        engine
            .set_display_aspect_mode("DISPLAY_1", VideoAspectMode::Fit)
            .unwrap();
        let ctrl = engine.controller("DISPLAY_1").unwrap();
        assert_eq!(ctrl.speed_percent, 150);
        assert_eq!(ctrl.aspect_mode, VideoAspectMode::Fit);

        let _ = std::fs::remove_file(v1);
        let _ = std::fs::remove_file(v2);
    }
}
