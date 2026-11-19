//! Auralis Application Entry Point & CLI

use auralis::{
    create_platform_adapter, validate_media_file, AppConfig, DisplayWallpaperConfig,
    PlatformCapabilities, PowerStatus, WallpaperEngine,
};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("gui");

    match command {
        "gui" => {
            let start_minimized = args
                .iter()
                .skip(1)
                .any(|a| a == "--minimized" || a == "--background");
            let initial_video = args
                .iter()
                .skip(1)
                .find(|a| !a.starts_with('-'))
                .map(PathBuf::from);
            run_gui(initial_video, start_minimized)
        }
        "--minimized" | "--background" => run_gui(None, true),
        "status" => run_status(),
        "validate" => {
            if let Some(path_str) = args.get(1) {
                run_validate(PathBuf::from(path_str))
            } else {
                eprintln!("Usage: auralis validate <media-file-path>");
                ExitCode::FAILURE
            }
        }
        "apply" => {
            if let Some(path_str) = args.get(1) {
                let target_display = args.get(2).map(String::as_str);
                run_apply(PathBuf::from(path_str), target_display)
            } else {
                eprintln!("Usage: auralis apply <media-file-path> [display_id]");
                ExitCode::FAILURE
            }
        }
        "play" => {
            if let Some(path_str) = args.get(1) {
                let seconds = args.get(2).and_then(|s| s.parse::<u64>().ok()).unwrap_or(5);
                run_play_for_duration(PathBuf::from(path_str), seconds)
            } else {
                eprintln!("Usage: auralis play <media-file-path> [duration_seconds]");
                ExitCode::FAILURE
            }
        }
        "verify-desktop" => run_verify_desktop(),
        "quit" => run_quit(),
        "update" => run_update(args.iter().skip(1).any(|a| a == "--check")),
        "--help" | "-h" | "help" => {
            print_help();
            ExitCode::SUCCESS
        }
        other => {
            // If the user dragged-and-dropped a video file onto auralis.exe, open the GUI with that file
            let candidate_path = PathBuf::from(other);
            if candidate_path.exists() {
                run_gui(Some(candidate_path), false)
            } else {
                eprintln!("Unknown command: '{other}'");
                print_help();
                ExitCode::FAILURE
            }
        }
    }
}

fn print_help() {
    println!("Auralis v{}", auralis::version_label());
    println!("\nUsage:");
    println!("  auralis [gui] [file] [--minimized]   Launch the Auralis desktop app");
    println!(
        "  auralis status                       Show system, display, power, and resource status"
    );
    println!("  auralis validate <file>              Validate a video file for wallpaper playback");
    println!("  auralis apply <file> [display_id]    Configure wallpaper and reload active session if running");
    println!(
        "  auralis play <file> [seconds]        Play a live video wallpaper for N seconds and exit"
    );
    println!(
        "  auralis verify-desktop               Check native wallpaper host creation and cleanup"
    );
    println!(
        "  auralis quit                         Quit the running app and restore the desktop (Windows)"
    );
    println!(
        "  auralis update [--check]             Install the newest build from GitHub (--check only reports it)"
    );
}

/// Check GitHub for a newer build and, unless `check_only`, download, verify and install it.
fn run_update(check_only: bool) -> ExitCode {
    use auralis::update::{self, InstallKind, Relaunch};
    println!(
        "Auralis {} ({}). Checking GitHub for a newer build...",
        update::ReleaseVersion::current().label(),
        match InstallKind::detect() {
            InstallKind::WindowsInstaller => "installed",
            InstallKind::WindowsPortable => "portable",
            InstallKind::MacApp => "app bundle",
            InstallKind::Unmanaged => "development build",
        }
    );
    let offer = match update::check_for_update() {
        Ok(Some(offer)) => offer,
        Ok(None) => {
            if InstallKind::detect().asset_suffix().is_none() {
                println!("This build does not update from GitHub releases.");
            } else {
                println!("You have the newest build.");
            }
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("[ERROR] {error}.");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "Auralis {} is available ({:.1} MB): {}",
        offer.version.label(),
        offer.size as f64 / 1_048_576.0,
        offer.release_url
    );
    if check_only {
        return ExitCode::SUCCESS;
    }
    if !offer.kind.can_self_install() {
        eprintln!("[ERROR] {}.", update::UpdateError::NotSupported);
        return ExitCode::FAILURE;
    }
    println!("Downloading {}...", offer.file_name);
    let file = match update::download(&offer) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("[ERROR] {error}.");
            return ExitCode::FAILURE;
        }
    };
    println!("Checksum verified. Installing...");
    match update::install(&offer, &file, Relaunch::No) {
        Ok(()) if offer.kind == InstallKind::WindowsInstaller => {
            println!("The installer is running and closes Auralis while it upgrades it. Open Auralis again when it finishes.");
            ExitCode::SUCCESS
        }
        Ok(()) => {
            println!(
                "Updated. Quit Auralis from the menu bar and open it again to use the new version."
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("[ERROR] {error}.");
            ExitCode::FAILURE
        }
    }
}

