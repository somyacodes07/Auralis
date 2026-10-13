//! Auralis Application Entry Point

use auralis::{create_platform_adapter, AppConfig, MediaController, PlatformCapabilities, VERSION};
use std::path::PathBuf;

fn main() {
    let caps = PlatformCapabilities::detect();
    println!("===============================================");
    println!("  Auralis Live Wallpaper Engine v{VERSION}");
    println!("===============================================");
    println!("Platform:                  {}", caps.os_name);
    println!("Desktop Layering Native:   {}", caps.wallpaper_layering);
    println!("Hardware Decoding Native:  {}", caps.hardware_decoding);
    println!("Multi-Display Support:     {}", caps.multi_display);

    // Initialize platform adapter and enumerate displays
    let adapter = create_platform_adapter();
    match adapter.enumerate_displays() {
        Ok(displays) => {
            println!("\nDetected Displays ({}):", displays.len());
            for d in &displays {
                let primary_tag = if d.is_primary { " (Primary)" } else { "" };
                println!(
                    "  [{}] {} — {}x{}{}",
                    d.id, d.name, d.width, d.height, primary_tag
                );
            }
        }
        Err(e) => eprintln!("Warning: Failed to enumerate displays: {e}"),
    }

    // Load or initialize persistent configuration
    let config_path = PathBuf::from("auralis_config.json");
    let config = AppConfig::load_from_file(&config_path).unwrap_or_default();
    println!("\nActive Policy:");
    println!("  Pause on full-screen:    {}", config.pause_on_fullscreen);
    println!("  Pause on battery:        {}", config.pause_on_battery);
    println!("  Launch at startup:       {}", config.launch_at_startup);

    let player = MediaController::new();
    println!("\nEngine Status: Ready ({:?})", player.state);
}
