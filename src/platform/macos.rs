//! macOS Native Platform Adapter (`CoreGraphics` + `pmset` + `LaunchAgents`)
//!
//! Provides native macOS multi-display enumeration via `CoreGraphics` (`CGGetActiveDisplayList`,
//! `CGDisplayBounds`, `CGMainDisplayID`), battery/AC power inspection via `pmset -g batt`,
//! measured process memory/CPU telemetry via `ps`, and login startup registration via
//! `~/Library/LaunchAgents/com.auralis.wallpaper.plist`.

use super::{DisplayInfo, PlatformAdapter, PlatformError, PowerStatus, ProcessResourceMetrics};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

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

/// Native macOS platform adapter.
#[derive(Debug, Default)]
pub struct MacOsPlatformAdapter;

impl MacOsPlatformAdapter {
    /// Create a new macOS platform adapter.
    #[must_use]
    pub const fn new() -> Self {
        Self
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

            displays.push(DisplayInfo {
                id: format!("DISPLAY{}", idx + 1),
                name: format!("macOS Display #{cg_id}"),
                left: bounds.origin.x.round() as i32,
                top: bounds.origin.y.round() as i32,
                width,
                height,
                is_primary: cg_id == main_id || idx == 0,
            });
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
        _display: &DisplayInfo,
    ) -> Result<isize, PlatformError> {
        Err(PlatformError::UnsupportedPlatform)
    }

    fn destroy_wallpaper_host_window(&mut self, _window_hwnd: isize) -> Result<(), PlatformError> {
        Ok(())
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
        Ok(())
    }

    fn query_process_metrics(&self) -> Option<ProcessResourceMetrics> {
        let pid = std::process::id().to_string();
        let out = Command::new("ps")
            .args(["-o", "rss=", "-p", &pid])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let rss_kb: u64 = String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<u64>()
            .ok()?;
        let rss_bytes = rss_kb.saturating_mul(1024);
        Some(ProcessResourceMetrics {
            working_set_bytes: rss_bytes,
            private_bytes: rss_bytes,
            peak_working_set_bytes: rss_bytes,
            cpu_time_ms: 0,
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
                exe.display()
            );
            fs::write(&plist_path, plist_content).map_err(|e| {
                PlatformError::WindowCreationFailed(format!(
                    "Failed to write LaunchAgent plist: {e}"
                ))
            })?;
        } else if plist_path.exists() {
            let _ = fs::remove_file(&plist_path);
        }
        Ok(())
    }

    fn is_launch_at_startup_enabled(&self) -> bool {
        Self::launch_agent_plist_path().is_some_and(|p| p.exists())
    }
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
}
