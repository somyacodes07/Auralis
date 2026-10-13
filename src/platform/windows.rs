//! Windows Native Platform Adapter
//!
//! Implements desktop wallpaper window layering behind desktop icons using the
//! Progman / WorkerW message pump architecture, multi-display monitor enumeration,
//! system power state inspection, and full-screen application occlusion detection.

use super::{DisplayInfo, PlatformAdapter, PlatformError, PowerStatus};
use std::mem;
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, OpenDesktopW, SetThreadDesktop,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, EnumWindows, FindWindowExW, FindWindowW, GetForegroundWindow,
    GetShellWindow, GetWindowRect, IsWindow, IsWindowVisible, SendMessageTimeoutW, SetParent,
    SetWindowPos, SMTO_NORMAL, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
};

/// Message code sent to Progman to spawn the WorkerW behind desktop icons.
const WM_SPAWN_WORKERW: u32 = 0x052C;

/// Desired access rights for opening the interactive Default desktop (`DESKTOP_ALL_ACCESS`).
const DESKTOP_ACCESS_ALL: u32 = 0x01FF;

/// Native Windows wallpaper layering adapter.
#[derive(Debug, Clone, Default)]
pub struct WindowsPlatformAdapter {
    /// Window handle of the WorkerW window behind desktop icons.
    /// Stored as an integer handle to remain thread-safe (`Send + Sync`).
    workerw_handle: Option<isize>,
}

impl WindowsPlatformAdapter {
    /// Create a new Windows platform adapter.
    #[must_use]
    pub fn new() -> Self {
        Self::ensure_interactive_desktop();
        Self {
            workerw_handle: None,
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
                let width = (rect.right - rect.left).unsigned_abs();
                let height = (rect.bottom - rect.top).unsigned_abs();
                let is_primary = (info.monitorInfo.dwFlags & 1) != 0;

                let dev_name = string_from_wide(&info.szDevice);
                let id = format!("DISPLAY_{}", displays.len() + 1);

                displays.push(DisplayInfo {
                    id,
                    name: dev_name,
                    left: rect.left,
                    top: rect.top,
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
        // SAFETY: GetForegroundWindow and GetShellWindow take no arguments and return window handles.
        let fg_hwnd = unsafe { GetForegroundWindow() };
        if fg_hwnd.is_null() {
            return false;
        }

        // Ignore desktop shell windows
        // SAFETY: GetShellWindow returns the desktop shell window handle.
        let shell_hwnd = unsafe { GetShellWindow() };
        if fg_hwnd == shell_hwnd {
            return false;
        }

        if let Some(workerw) = self.workerw_handle {
            if fg_hwnd == (workerw as HWND) {
                return false;
            }
        }

        // SAFETY: IsWindowVisible checks the visibility bit of fg_hwnd.
        if unsafe { IsWindowVisible(fg_hwnd) } == 0 {
            return false;
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
        let workerw = self.get_or_spawn_workerw()?;

        let class_name = wide_string("STATIC");
        let window_title = wide_string(&format!("AuralisWallpaperHost_{}", display.id));

        // SAFETY: GetModuleHandleW(null) returns the base HINSTANCE of the current executable.
        let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };

        // SAFETY: CreateWindowExW creates a borderless tool window using the standard system "STATIC" class.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                window_title.as_ptr(),
                WS_POPUP | WS_VISIBLE,
                display.left,
                display.top,
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

        // Attach the host window into the WorkerW layer behind desktop icons
        // SAFETY: SetParent reparents the newly created valid hwnd into workerw.
        unsafe { SetParent(hwnd, workerw) };

        // Position the window within the WorkerW coordinate space
        // SAFETY: SetWindowPos positions our valid hwnd without stealing focus.
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                display.left,
                display.top,
                display.width as i32,
                display.height as i32,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }

        Ok(hwnd as isize)
    }

    fn destroy_wallpaper_host_window(&mut self, window_hwnd: isize) -> Result<(), PlatformError> {
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
}

/// Converts a Rust string slice to a null-terminated UTF-16 wide string.
fn wide_string(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Converts a null-terminated UTF-16 slice to a Rust String.
fn string_from_wide(slice: &[u16]) -> String {
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
