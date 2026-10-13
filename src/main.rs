//! Auralis Application Entry Point & CLI

use auralis::{
    create_platform_adapter, validate_media_file, AppConfig, PlatformCapabilities, PowerStatus,
    WallpaperEngine, VERSION,
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
            let initial_video = args.get(1).map(PathBuf::from);
            run_gui(initial_video)
        }
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
        "--help" | "-h" | "help" => {
            print_help();
            ExitCode::SUCCESS
        }
        other => {
            // If the user dragged-and-dropped a video file onto auralis.exe, open the GUI with that file
            let candidate_path = PathBuf::from(other);
            if candidate_path.exists() {
                run_gui(Some(candidate_path))
            } else {
                eprintln!("Unknown command: '{other}'");
                print_help();
                ExitCode::FAILURE
            }
        }
    }
}

fn print_help() {
    println!("Auralis Live Wallpaper Engine v{VERSION}");
    println!("\nUsage:");
    println!("  auralis [gui] [file]                 Launch Auralis Configuration Studio UI");
    println!("  auralis status                       Show system, display, and power status");
    println!("  auralis validate <file>              Validate a video file for wallpaper playback");
    println!("  auralis apply <file> [display_id]    Apply a video wallpaper and persist config");
    println!(
        "  auralis play <file> [seconds]        Play a live video wallpaper for N seconds and exit"
    );
    println!(
        "  auralis verify-desktop               Verify native desktop WorkerW window layering"
    );
}

fn run_gui(initial_video: Option<PathBuf>) -> ExitCode {
    #[cfg(target_os = "windows")]
    {
        match auralis::platform::windows_gui::run_windows_gui(initial_video) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Error running Auralis Configuration Studio: {e}");
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = initial_video;
        run_status()
    }
}

fn run_status() -> ExitCode {
    let caps = PlatformCapabilities::detect();
    println!("===============================================");
    println!("  Auralis Live Wallpaper Engine v{VERSION}");
    println!("===============================================");
    println!("Platform:                  {}", caps.os_name);
    println!("Desktop Layering Native:   {}", caps.wallpaper_layering);
    println!("Hardware Decoding Native:  {}", caps.hardware_decoding);
    println!("Multi-Display Support:     {}", caps.multi_display);

    let config_path = PathBuf::from("auralis_config.json");
    let config = AppConfig::load_from_file(&config_path).unwrap_or_default();
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
                "  Launch at startup:       {}",
                engine.config.launch_at_startup
            );

            if let Some(default_wp) = &engine.config.default_wallpaper {
                println!(
                    "  Configured Wallpaper:    {}",
                    default_wp.media_path.display()
                );
            } else {
                println!("  Configured Wallpaper:    None");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error initializing Auralis Wallpaper Engine: {e}");
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
    let config_path = PathBuf::from("auralis_config.json");
    let config = AppConfig::load_from_file(&config_path).unwrap_or_default();
    let adapter = create_platform_adapter();

    let mut engine = match WallpaperEngine::new(config, adapter) {
        Ok(eng) => eng,
        Err(e) => {
            eprintln!("Failed to initialize engine: {e}");
            return ExitCode::FAILURE;
        }
    };

    let display_id = if let Some(id) = target_display {
        id.to_string()
    } else if let Some(primary) = engine.displays().iter().find(|d| d.is_primary) {
        primary.id.clone()
    } else if let Some(first) = engine.displays().first() {
        first.id.clone()
    } else {
        eprintln!("No displays available.");
        return ExitCode::FAILURE;
    };

    match engine.apply_wallpaper(&display_id, path.clone()) {
        Ok(()) => {
            if let Err(e) = engine.config.save_to_file(&config_path) {
                eprintln!("Warning: Wallpaper applied but config save failed: {e}");
                return ExitCode::FAILURE;
            }
            println!(
                "[APPLIED] Wallpaper '{}' active on display [{}] and saved to {}.",
                path.display(),
                display_id,
                config_path.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to apply wallpaper: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_play_for_duration(path: PathBuf, seconds: u64) -> ExitCode {
    let config_path = PathBuf::from("auralis_config.json");
    let mut config = AppConfig::load_from_file(&config_path).unwrap_or_default();
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
        .unwrap_or_else(|| "DISPLAY_1".to_string());

    if let Err(e) = engine.apply_wallpaper(&display_id, path.clone()) {
        eprintln!("[ERROR] Failed to start hardware video playback: {e}");
        return ExitCode::FAILURE;
    }

    let hwnd = engine.host_window(&display_id).unwrap_or(0);
    println!(
        "[PLAYING] Hardware-decoded video '{}' rendering on [{}] (WorkerW HWND 0x{hwnd:X}) for {seconds}s...",
        path.display(),
        display_id
    );

    let start = Instant::now();
    let total = Duration::from_secs(seconds);
    while start.elapsed() < total {
        #[cfg(target_os = "windows")]
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
        let _ = engine.tick();
        std::thread::sleep(Duration::from_millis(50));
    }

    engine.shutdown();
    println!("[DONE] Playback completed and WorkerW host window cleanly destroyed.");
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

    let primary = displays
        .iter()
        .find(|d| d.is_primary)
        .or_else(|| displays.first())
        .expect("At least one display must be returned");

    println!(
        "Verifying native desktop layering on [{}] ({}x{} at {}, {})...",
        primary.id, primary.width, primary.height, primary.left, primary.top
    );

    match adapter.create_wallpaper_host_window(primary) {
        Ok(hwnd) => {
            println!(
                "[PASS] Created and layered native host window (HWND: 0x{hwnd:X}) behind desktop icons."
            );
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
