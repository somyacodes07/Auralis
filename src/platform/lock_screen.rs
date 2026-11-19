//! "Match lock screen to wallpaper" (macOS, opt-in).
//!
//! macOS draws the lock screen in `loginwindow`, and third-party apps cannot play video there.
//! The closest honest option is to set a still frame of the wallpaper video as the display's
//! desktop picture, which the lock screen and Mission Control then show. This module owns the
//! decisions (when to record the original picture, when to replace it, when to restore it) and
//! is plain Rust, so it is unit-tested on every platform. The native calls (frame extraction,
//! `NSWorkspace` desktop pictures) sit behind [`DesktopPictureBackend`].
//!
//! Auralis only ever writes its own still files into the lock-screen directory; it never
//! modifies the user's videos or pictures. A force-quit leaves the still frame in place until
//! Auralis next restores it (the original stays recorded in the configuration).

use crate::config::{DesktopPicture, VideoAspectMode};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// `NSImageScaleProportionallyUpOrDown`.
pub const SCALE_PROPORTIONALLY_UP_OR_DOWN: i32 = 3;

/// Help text shown next to the setting.
pub const HELP_TEXT: &str =
    "Uses a still frame from your video; macOS does not allow moving wallpapers on the lock screen.";

/// Native operations needed to match the lock screen.
pub trait DesktopPictureBackend {
    /// The display's current desktop picture.
    fn current(&mut self, display_id: &str) -> Result<DesktopPicture, String>;
    /// Set the display's desktop picture.
    fn set(&mut self, display_id: &str, picture: &DesktopPicture) -> Result<(), String>;
    /// Write a representative still frame of `video`, sized for the display, to `output`.
    fn extract_still(
        &mut self,
        display_id: &str,
        video: &Path,
        output: &Path,
    ) -> Result<(), String>;
}

/// What a display's desktop picture is currently matched to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Matched {
    video: PathBuf,
    aspect: VideoAspectMode,
    /// The still Auralis wrote, or `None` when matching failed (not retried until the
    /// wallpaper or its scaling changes).
    still: Option<PathBuf>,
}

/// One display's wallpaper, as the engine reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallpaperTarget {
    /// Display id.
    pub display_id: String,
    /// Wallpaper video.
    pub video: PathBuf,
    /// Fill or Fit.
    pub aspect: VideoAspectMode,
}

/// Outcome of a [`LockScreenSync`] pass.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Whether `originals` changed and the configuration must be saved.
    pub config_changed: bool,
    /// User-facing errors, one per failed display.
    pub errors: Vec<String>,
}

/// Keeps each display's desktop picture in step with its wallpaper while the setting is on.
#[derive(Debug)]
pub struct LockScreenSync {
    directory: PathBuf,
    matched: HashMap<String, Matched>,
    counter: u64,
    /// Displays whose restore failed; skipped by [`Self::reconcile`] so the error is reported
    /// once, and retried only by an explicit [`Self::restore_all`].
    restore_failed: HashSet<String>,
}

/// The picture Auralis sets for a still: scaled to cover (Fill) or fit on black (Fit).
#[must_use]
pub fn still_picture(still: &Path, aspect: VideoAspectMode) -> DesktopPicture {
    let fit = matches!(aspect, VideoAspectMode::Fit);
    DesktopPicture {
        url: still.to_string_lossy().into_owned(),
        scaling: Some(SCALE_PROPORTIONALLY_UP_OR_DOWN),
        allow_clipping: Some(!fit),
        fill_rgba: Some([0, 0, 0, u16::MAX]),
    }
}

fn dedup(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids.dedup();
    ids
}

