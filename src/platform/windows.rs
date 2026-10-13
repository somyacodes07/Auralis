//! Windows Native Platform Adapter
//!
//! Implements desktop wallpaper window layering behind desktop icons using the
//! Progman / WorkerW message pump architecture, DPI-aware physical monitor enumeration,
//! flicker-free host window management, system power state inspection, and accurate
//! full-screen application occlusion detection.

use super::windows_player::MfVideoPlayer;
use super::{DisplayInfo, PlatformAdapter, PlatformError, PowerStatus};
use std::collections::HashMap;
use std::mem;
use std::path::Path;
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, ENUM_CURRENT_SETTINGS,
    HDC, HMONITOR, MONITORINFOEXW,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, OpenDesktopW, SetThreadDesktop,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, FindWindowExW, FindWindowW,
    GetClassNameW, GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetWindowLongPtrW,
    GetWindowRect, IsIconic, IsWindow, IsWindowVisible, RegisterClassW, SendMessageTimeoutW,
    SetParent, SetProcessDPIAware, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, HTTRANSPARENT,
    SMTO_NORMAL, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, WM_ERASEBKGND,
    WM_NCHITTEST, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
};

/// Message code sent to Progman to spawn the WorkerW behind desktop icons.
const WM_SPAWN_WORKERW: u32 = 0x052C;

/// Desired access rights for opening the interactive Default desktop (`DESKTOP_ALL_ACCESS`).
const DESKTOP_ACCESS_ALL: u32 = 0x01FF;

/// Custom window class name for our flicker-free wallpaper host window.
const WALLPAPER_HOST_CLASS_NAME: &str = "AuralisWallpaperHostWindowClass";

/// `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` constant (`(DPI_AWARENESS_CONTEXT)-4`).
const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

/// Native Windows wallpaper layering and Media Foundation playback adapter.
#[derive(Debug, Default)]
pub struct WindowsPlatformAdapter {
    /// Window handle of the WorkerW window behind desktop icons.
    /// Stored as an integer handle to remain thread-safe (`Send + Sync`).
    workerw_handle: Option<isize>,
    /// Active Media Foundation video players keyed by wallpaper host window handle (`HWND`).
    players: HashMap<isize, MfVideoPlayer>,
}

impl WindowsPlatformAdapter {
    /// Create a new Windows platform adapter with Per-Monitor V2 DPI awareness and
    /// interactive desktop attachment.
    #[must_use]
    pub fn new() -> Self {
        Self::ensure_dpi_awareness();
        Self::ensure_interactive_desktop();
        Self {
            workerw_handle: None,
            players: HashMap::new(),
        }
    }