/// Ask a running Studio to quit and restore the desktop; used by installers and uninstallers.
/// Exits 0 when no instance runs or it quit, non-zero if it was still running after 5 seconds.
fn run_quit() -> ExitCode {
    #[cfg(target_os = "windows")]
    {
        use auralis::platform::windows_gui::{quit_running_instance, QuitOutcome};
        match quit_running_instance(Duration::from_secs(5)) {
            QuitOutcome::NotRunning => {
                println!("Auralis is not running.");
                ExitCode::SUCCESS
            }
            QuitOutcome::Quit => {
                println!("Auralis quit and restored the desktop.");
                ExitCode::SUCCESS
            }
            QuitOutcome::TimedOut => {
                eprintln!(
                    "Auralis did not quit within 5 seconds. Choose Quit from its tray icon menu, then try again."
                );
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        eprintln!(
            "auralis quit is available on Windows only. Quit Auralis from its menu bar icon."
        );
        ExitCode::FAILURE
    }
}

/// Start the diagnostics log for long-running commands. One-shot CLI commands print to the
/// terminal instead and do not write a log.
fn start_diagnostics(command: &str) {
    auralis::diagnostics::init();
    auralis::diagnostics::info(&format!(
        "Auralis {} starting ({command}) on {}",
        auralis::version_label(),
        PlatformCapabilities::detect().os_name
    ));
}

fn run_gui(initial_video: Option<PathBuf>, start_minimized: bool) -> ExitCode {
    start_diagnostics("gui");
    #[cfg(target_os = "windows")]
    {
        match auralis::platform::windows_gui::run_windows_gui(initial_video, start_minimized) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Error running Auralis Studio: {e}");
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        match auralis::platform::macos::run_macos_gui(initial_video, start_minimized) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Error running Auralis menu bar app: {e}");
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (initial_video, start_minimized);
        run_status()
    }
}

fn run_status() -> ExitCode {
    let caps = PlatformCapabilities::detect();
    println!("===============================================");
    println!("  Auralis v{}", auralis::version_label());
    println!("===============================================");
    println!("Platform:                  {}", caps.os_name);
    println!("Desktop Layering Native:   {}", caps.wallpaper_layering);
    println!("Hardware Decoding Native:  {}", caps.hardware_decoding);
    println!("Multi-Display Support:     {}", caps.multi_display);

    let config_path = AppConfig::default_config_path();
    let (config, warning) = AppConfig::load_or_recover(&config_path);
    if let Some(w) = warning {
        eprintln!("Warning: {w}");
    }
    println!("Configuration File:        {}", config_path.display());
    println!(
        "Diagnostics Log:           {}",
        auralis::diagnostics::default_log_path().display()
    );

    let adapter = create_platform_adapter();

    match WallpaperEngine::new(config, adapter) {
        Ok(engine) => {
            match engine.query_power_status() {
                Ok(PowerStatus::OnAcPower) => {
                    println!("System Power Status:       AC Power (Full Performance)");
                }
                Ok(PowerStatus::OnBattery {
                    percent,
                    battery_saver,
                }) => {
                    let pct_str = percent
                        .map(|p| format!("{p}%"))
                        .unwrap_or_else(|| "Unknown".to_string());
                    println!(
                        "System Power Status:       Battery ({pct_str}, Saver: {battery_saver})"
                    );
                }
                Ok(PowerStatus::Unknown) => {
                    println!("System Power Status:       Unknown");
                }
                Err(e) => eprintln!("Warning: Power status query failed: {e}"),
            }

            if let Some(metrics) = engine.process_metrics() {
                let ram_mb = (metrics.working_set_bytes as f64) / (1024.0 * 1024.0);
                let priv_mb = (metrics.private_bytes as f64) / (1024.0 * 1024.0);
                let peak_mb = (metrics.peak_working_set_bytes as f64) / (1024.0 * 1024.0);
                let cpu_sec = (metrics.cpu_time_ms as f64) / 1000.0;
                println!(
                    "Measured Process Metrics:  Working Set {ram_mb:.2} MB | Private {priv_mb:.2} MB (Peak {peak_mb:.2} MB), CPU {cpu_sec:.3}s"
                );
            }

            let displays = engine.displays();
            println!("\nDetected Displays ({}):", displays.len());
            for d in displays {
                let primary_tag = if d.is_primary { " (Primary)" } else { "" };
                println!(
                    "  [{}] {} — {}x{} at ({}, {}){}",
                    d.id, d.name, d.width, d.height, d.left, d.top, primary_tag
                );
            }

            println!("\nActive Policy:");
            println!(
                "  Pause on full-screen:    {}",
                engine.config.pause_on_fullscreen
            );
            println!(
                "  Pause on battery:        {}",
                engine.config.pause_on_battery
            );
            println!(
                "  Launch at startup:       {} (OS Active: {})",
                engine.config.launch_at_startup,
                engine.is_launch_at_startup_enabled()
            );
            println!(
                "  Recent Wallpapers:       {}",
                engine.config.recent_wallpapers.len()
            );

            if let Some(default_wp) = &engine.config.default_wallpaper {
                println!(
                    "  Configured Wallpaper:    {} (Speed: {}%, Scale: {})",
                    default_wp.media_path.display(),
                    default_wp.speed_percent,
                    default_wp.aspect_mode.label()
                );
            } else {
                println!("  Configured Wallpaper:    None");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error initializing Auralis: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_validate(path: PathBuf) -> ExitCode {
    match validate_media_file(&path) {
        Ok(()) => {
            println!(
                "[VALID] '{}' is a supported, non-empty video file.",
                path.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[INVALID] {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_apply(path: PathBuf, target_display: Option<&str>) -> ExitCode {
    let wp_cfg = match DisplayWallpaperConfig::new(path.clone()) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("[ERROR] Failed to validate wallpaper media: {e}");
            return ExitCode::FAILURE;
        }
    };

    let config_path = AppConfig::default_config_path();
    let (mut config, warning) = AppConfig::load_or_recover(&config_path);
    if let Some(w) = warning {
        eprintln!("Warning: {w}");
    }

    let adapter = create_platform_adapter();
    let displays = match adapter.enumerate_displays() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[ERROR] Failed to enumerate displays: {e}");
            return ExitCode::FAILURE;
        }
    };

    let display_id = if let Some(id) = target_display {
        if !displays.iter().any(|d| d.id == id) {
            eprintln!("[ERROR] Display '{id}' not found among active monitors.");
            return ExitCode::FAILURE;
        }
        id.to_string()
    } else if let Some(primary) = displays.iter().find(|d| d.is_primary) {
        primary.id.clone()
    } else if let Some(first) = displays.first() {
        first.id.clone()
    } else {
        eprintln!("[ERROR] No active displays available.");
        return ExitCode::FAILURE;
    };

    // Preserve user audio/loop/speed/aspect settings if already configured on this display or default
    let mut final_wp = wp_cfg;
    if let Some(existing) = config
        .per_display
        .get(&display_id)
        .or(config.default_wallpaper.as_ref())
    {
        final_wp.loop_playback = existing.loop_playback;
        final_wp.muted = existing.muted;
        final_wp.volume = existing.volume;
        final_wp.speed_percent = existing.speed_percent;
        final_wp.aspect_mode = existing.aspect_mode;
    }

    config.record_recent_wallpaper(path.clone());
    config
        .per_display
        .insert(display_id.clone(), final_wp.clone());
    if config.default_wallpaper.is_none() || displays.len() == 1 {
        config.default_wallpaper = Some(final_wp);
    }

    if let Err(e) = config.save_to_file(&config_path) {
        eprintln!(
            "[ERROR] Failed to save configuration to {}: {e}",
            config_path.display()
        );
        return ExitCode::FAILURE;
    }

    #[cfg(target_os = "windows")]
    let notified_running = auralis::platform::windows_gui::notify_running_instance_reload();
    #[cfg(not(target_os = "windows"))]
    let notified_running = false;

    if notified_running {
        println!(
            "[APPLIED] Wallpaper '{}' saved to {} and reloaded in active Auralis session on [{}].",
            path.display(),
            config_path.display(),
            display_id
        );
    } else {
        let reload_note = if cfg!(target_os = "macos") {
            "a running macOS menu-bar session does not hot-reload CLI changes; use its menu or restart it"
        } else {
            "no resident renderer was notified; launch the desktop app to start playback"
        };
        println!(
            "[CONFIGURED] Wallpaper '{}' validated and saved for display [{}] in {} ({reload_note}).",
            path.display(),
            display_id,
            config_path.display()
        );
    }
    ExitCode::SUCCESS
}

fn run_play_for_duration(path: PathBuf, seconds: u64) -> ExitCode {
    start_diagnostics("play");
    let config_path = AppConfig::default_config_path();
    let (mut config, warning) = AppConfig::load_or_recover(&config_path);
    if let Some(w) = warning {
        eprintln!("Warning: {w}");
    }
    // Ensure battery pause does not block explicit CLI verification playback
    config.pause_on_battery = false;
    let adapter = create_platform_adapter();

    let mut engine = match WallpaperEngine::new(config, adapter) {
        Ok(eng) => eng,
        Err(e) => {
            eprintln!("Failed to initialize engine: {e}");
            return ExitCode::FAILURE;
        }
    };

    let display_id = engine
        .displays()
        .iter()
        .find(|d| d.is_primary)
        .or_else(|| engine.displays().first())
        .map(|d| d.id.clone())
        .unwrap_or_else(|| "DISPLAY1".to_string());

    if let Err(e) = engine.apply_wallpaper(&display_id, path.clone()) {
        eprintln!("[ERROR] Failed to start native video playback: {e}");
        return ExitCode::FAILURE;
    }

    let hwnd = engine.host_window(&display_id).unwrap_or(0);
    println!(
        "[STARTING] Native video '{}' on [{}] (host handle 0x{hwnd:X}) for {seconds}s...",
        path.display(),
        display_id
    );

    let start = Instant::now();
    let total = Duration::from_secs(seconds);
    while start.elapsed() < total {
        #[cfg(target_os = "windows")]
        // SAFETY: Standard Win32 non-blocking PeekMessageW loop on the current STA thread.
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
            };
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        #[cfg(target_os = "macos")]
        auralis::platform::macos::pump_native_events();
        if let Err(error) = engine.tick() {
            eprintln!("[ERROR] Native playback failed: {error}");
            engine.shutdown();
            return ExitCode::FAILURE;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if let Some(metrics) = engine.process_metrics() {
        let ws_mb = (metrics.working_set_bytes as f64) / (1024.0 * 1024.0);
        let priv_mb = (metrics.private_bytes as f64) / (1024.0 * 1024.0);
        let cpu_sec = (metrics.cpu_time_ms as f64) / 1000.0;
        println!(
            "[METRICS] Steady-state during playback: Working Set {ws_mb:.2} MB | Private {priv_mb:.2} MB | CPU {cpu_sec:.3}s"
        );
    }

    engine.shutdown();
    println!("[DONE] Playback completed and native host window cleanly destroyed.");
    ExitCode::SUCCESS
}

fn run_verify_desktop() -> ExitCode {
    let mut adapter = create_platform_adapter();
    let displays = match adapter.enumerate_displays() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[FAIL] Display enumeration failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    let Some(primary) = displays
        .iter()
        .find(|d| d.is_primary)
        .or_else(|| displays.first())
    else {
        eprintln!("[FAIL] No active displays found.");
        return ExitCode::FAILURE;
    };

    println!(
        "Verifying native desktop host creation on [{}] ({}x{} at {}, {})...",
        primary.id, primary.width, primary.height, primary.left, primary.top
    );

    match adapter.create_wallpaper_host_window(primary) {
        Ok(hwnd) => {
            println!("[PASS] Created native wallpaper host window (handle: 0x{hwnd:X}).");
            #[cfg(target_os = "macos")]
            {
                // Let AppKit order the window before reporting its on-screen state.
                for _ in 0..20 {
                    auralis::platform::macos::pump_native_events();
                    std::thread::sleep(Duration::from_millis(50));
                }
                println!(
                    "       Host window: {}",
                    auralis::platform::macos::describe_host_window(hwnd)
                );
            }
            if let Err(e) = adapter.destroy_wallpaper_host_window(hwnd) {
                eprintln!("[FAIL] Failed to clean up host window: {e}");
                return ExitCode::FAILURE;
            }
            println!("[PASS] Cleanly detached and destroyed native host window.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[FAIL] Desktop layering verification failed: {e}");
            ExitCode::FAILURE
        }
    }
}
