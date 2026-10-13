//! Windows Native Platform Adapter
//!
//! Implements desktop wallpaper window layering behind desktop icons using the
//! Progman / WorkerW message pump architecture, and multi-display monitor enumeration.

use super::{DisplayInfo, PlatformAdapter, PlatformError};
use std::mem;
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, SendMessageTimeoutW, SetParent, SMTO_NORMAL,
};

/// Message code sent to Progman to spawn the WorkerW behind desktop icons.
const WM_SPAWN_WORKERW: u32 = 0x052C;

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
        Self {
            workerw_handle: None,
        }
    }

    /// Locate or spawn the WorkerW window layered directly behind desktop icons.
    pub fn get_or_spawn_workerw(&mut self) -> Result<HWND, PlatformError> {
        if let Some(hwnd_val) = self.workerw_handle {
            return Ok(hwnd_val as HWND);
        }

        // Encode wide strings
        let progman_class = wide_string("Progman");
        let shelldll_class = wide_string("SHELLDLL_DefView");
        let workerw_class = wide_string("WorkerW");

        // SAFETY: FindWindowW is called with a valid null-terminated wide string pointer.
        let progman = unsafe { FindWindowW(progman_class.as_ptr(), std::ptr::null()) };
        if progman.is_null() {
            return Err(PlatformError::DesktopWindowNotFound);
        }

        // Send undocumented 0x052C message to Progman to split desktop into WorkerW layers
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

        // Context to store located WorkerW handle during enumeration
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
            let ctx = unsafe { &mut *(lparam as *mut EnumContext) };

            // Look for SHELLDLL_DefView inside this top-level window
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

        // SAFETY: EnumWindows is called with a valid extern C callback and pointer to context.
        unsafe {
            EnumWindows(Some(enum_windows_proc), &mut context as *mut _ as LPARAM);
        }

        if context.target_workerw.is_null() {
            return Err(PlatformError::WorkerWCreationFailed(
                "Could not find WorkerW sibling of SHELLDLL_DefView".to_string(),
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
            let displays = unsafe { &mut *(lparam as *mut Vec<DisplayInfo>) };

            let mut info: MONITORINFOEXW = unsafe { mem::zeroed() };
            info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;

            // SAFETY: GetMonitorInfoW is called with a valid hmonitor and allocated MONITORINFOEXW struct.
            if unsafe { GetMonitorInfoW(hmonitor, &mut info as *mut _ as *mut _) } != 0 {
                let rect = info.monitorInfo.rcMonitor;
                let width = (rect.right - rect.left).unsigned_abs();
                let height = (rect.bottom - rect.top).unsigned_abs();
                let is_primary = (info.monitorInfo.dwFlags & 1) != 0;

                // Extract null-terminated device string
                let dev_name = string_from_wide(&info.szDevice);
                let id = format!("DISPLAY_{}", displays.len() + 1);

                displays.push(DisplayInfo {
                    id,
                    name: dev_name,
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
                width: 1920,
                height: 1080,
                is_primary: true,
            });
        }

        Ok(displays)
    }

    fn attach_to_desktop(
        &mut self,
        _display_id: &str,
        window_hwnd: isize,
    ) -> Result<(), PlatformError> {
        let workerw = self.get_or_spawn_workerw()?;
        let child_hwnd = window_hwnd as HWND;

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
    }
}