    /// Enable Per-Monitor V2 DPI awareness on the current process so that monitor enumeration
    /// and `WorkerW` child window coordinates operate in true physical pixels (e.g. `1920x1080`
    /// rather than DPI-virtualized `1536x864` at 125% scaling).
    pub fn ensure_dpi_awareness() {
        type FnSetProcessDpiAwarenessContext = unsafe extern "system" fn(value: isize) -> BOOL;

        let user32_name = wide_string("user32.dll");
        // SAFETY: user32.dll is already loaded in every Win32 process.
        let h_user32 = unsafe { GetModuleHandleW(user32_name.as_ptr()) };
        if !h_user32.is_null() {
            let proc_name = b"SetProcessDpiAwarenessContext\0";
            // SAFETY: Resolves SetProcessDpiAwarenessContext from user32.dll if supported (Win10 1703+).
            if let Some(addr) = unsafe { GetProcAddress(h_user32, proc_name.as_ptr()) } {
                let set_ctx: FnSetProcessDpiAwarenessContext = unsafe { mem::transmute(addr) };
                let ok = unsafe { set_ctx(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
                if ok != 0 {
                    return;
                }
            }
        }

        // Fallback to system DPI awareness
        // SAFETY: SetProcessDPIAware marks the process as system DPI aware.
        unsafe {
            let _ = SetProcessDPIAware();
        }
    }

    /// Ensure the calling thread is attached to the interactive "Default" desktop
    /// so that `Progman` and `WorkerW` are visible even when launched from an isolated
    /// terminal or background job object.
    fn ensure_interactive_desktop() {
        let progman_class = wide_string("Progman");
        // SAFETY: FindWindowW checks if Progman is already visible on the current thread desktop.
        let existing = unsafe { FindWindowW(progman_class.as_ptr(), std::ptr::null()) };
        if !existing.is_null() {
            return;
        }

        let default_desk = wide_string("Default");
        // SAFETY: OpenDesktopW opens a handle to the interactive "Default" desktop.
        let h_desk = unsafe { OpenDesktopW(default_desk.as_ptr(), 0, 0, DESKTOP_ACCESS_ALL) };
        if !h_desk.is_null() {
            // SAFETY: SetThreadDesktop attaches the current thread to the interactive desktop.
            // If it fails (e.g., thread already owns windows), we close the unused handle.
            let switched = unsafe { SetThreadDesktop(h_desk) };
            if switched == 0 {
                unsafe { CloseDesktop(h_desk) };
            }
        }
    }

    /// Register the custom flicker-free window class for wallpaper host windows.
    fn register_wallpaper_host_class() {
        unsafe extern "system" fn wallpaper_host_wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            match msg {
                // Never erase background with a solid brush; EVR owns the entire surface.
                // This eliminates background repaint flicker during video presentation.
                WM_ERASEBKGND => 1,
                // Pass mouse hit-testing through to the desktop icon layer.
                WM_NCHITTEST => HTTRANSPARENT as LRESULT,
                _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }

        let class_name = wide_string(WALLPAPER_HOST_CLASS_NAME);
        // SAFETY: GetModuleHandleW(null) retrieves the current executable's HINSTANCE.
        let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };

        let wnd_class = WNDCLASSW {
            style: 0, // No CS_HREDRAW | CS_VREDRAW to prevent full-surface invalidation flicker
            lpfnWndProc: Some(wallpaper_host_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(), // Null brush prevents white/gray background flash
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };

        // SAFETY: RegisterClassW is idempotent if already registered for this module.
        unsafe {
            RegisterClassW(&wnd_class);
        }
    }

    /// Locate or spawn the WorkerW window layered directly behind desktop icons.
    pub fn get_or_spawn_workerw(&mut self) -> Result<HWND, PlatformError> {
        if let Some(hwnd_val) = self.workerw_handle {
            let candidate = hwnd_val as HWND;
            // SAFETY: IsWindow checks whether the cached HWND is still valid in the OS window table.
            if unsafe { IsWindow(candidate) } != 0 {
                return Ok(candidate);
            }
            self.workerw_handle = None;
        }

        Self::ensure_dpi_awareness();
        Self::ensure_interactive_desktop();

        let progman_class = wide_string("Progman");
        let shelldll_class = wide_string("SHELLDLL_DefView");
        let workerw_class = wide_string("WorkerW");

        // SAFETY: FindWindowW is called with a valid null-terminated wide string pointer.
        let progman = unsafe { FindWindowW(progman_class.as_ptr(), std::ptr::null()) };
        if progman.is_null() {
            return Err(PlatformError::DesktopWindowNotFound);
        }

        // Send 0x052C message to Progman to split desktop into WorkerW layers
        let mut send_result: usize = 0;
        // SAFETY: SendMessageTimeoutW is called with valid progman handle and output pointer.
        unsafe {
            SendMessageTimeoutW(
                progman,
                WM_SPAWN_WORKERW,
                0,
                0,
                SMTO_NORMAL,
                1000,
                &mut send_result,
            );
        }

        struct EnumContext {
            shelldll_class: Vec<u16>,
            workerw_class: Vec<u16>,
            target_workerw: HWND,
        }

        let mut context = EnumContext {
            shelldll_class,
            workerw_class,
            target_workerw: std::ptr::null_mut(),
        };

        unsafe extern "system" fn enum_windows_proc(top_hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: lparam points to our stack-allocated EnumContext for the duration of EnumWindows.
            let ctx = unsafe { &mut *(lparam as *mut EnumContext) };

            // Look for SHELLDLL_DefView inside this top-level window
            // SAFETY: FindWindowExW is called with valid window handles and null-terminated class name.
            let shell_view = unsafe {
                FindWindowExW(
                    top_hwnd,
                    std::ptr::null_mut(),
                    ctx.shelldll_class.as_ptr(),
                    std::ptr::null(),
                )
            };

            if !shell_view.is_null() {
                // The target WorkerW is the next sibling window behind SHELLDLL_DefView
                // SAFETY: FindWindowExW searches for the WorkerW sibling after top_hwnd.
                let workerw = unsafe {
                    FindWindowExW(
                        std::ptr::null_mut(),
                        top_hwnd,
                        ctx.workerw_class.as_ptr(),
                        std::ptr::null(),
                    )
                };

                if !workerw.is_null() {
                    ctx.target_workerw = workerw;
                    return 0; // Stop enumeration
                }
            }

            1 // Continue enumeration
        }

        // SAFETY: EnumWindows is called with a valid extern "system" callback and pointer to context.
        unsafe {
            EnumWindows(Some(enum_windows_proc), &mut context as *mut _ as LPARAM);
        }

        // Fallback: on Newer Windows 11 builds (24H2+), WorkerW may be a child of Progman directly
        if context.target_workerw.is_null() {
            let workerw_in_progman = unsafe {
                // SAFETY: Searches for a WorkerW child window inside Progman.
                FindWindowExW(
                    progman,
                    std::ptr::null_mut(),
                    context.workerw_class.as_ptr(),
                    std::ptr::null(),
                )
            };
            if !workerw_in_progman.is_null() {
                context.target_workerw = workerw_in_progman;
            }
        }

        if context.target_workerw.is_null() {
            return Err(PlatformError::WorkerWCreationFailed(
                "Could not find WorkerW sibling or child of Progman".to_string(),
            ));
        }

        self.workerw_handle = Some(context.target_workerw as isize);
        Ok(context.target_workerw)
    }
}

impl PlatformAdapter for WindowsPlatformAdapter {
    fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        Self::ensure_dpi_awareness();
        let mut displays = Vec::new();

        unsafe extern "system" fn monitor_enum_proc(
            hmonitor: HMONITOR,
            _hdc: HDC,
            _lprect: *mut RECT,
            lparam: LPARAM,
        ) -> BOOL {
            // SAFETY: lparam points to our stack-allocated Vec<DisplayInfo> for the duration of EnumDisplayMonitors.
            let displays = unsafe { &mut *(lparam as *mut Vec<DisplayInfo>) };

            // SAFETY: MONITORINFOEXW is a plain-old-data C struct safe to zero-initialize.
            let mut info: MONITORINFOEXW = unsafe { mem::zeroed() };
            info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;

            // SAFETY: GetMonitorInfoW is called with a valid hmonitor and properly sized MONITORINFOEXW struct.
            if unsafe { GetMonitorInfoW(hmonitor, &mut info as *mut _ as *mut _) } != 0 {
                let rect = info.monitorInfo.rcMonitor;
                let mut left = rect.left;
                let mut top = rect.top;
                let mut width = (rect.right - rect.left).unsigned_abs();
                let mut height = (rect.bottom - rect.top).unsigned_abs();
                let is_primary = (info.monitorInfo.dwFlags & 1) != 0;

                // Query true hardware physical pixel dimensions via EnumDisplaySettingsW
                // so DPI scaling (e.g., 125% scaling turning 1920x1080 into 1536x864) never
                // shrinks the wallpaper below full-screen physical resolution.
                let mut devmode: DEVMODEW = unsafe { mem::zeroed() };
                devmode.dmSize = mem::size_of::<DEVMODEW>() as u16;
                // SAFETY: EnumDisplaySettingsW queries active hardware display mode for szDevice.
                if unsafe {
                    EnumDisplaySettingsW(
                        info.szDevice.as_ptr(),
                        ENUM_CURRENT_SETTINGS,
                        &mut devmode,
                    )
                } != 0
                    && devmode.dmPelsWidth > 0
                    && devmode.dmPelsHeight > 0
                {
                    width = devmode.dmPelsWidth;
                    height = devmode.dmPelsHeight;
                    // Position fields in DEVMODEW union1.s2.dmPosition
                    let pos = unsafe { devmode.Anonymous1.Anonymous2.dmPosition };
                    left = pos.x;
                    top = pos.y;
                }

                let dev_name = string_from_wide(&info.szDevice);
                let id = format!("DISPLAY_{}", displays.len() + 1);

                displays.push(DisplayInfo {
                    id,
                    name: dev_name,
                    left,
                    top,
                    width,
                    height,
                    is_primary,
                });
            }

            1 // Continue enumeration
        }

        // SAFETY: EnumDisplayMonitors is called with null HDC and valid callback and vector pointer.
        let success = unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(monitor_enum_proc),
                &mut displays as *mut _ as LPARAM,
            )
        };

