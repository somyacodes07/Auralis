//! Auralis Application Entry Point & CLI

use auralis::{
    create_platform_adapter, validate_media_file, AppConfig, PlatformCapabilities, PowerStatus,
    WallpaperEngine, VERSION,
};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("status");

    match command {
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
        "verify-desktop" => run_verify_desktop(),
        "--help" | "-h" | "help" => {
            print_help();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("Unknown command: '{other}'");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("Auralis Live Wallpaper Engine v{VERSION}");
    println!("\nUsage:");
    println!("  auralis [status]                     Show system, display, and power status");
    println!("  auralis validate <file>              Validate a video file for wallpaper playback");
    println!("  auralis apply <file> [display_id]    Apply a video wallpaper and persist config");
    println!(
        "  auralis verify-desktop               Verify native desktop WorkerW window layering"
    );
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
