//! Auralis Application Entry Point

use auralis::{PlatformCapabilities, VERSION};

fn main() {
    let caps = PlatformCapabilities::detect();
    println!("Auralis Live Wallpaper Engine v{VERSION}");
    println!("Target Platform: {}", caps.os_name);
    println!("Desktop Layering Supported: {}", caps.wallpaper_layering);
    println!("Hardware Decoding Supported: {}", caps.hardware_decoding);
    println!("Multi-Display Supported: {}", caps.multi_display);

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        eprintln!("Warning: Auralis primarily targets macOS and Windows. Current OS is unsupported.");
    }
}