        if success == 0 {
            return Err(PlatformError::DisplayEnumerationFailed(
                "EnumDisplayMonitors call failed".to_string(),
            ));
        }

        if displays.is_empty() {
            displays.push(DisplayInfo {
                id: "PRIMARY".to_string(),
                name: "Primary Display".to_string(),
                left: 0,
                top: 0,
                width: 1920,
                height: 1080,
                is_primary: true,
            });
        }

        Ok(displays)
    }

    fn query_power_status(&self) -> Result<PowerStatus, PlatformError> {
        // SAFETY: SYSTEM_POWER_STATUS is a plain C struct safe to zero-initialize.
        let mut status: SYSTEM_POWER_STATUS = unsafe { mem::zeroed() };

        // SAFETY: GetSystemPowerStatus writes into the valid SYSTEM_POWER_STATUS pointer.
        let ok = unsafe { GetSystemPowerStatus(&mut status) };
        if ok == 0 {
            return Err(PlatformError::PowerQueryFailed(
                "GetSystemPowerStatus returned 0".to_string(),
            ));
        }

        let percent = if status.BatteryLifePercent <= 100 {
            Some(status.BatteryLifePercent)
        } else {
            None
        };
        let battery_saver = status.SystemStatusFlag == 1;

        match status.ACLineStatus {
            0 => Ok(PowerStatus::OnBattery {
                percent,
                battery_saver,
            }),
            1 => {
                if battery_saver {
                    Ok(PowerStatus::OnBattery {
                        percent,
                        battery_saver: true,
                    })
                } else {
                    Ok(PowerStatus::OnAcPower)
                }
            }
            _ => Ok(PowerStatus::Unknown),
        }
    }

    fn is_display_occluded_by_fullscreen(&self, display: &DisplayInfo) -> bool {
        // SAFETY: GetForegroundWindow takes no arguments and returns the active foreground HWND.
        let fg_hwnd = unsafe { GetForegroundWindow() };
        if fg_hwnd.is_null() {
            return false;
        }

        // Ignore desktop, shell, minimized, or invisible windows
        // SAFETY: Standard Win32 window state queries on fg_hwnd.
        unsafe {
            if fg_hwnd == GetShellWindow() || fg_hwnd == GetDesktopWindow() {
                return false;
            }
            if IsWindowVisible(fg_hwnd) == 0 || IsIconic(fg_hwnd) != 0 {
                return false;
            }
        }

        if let Some(workerw) = self.workerw_handle {
            if fg_hwnd == (workerw as HWND) {
                return false;
            }
        }

        // Inspect window class name to ignore Explorer desktop layers, taskbars, and our own windows.
        // Without this check, clicking on the desktop selects `Progman` or `WorkerW` (which spans
        // the entire screen), falsely triggering full-screen pause and causing playback flicker!
        let mut class_buf = [0u16; 256];
        // SAFETY: GetClassNameW copies up to 256 UTF-16 code units of the window's class name.
        let class_len =
            unsafe { GetClassNameW(fg_hwnd, class_buf.as_mut_ptr(), class_buf.len() as i32) };
        if class_len > 0 {
            let class_name = String::from_utf16_lossy(&class_buf[..class_len as usize]);
            if matches!(
                class_name.as_str(),
                "Progman"
                    | "WorkerW"
                    | "Shell_TrayWnd"
                    | "Shell_SecondaryTrayWnd"
                    | "NotifyIconOverflowWindow"
                    | "Windows.UI.Core.CoreWindow"
                    | "XamlExplorerHostIslandWindow"
                    | "TopLevelWindowForOverflowXamlIsland"
                    | "MultitaskingViewFrame"
                    | "TaskListThumbnailWnd"
                    | "AuralisConfigStudioWindowClass"
                    | "AuralisWallpaperHostWindowClass"
            ) {
                return false;
            }
        }

        // SAFETY: RECT is a plain C struct safe to zero-initialize; GetWindowRect populates it.
        let mut rect: RECT = unsafe { mem::zeroed() };
        if unsafe { GetWindowRect(fg_hwnd, &mut rect) } == 0 {
            return false;
        }

        let display_right = display.left + (display.width as i32);
        let display_bottom = display.top + (display.height as i32);

        rect.left <= display.left
            && rect.top <= display.top
            && rect.right >= display_right
            && rect.bottom >= display_bottom
    }

    fn create_wallpaper_host_window(
        &mut self,
        display: &DisplayInfo,
    ) -> Result<isize, PlatformError> {
        Self::ensure_dpi_awareness();
        Self::register_wallpaper_host_class();
        let workerw = self.get_or_spawn_workerw()?;

        let class_name = wide_string(WALLPAPER_HOST_CLASS_NAME);
        let window_title = wide_string(&format!("AuralisWallpaperHost_{}", display.id));

        // SAFETY: GetModuleHandleW(null) returns the base HINSTANCE of the current executable.
        let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };

        // Compute coordinates relative to WorkerW's origin so the child window fills 100%
        // of the target monitor in physical pixels.
        let mut workerw_rect: RECT = unsafe { mem::zeroed() };
        let (rel_left, rel_top) = if unsafe { GetWindowRect(workerw, &mut workerw_rect) } != 0 {
            (
                display.left - workerw_rect.left,
                display.top - workerw_rect.top,
            )
        } else {
            (display.left, display.top)
        };

        // SAFETY: CreateWindowExW creates a borderless host window using our flicker-free class.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                window_title.as_ptr(),
                WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                rel_left,
                rel_top,
                display.width as i32,
                display.height as i32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null(),
            )
        };

        if hwnd.is_null() {
            return Err(PlatformError::WindowCreationFailed(format!(
                "CreateWindowExW failed for display {}",
                display.id
            )));
        }

        // Attach the host window into the WorkerW layer behind desktop icons and transition
        // its window style from WS_POPUP to WS_CHILD so the desktop compositor layers it cleanly.
        // SAFETY: SetParent and SetWindowLongPtrW operate on our valid hwnd and workerw.
        unsafe {
            SetParent(hwnd, workerw);
            let cur_style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let child_style =
                (cur_style & !WS_POPUP) | WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN;
            SetWindowLongPtrW(hwnd, GWL_STYLE, child_style as isize);

            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                rel_left,
                rel_top,
                display.width as i32,
                display.height as i32,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
            );
        }

        Ok(hwnd as isize)
    }

    fn destroy_wallpaper_host_window(&mut self, window_hwnd: isize) -> Result<(), PlatformError> {
        if let Some(mut player) = self.players.remove(&window_hwnd) {
            player.shutdown();
        }

        let hwnd = window_hwnd as HWND;
        if hwnd.is_null() {
            return Ok(());
        }

        // SAFETY: Check if window handle is valid before detaching and destroying.
        if unsafe { IsWindow(hwnd) } != 0 {
            unsafe {
                SetParent(hwnd, std::ptr::null_mut());
                DestroyWindow(hwnd);
            }
        }
        Ok(())
    }

    fn attach_to_desktop(
        &mut self,
        _display_id: &str,
        window_hwnd: isize,
    ) -> Result<(), PlatformError> {
        let workerw = self.get_or_spawn_workerw()?;
        let child_hwnd = window_hwnd as HWND;
        if child_hwnd.is_null() {
            return Err(PlatformError::AttachmentFailed(
                "Cannot attach null window handle".to_string(),
            ));
        }

        // SAFETY: SetParent attaches the wallpaper window handle into the desktop WorkerW container.
        unsafe { SetParent(child_hwnd, workerw) };
        Ok(())
    }

    fn detach_from_desktop(
        &mut self,
        _display_id: &str,
        window_hwnd: isize,
    ) -> Result<(), PlatformError> {
        let child_hwnd = window_hwnd as HWND;
        if child_hwnd.is_null() {
            return Ok(());
        }

        // SAFETY: SetParent(child_hwnd, null) restores the window to top-level window.
        unsafe { SetParent(child_hwnd, std::ptr::null_mut()) };
        Ok(())
    }

    fn start_video_playback(
        &mut self,
        window_hwnd: isize,
        path: &Path,
        muted: bool,
        volume: u8,
    ) -> Result<(), PlatformError> {
        // Create new player first so if Media Foundation rejects the file, the previous player stays alive
        let new_player = MfVideoPlayer::create_and_play(window_hwnd, path, muted, volume)?;
        if let Some(mut old_player) = self.players.insert(window_hwnd, new_player) {
            old_player.shutdown();
        }
        Ok(())
    }

    fn pause_video_playback(&mut self, window_hwnd: isize) -> Result<(), PlatformError> {
        if let Some(player) = self.players.get(&window_hwnd) {
            player.pause()?;
        }
        Ok(())
    }

    fn resume_video_playback(&mut self, window_hwnd: isize) -> Result<(), PlatformError> {
        if let Some(player) = self.players.get(&window_hwnd) {
            player.play()?;
        }
        Ok(())
    }

    fn stop_video_playback(&mut self, window_hwnd: isize) -> Result<(), PlatformError> {
        if let Some(mut player) = self.players.remove(&window_hwnd) {
            player.shutdown();
        }
        Ok(())
    }

    fn set_video_audio(
        &mut self,
        window_hwnd: isize,
        muted: bool,
        volume: u8,
    ) -> Result<(), PlatformError> {
        if let Some(player) = self.players.get_mut(&window_hwnd) {
            player.set_audio(muted, volume)?;
        }
        Ok(())
    }

    fn tick_video_playback(
        &mut self,
        window_hwnd: isize,
        loop_playback: bool,
    ) -> Result<(), PlatformError> {
        if let Some(player) = self.players.get_mut(&window_hwnd) {
            player.tick(loop_playback)?;
        }
        Ok(())
    }
}