fn sanitize(display_id: &str) -> String {
    display_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

impl LockScreenSync {
    /// Create a sync that writes stills into `directory`
    /// (`~/Library/Application Support/Auralis/lock-screen/` on macOS).
    #[must_use]
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            matched: HashMap::new(),
            counter: 0,
            restore_failed: HashSet::new(),
        }
    }

    /// Whether a desktop picture URL points at a still Auralis wrote.
    #[must_use]
    pub fn is_auralis_still(url: &str) -> bool {
        url.contains("Auralis/lock-screen/")
    }

    /// Bring every display in line with `targets`: match displays with a wallpaper when
    /// `enabled`, restore the rest (all of them when disabled).
    pub fn reconcile(
        &mut self,
        enabled: bool,
        targets: &[WallpaperTarget],
        originals: &mut HashMap<String, DesktopPicture>,
        backend: &mut dyn DesktopPictureBackend,
    ) -> SyncReport {
        let mut report = SyncReport::default();
        let ids: Vec<String> = self
            .matched
            .keys()
            .chain(originals.keys())
            .filter(|id| !enabled || !targets.iter().any(|t| &t.display_id == *id))
            .filter(|id| !self.restore_failed.contains(*id))
            .cloned()
            .collect();
        let ids = dedup(ids);
        for display_id in ids {
            self.restore(&display_id, originals, backend, &mut report);
        }
        if enabled {
            for target in targets {
                self.match_display(target, originals, backend, &mut report);
            }
        }
        report
    }

    /// Restore every recorded original, retrying earlier failures (Quit).
    pub fn restore_all(
        &mut self,
        originals: &mut HashMap<String, DesktopPicture>,
        backend: &mut dyn DesktopPictureBackend,
    ) -> SyncReport {
        self.restore_failed.clear();
        let mut report = SyncReport::default();
        let ids: Vec<String> = self
            .matched
            .keys()
            .chain(originals.keys())
            .cloned()
            .collect();
        let ids = dedup(ids);
        for display_id in ids {
            self.restore(&display_id, originals, backend, &mut report);
        }
        report
    }

    fn match_display(
        &mut self,
        target: &WallpaperTarget,
        originals: &mut HashMap<String, DesktopPicture>,
        backend: &mut dyn DesktopPictureBackend,
        report: &mut SyncReport,
    ) {
        let id = &target.display_id;
        if let Some(matched) = self.matched.get(id) {
            if matched.video == target.video && matched.aspect == target.aspect {
                return;
            }
        }
        let previous_still = self.matched.get(id).and_then(|m| m.still.clone());
        let failed = Matched {
            video: target.video.clone(),
            aspect: target.aspect,
            still: previous_still.clone(),
        };

        if !originals.contains_key(id) {
            match backend.current(id) {
                // A still left behind by a force-quit is not the user's original; keep looking
                // for the real one in the saved configuration instead of recording it.
                Ok(picture) if Self::is_auralis_still(&picture.url) => {}
                Ok(picture) => {
                    originals.insert(id.clone(), picture);
                    report.config_changed = true;
                }
                Err(error) => {
                    report.errors.push(format!(
                        "Could not read the current desktop picture: {error}"
                    ));
                    self.matched.insert(id.clone(), failed);
                    return;
                }
            }
        }

        if let Err(error) = fs::create_dir_all(&self.directory) {
            report
                .errors
                .push(format!("Could not create the lock screen folder: {error}"));
            self.matched.insert(id.clone(), failed);
            return;
        }
        // A new file name each time: macOS caches desktop pictures by URL.
        self.counter = self.counter.wrapping_add(1);
        let still = self.directory.join(format!(
            "{}-{}-{}.jpg",
            sanitize(id),
            std::process::id(),
            self.counter
        ));
        if let Err(error) = backend.extract_still(id, &target.video, &still) {
            report
                .errors
                .push(format!("Could not capture a still frame: {error}"));
            self.matched.insert(id.clone(), failed);
            return;
        }
        if let Err(error) = backend.set(id, &still_picture(&still, target.aspect)) {
            let _ = fs::remove_file(&still);
            report
                .errors
                .push(format!("Could not set the lock screen picture: {error}"));
            self.matched.insert(id.clone(), failed);
            return;
        }
        if let Some(old) = previous_still {
            let _ = fs::remove_file(old);
        }
        self.restore_failed.remove(id);
        self.matched.insert(
            id.clone(),
            Matched {
                video: target.video.clone(),
                aspect: target.aspect,
                still: Some(still),
            },
        );
    }

    fn restore(
        &mut self,
        display_id: &str,
        originals: &mut HashMap<String, DesktopPicture>,
        backend: &mut dyn DesktopPictureBackend,
        report: &mut SyncReport,
    ) {
        let matched = self.matched.remove(display_id);
        if let Some(original) = originals.get(display_id) {
            if let Err(error) = backend.set(display_id, original) {
                // Keep the original recorded so Quit (or the next launch) can retry.
                self.restore_failed.insert(display_id.to_string());
                report.errors.push(format!(
                    "Could not restore the original desktop picture: {error}"
                ));
                return;
            }
            originals.remove(display_id);
            self.restore_failed.remove(display_id);
            report.config_changed = true;
        }
        if let Some(still) = matched.and_then(|m| m.still) {
            let _ = fs::remove_file(still);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        pictures: HashMap<String, DesktopPicture>,
        extracted: Vec<PathBuf>,
        fail_extract: bool,
        fail_set: bool,
    }

    impl DesktopPictureBackend for FakeBackend {
        fn current(&mut self, display_id: &str) -> Result<DesktopPicture, String> {
            self.pictures
                .get(display_id)
                .cloned()
                .ok_or_else(|| "no screen".to_string())
        }

        fn set(&mut self, display_id: &str, picture: &DesktopPicture) -> Result<(), String> {
            if self.fail_set {
                return Err("denied".into());
            }
            self.pictures
                .insert(display_id.to_string(), picture.clone());
            Ok(())
        }

        fn extract_still(
            &mut self,
            _display_id: &str,
            _video: &Path,
            output: &Path,
        ) -> Result<(), String> {
            if self.fail_extract {
                return Err("unreadable".into());
            }
            fs::write(output, b"jpeg").map_err(|e| e.to_string())?;
            self.extracted.push(output.to_path_buf());
            Ok(())
        }
    }

    fn original() -> DesktopPicture {
        DesktopPicture {
            url: "file:///System/Library/Desktop%20Pictures/Sequoia.heic".into(),
            scaling: Some(3),
            allow_clipping: Some(true),
            fill_rgba: None,
        }
    }

    fn target(video: &str, aspect: VideoAspectMode) -> WallpaperTarget {
        WallpaperTarget {
            display_id: "CGDISPLAY1".into(),
            video: PathBuf::from(video),
            aspect,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("auralis-lock-{name}-{}", std::process::id()))
            .join("Auralis")
            .join("lock-screen");
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn records_original_applies_still_and_restores_exactly() {
        let dir = temp_dir("cycle");
        let mut sync = LockScreenSync::new(dir.clone());
        let mut backend = FakeBackend::default();
        backend.pictures.insert("CGDISPLAY1".into(), original());
        let mut originals = HashMap::new();

        let report = sync.reconcile(
            true,
            &[target("/m/a.mp4", VideoAspectMode::Fill)],
            &mut originals,
            &mut backend,
        );
        assert!(report.errors.is_empty());
        assert!(report.config_changed);
        assert_eq!(originals.get("CGDISPLAY1"), Some(&original()));
        let set = backend.pictures["CGDISPLAY1"].clone();
        assert!(LockScreenSync::is_auralis_still(
            &set.url.replace('\\', "/")
        ));
        assert_eq!(set.allow_clipping, Some(true));
        let first_still = backend.extracted[0].clone();
        assert!(first_still.exists());

        // Same wallpaper: nothing happens.
        let report = sync.reconcile(
            true,
            &[target("/m/a.mp4", VideoAspectMode::Fill)],
            &mut originals,
            &mut backend,
        );
        assert_eq!(report, SyncReport::default());
        assert_eq!(backend.extracted.len(), 1);

        // New scaling: a new still replaces the old one; the original stays recorded once.
        sync.reconcile(
            true,
            &[target("/m/a.mp4", VideoAspectMode::Fit)],
            &mut originals,
            &mut backend,
        );
        assert_eq!(backend.extracted.len(), 2);
        assert!(!first_still.exists());
        assert_eq!(backend.pictures["CGDISPLAY1"].allow_clipping, Some(false));
        assert_eq!(originals.get("CGDISPLAY1"), Some(&original()));

        // Wallpaper stopped: the original comes back and the still is deleted.
        let report = sync.reconcile(true, &[], &mut originals, &mut backend);
        assert!(report.config_changed && report.errors.is_empty());
        assert_eq!(backend.pictures["CGDISPLAY1"], original());
        assert!(originals.is_empty());
        assert!(!backend.extracted[1].exists());
        let _ = fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn turning_off_or_quitting_restores_and_a_relaunch_finds_the_saved_original() {
        let dir = temp_dir("off");
        let mut backend = FakeBackend::default();
        backend.pictures.insert("CGDISPLAY1".into(), original());
        let mut originals = HashMap::new();
        let mut sync = LockScreenSync::new(dir.clone());
        sync.reconcile(
            true,
            &[target("/m/a.mp4", VideoAspectMode::Fill)],
            &mut originals,
            &mut backend,
        );

        // Force-quit: a fresh sync (next launch) sees an Auralis still as the current picture
        // and must not record it as the original; the saved original is still restored.
        let mut relaunched = LockScreenSync::new(dir.clone());
        relaunched.reconcile(
            true,
            &[target("/m/b.mp4", VideoAspectMode::Fill)],
            &mut originals,
            &mut backend,
        );
        assert_eq!(originals.get("CGDISPLAY1"), Some(&original()));

        let report = relaunched.reconcile(
            false,
            &[target("/m/b.mp4", VideoAspectMode::Fill)],
            &mut originals,
            &mut backend,
        );
        assert!(report.errors.is_empty());
        assert_eq!(backend.pictures["CGDISPLAY1"], original());
        assert!(originals.is_empty());
        let _ = fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn failures_are_reported_once_and_keep_the_original() {
        let dir = temp_dir("fail");
        let mut backend = FakeBackend {
            fail_extract: true,
            ..FakeBackend::default()
        };
        backend.pictures.insert("CGDISPLAY1".into(), original());
        let mut originals = HashMap::new();
        let mut sync = LockScreenSync::new(dir.clone());
        let wallpaper = [target("/m/a.mp4", VideoAspectMode::Fill)];

        let report = sync.reconcile(true, &wallpaper, &mut originals, &mut backend);
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].starts_with("Could not capture a still frame"));
        assert_eq!(backend.pictures["CGDISPLAY1"], original());
        // Not retried every tick.
        let report = sync.reconcile(true, &wallpaper, &mut originals, &mut backend);
        assert!(report.errors.is_empty());

        // A failed restore keeps the original recorded for a later retry.
        backend.fail_extract = false;
        sync.reconcile(
            true,
            &[target("/m/b.mp4", VideoAspectMode::Fill)],
            &mut originals,
            &mut backend,
        );
        backend.fail_set = true;
        let report = sync.restore_all(&mut originals, &mut backend);
        assert_eq!(report.errors.len(), 1);
        assert!(originals.contains_key("CGDISPLAY1"));
        // Turning the setting off does not re-report the same failure on every pass.
        let report = sync.reconcile(false, &[], &mut originals, &mut backend);
        assert!(report.errors.is_empty());
        // Quit retries and succeeds once macOS accepts the change.
        backend.fail_set = false;
        let report = sync.restore_all(&mut originals, &mut backend);
        assert!(report.errors.is_empty());
        assert_eq!(backend.pictures["CGDISPLAY1"], original());
        assert!(originals.is_empty());
        let _ = fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn still_picture_scales_like_the_wallpaper() {
        let fill = still_picture(Path::new("/x/s.jpg"), VideoAspectMode::Fill);
        assert_eq!(fill.scaling, Some(SCALE_PROPORTIONALLY_UP_OR_DOWN));
        assert_eq!(fill.allow_clipping, Some(true));
        let fit = still_picture(Path::new("/x/s.jpg"), VideoAspectMode::Fit);
        assert_eq!(fit.allow_clipping, Some(false));
        assert_eq!(fit.fill_rgba, Some([0, 0, 0, u16::MAX]));
        assert!(HELP_TEXT.contains("still frame"));
    }
}