/// Converts a Rust string slice to a null-terminated UTF-16 wide string.
pub(crate) fn wide_string(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Converts a null-terminated UTF-16 slice to a Rust String.
pub(crate) fn string_from_wide(slice: &[u16]) -> String {
    let len = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    String::from_utf16_lossy(&slice[..len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wide_string_conversion() {
        let text = "Auralis";
        let wide = wide_string(text);
        assert_eq!(wide.last(), Some(&0));
        assert_eq!(string_from_wide(&wide), text);
    }

    #[test]
    fn test_windows_adapter_instantiation() {
        let adapter = WindowsPlatformAdapter::new();
        assert!(adapter.workerw_handle.is_none());
    }

    #[test]
    fn test_enumerate_displays_windows() {
        let adapter = WindowsPlatformAdapter::new();
        let displays = adapter.enumerate_displays();
        assert!(displays.is_ok());
        let list = displays.unwrap();
        assert!(!list.is_empty());
        assert!(list.iter().any(|d| d.is_primary));
        for d in &list {
            assert!(d.width > 0);
            assert!(d.height > 0);
        }
    }

    #[test]
    fn test_query_power_status_windows() {
        let adapter = WindowsPlatformAdapter::new();
        let status = adapter.query_power_status();
        assert!(status.is_ok());
    }

    #[test]
    fn test_fullscreen_occlusion_query_does_not_fail() {
        let adapter = WindowsPlatformAdapter::new();
        let displays = adapter.enumerate_displays().unwrap();
        let _occluded = adapter.is_display_occluded_by_fullscreen(&displays[0]);
    }

    #[test]
    fn test_workerw_and_host_window_lifecycle() {
        let mut adapter = WindowsPlatformAdapter::new();
        let displays = adapter.enumerate_displays().unwrap();
        let primary = &displays[0];

        match adapter.create_wallpaper_host_window(primary) {
            Ok(hwnd) => {
                assert_ne!(hwnd, 0);
                assert!(adapter.destroy_wallpaper_host_window(hwnd).is_ok());
            }
            Err(PlatformError::DesktopWindowNotFound) => {
                // Acceptable only in headless CI service sessions where explorer.exe is not running
            }
            Err(other) => panic!("Unexpected desktop layering error: {other}"),
        }
    }
}
