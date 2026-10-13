//! Native Windows Configuration UI, In-App Video Preview, System Tray & Event Loop (`windows_gui.rs`)
//!
//! Provides a lightweight, DPI-aware, flicker-free Win32 Configuration Studio window
//! with an embedded hardware-decoded video preview panel, drag-and-drop file support,
//! a persistent Windows system tray icon (`Shell_NotifyIconW`) with right-click context menu,
//! Windows login startup registration, and real-time hardware/process telemetry.
//!
//! All thread-local state accesses use `try_borrow` / `try_borrow_mut` and perform Win32
//! message-sending calls outside borrows where practical so synchronous Win32 message
//! re-entrancy can never trigger a `RefCell` borrow panic.

use super::windows::{string_from_wide, wide_string, WindowsPlatformAdapter};
use super::windows_player::MfVideoPlayer;
use crate::config::AppConfig;
use crate::engine::WallpaperEngine;
use crate::media::PlaybackState;
use crate::platform::{create_platform_adapter, PowerStatus};
use crate::{PlatformEvent, VERSION};
use std::cell::RefCell;
use std::mem;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetDC, GetDeviceCaps, GetStockObject, ReleaseDC, BLACK_BRUSH,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, DEFAULT_CHARSET, DEFAULT_GUI_FONT,
    DEFAULT_PITCH, FF_SWISS, FW_NORMAL, HBRUSH, HFONT, LOGPIXELSY, OUT_TT_PRECIS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Shell::{
    DragAcceptFiles, DragFinish, DragQueryFileW, Shell_NotifyIconW, HDROP, NIF_ICON, NIF_MESSAGE,
    NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, GetWindowTextLengthW, GetWindowTextW,
    KillTimer, LoadCursorW, LoadIconW, PostQuitMessage, RegisterClassW, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowTextW, ShowWindow, TrackPopupMenu, TranslateMessage,
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_PUSHBUTTON, CBN_SELCHANGE, CBS_DROPDOWNLIST,
    CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, ES_AUTOHSCROLL, IDC_ARROW, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG,
    PBT_APMPOWERSTATUSCHANGE, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
    SW_HIDE, SW_RESTORE, SW_SHOW, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, WM_APP, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DROPFILES, WM_LBUTTONDBLCLK,
    WM_LBUTTONUP, WM_POWERBROADCAST, WM_RBUTTONUP, WM_SETFONT, WM_TIMER, WNDCLASSW, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_MINIMIZEBOX, WS_OVERLAPPED,
    WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

pub const WINDOW_CLASS_NAME: &str = "AuralisConfigStudioWindowClass";
const PREVIEW_CLASS_NAME: &str = "AuralisPreviewSurfaceWindowClass";

const WM_AURALIS_SHOW_UI: u32 = WM_APP + 1;
const WM_AURALIS_RELOAD_CONFIG: u32 = WM_APP + 2;
const WM_AURALIS_TRAY_ICON: u32 = WM_APP + 3;

/// `SS_NOPREFIX` prevents `STATIC` controls from interpreting `&` as an underline accelerator.
const SS_NOPREFIX: u32 = 0x0080;

const ID_CMB_DISPLAY: usize = 100;
const ID_EDT_PATH: usize = 101;
const ID_BTN_BROWSE: usize = 102;
const ID_BTN_APPLY: usize = 103;
const ID_BTN_PAUSE_RESUME: usize = 104;
const ID_BTN_STOP: usize = 105;
const ID_CHK_MUTE: usize = 106;
const ID_CHK_LOOP: usize = 107;
const ID_BTN_VOL_DOWN: usize = 108;
const ID_BTN_VOL_UP: usize = 109;
const ID_CHK_FULLSCREEN: usize = 110;
const ID_CHK_BATTERY: usize = 111;
const ID_BTN_HIDE: usize = 112;
const ID_BTN_QUIT: usize = 113;
const ID_CHK_STARTUP: usize = 114;
const ID_BTN_PREVIEW: usize = 115;
const ID_BTN_STOP_PREVIEW: usize = 116;

const TRAY_ICON_ID: u32 = 1;
const IDM_TRAY_OPEN: usize = 2001;
const IDM_TRAY_PAUSE_RESUME: usize = 2002;
const IDM_TRAY_MUTE_UNMUTE: usize = 2003;
const IDM_TRAY_STOP: usize = 2004;
const IDM_TRAY_QUIT: usize = 2005;

const ID_TIMER_TICK: usize = 1;
/// 100ms tick interval ensures pre-EOS seamless loop seeking and responsive policy updates.
const TIMER_INTERVAL_MS: u32 = 100;

const BST_CHECKED: usize = 1;
const BST_UNCHECKED: usize = 0;

struct GuiState {
    engine: WallpaperEngine,
    config_path: PathBuf,
    ui_font: HFONT,
    cmb_display: HWND,
    edt_path: HWND,
    chk_mute: HWND,
    chk_loop: HWND,
    lbl_volume: HWND,
    chk_fullscreen: HWND,
    chk_battery: HWND,
    chk_startup: HWND,
    hwnd_preview: HWND,
    preview_player: Option<MfVideoPlayer>,
    preview_path: Option<PathBuf>,
    lbl_status: HWND,
    last_error: Option<String>,
    last_status_text: String,
    tray_added: bool,
}

thread_local! {
    static GUI_STATE: RefCell<Option<GuiState>> = const { RefCell::new(None) };
}

/// If a resident Auralis Configuration Studio instance is running on the desktop,
/// send it `WM_AURALIS_RELOAD_CONFIG` so it reloads `AppConfig` and applies changes live.
#[must_use]
pub fn notify_running_instance_reload() -> bool {
    WindowsPlatformAdapter::ensure_dpi_awareness();
    let class_wide = wide_string(WINDOW_CLASS_NAME);
    // SAFETY: FindWindowW queries the OS window table using a valid null-terminated UTF-16 class name.
    let existing = unsafe { FindWindowW(class_wide.as_ptr(), std::ptr::null()) };
    if existing.is_null() {
        return false;
    }
    // SAFETY: SendMessageW dispatches our custom reload message to the valid existing window handle.
    unsafe {
        SendMessageW(existing, WM_AURALIS_RELOAD_CONFIG, 0, 0);
    }
    true
}

/// Query the current system DPI scaling factor relative to 96 DPI (e.g. `1.25` at 125% scaling).
fn query_dpi_scale() -> (i32, f32) {
    // SAFETY: GetDC(null) retrieves the screen DC, GetDeviceCaps queries logical DPI, and ReleaseDC frees it.
    unsafe {
        let hdc = GetDC(std::ptr::null_mut());
        if hdc.is_null() {
            return (96, 1.0);
        }
        let dpi = GetDeviceCaps(hdc, LOGPIXELSY as i32).max(96);
        ReleaseDC(std::ptr::null_mut(), hdc);
        (dpi, (dpi as f32) / 96.0)
    }
}

#[inline]
fn scale_px(px: i32, scale: f32) -> i32 {
    ((px as f32) * scale).round() as i32
}

/// Register the black-background child window class used for the in-app video preview canvas.
fn register_preview_surface_class(hinstance: *mut std::ffi::c_void) {
    let class_wide = wide_string(PREVIEW_CLASS_NAME);
    // SAFETY: GetStockObject(BLACK_BRUSH) returns the system stock black brush handle.
    let black_brush = unsafe { GetStockObject(BLACK_BRUSH) };
    let wnd_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(DefWindowProcW),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: black_brush as HBRUSH,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_wide.as_ptr(),
    };
    // SAFETY: RegisterClassW registers the preview surface window class for this process.
    unsafe { RegisterClassW(&wnd_class) };
}

/// Launch the native Windows Configuration UI and enter the Win32 message & playback loop.
pub fn run_windows_gui(
    initial_video: Option<PathBuf>,
    start_minimized: bool,
) -> Result<(), String> {
    WindowsPlatformAdapter::ensure_dpi_awareness();
    WindowsPlatformAdapter::register_wallpaper_host_class();

    // If an Auralis Studio instance is already running in the background, bring its UI to front
    let class_wide = wide_string(WINDOW_CLASS_NAME);
    // SAFETY: FindWindowW checks for an existing Auralis configuration window on the desktop.
    let existing = unsafe { FindWindowW(class_wide.as_ptr(), std::ptr::null()) };
    if !existing.is_null() && initial_video.is_none() {
        // SAFETY: Existing window handle is non-null; sends show message and brings it to foreground.
        unsafe {
            SendMessageW(existing, WM_AURALIS_SHOW_UI, 0, 0);
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
        }
        return Ok(());
    }

    let config_path = AppConfig::default_config_path();
    let (mut config, load_warning) = AppConfig::load_or_recover(&config_path);
    if let Some(warn) = &load_warning {
        eprintln!("Warning: {warn}");
    }
    let adapter = create_platform_adapter();
    let startup_actual = adapter.is_launch_at_startup_enabled();
    config.launch_at_startup = startup_actual;

    let mut engine = WallpaperEngine::new(config, adapter).map_err(|e| e.to_string())?;

    // Restore any previously saved wallpaper configuration or apply initial CLI video
    if let Some(video_path) = initial_video {
        if let Some(primary) = engine
            .displays()
            .iter()
            .find(|d| d.is_primary)
            .or_else(|| engine.displays().first())
            .cloned()
        {
            let _ = engine.apply_wallpaper(&primary.id, video_path);
            let _ = engine.config.save_to_file(&config_path);
        }
    } else {
        engine.restore_saved_wallpapers();
    }

    let (dpi, scale) = query_dpi_scale();
    let font_height = -((10 * dpi) / 72);
    let font_face = wide_string("Segoe UI");
    // SAFETY: CreateFontW creates a ClearType-antialiased Segoe UI font scaled to system DPI.
    let ui_font = unsafe {
        CreateFontW(
            font_height,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_TT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_SWISS) as u32,
            font_face.as_ptr(),
        )
    };

    GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            *guard = Some(GuiState {
                engine,
                config_path,
                ui_font,
                cmb_display: std::ptr::null_mut(),
                edt_path: std::ptr::null_mut(),
                chk_mute: std::ptr::null_mut(),
                chk_loop: std::ptr::null_mut(),
                lbl_volume: std::ptr::null_mut(),
                chk_fullscreen: std::ptr::null_mut(),
                chk_battery: std::ptr::null_mut(),
                chk_startup: std::ptr::null_mut(),
                hwnd_preview: std::ptr::null_mut(),
                preview_player: None,
                preview_path: None,
                lbl_status: std::ptr::null_mut(),
                last_error: load_warning,
                last_status_text: String::new(),
                tray_added: false,
            });
        }
    });

    // SAFETY: GetModuleHandleW(null) retrieves the current process module handle.
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    register_preview_surface_class(hinstance);

    // SAFETY: LoadIconW and LoadCursorW load standard system application icon and arrow cursor.
    let (hicon, hcursor) = unsafe {
        (
            LoadIconW(std::ptr::null_mut(), IDI_APPLICATION),
            LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        )
    };

    let wnd_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: hicon,
        hCursor: hcursor,
        hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_wide.as_ptr(),
    };

    // SAFETY: RegisterClassW registers the main window class for this process.
    unsafe { RegisterClassW(&wnd_class) };

    let title = wide_string(&format!("Auralis Live Wallpaper Studio v{VERSION}"));
    // WS_CLIPCHILDREN prevents the parent window from erasing over child buttons/labels/preview on repaint.
    let mut style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    if !start_minimized {
        style |= WS_VISIBLE;
    }

    // SAFETY: CreateWindowExW creates the main configuration window and triggers WM_CREATE.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_wide.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale_px(920, scale),
            scale_px(535, scale),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    };

    if hwnd.is_null() {
        return Err("Failed to create Auralis Configuration Studio window".to_string());
    }

    // SAFETY: Standard Win32 message pump driving UI events, timers, and Media Foundation EVR updates.
    unsafe {
        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // Clean shutdown when the message loop exits
    GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(mut state) = guard.take() {
                if let Some(mut preview) = state.preview_player.take() {
                    preview.shutdown();
                }
                let _ = state.engine.config.save_to_file(&state.config_path);
                state.engine.shutdown();
                if !state.ui_font.is_null() {
                    // SAFETY: DeleteObject frees the GDI font handle created in run_windows_gui.
                    unsafe {
                        DeleteObject(state.ui_font);
                    }
                }
            }
        }
    });

    Ok(())
}

/// Register the Auralis system tray icon (`Shell_NotifyIconW`) for background operation.
fn add_tray_icon(hwnd: HWND) {
    // SAFETY: NOTIFYICONDATAW is a plain C struct safe to zero-initialize.
    let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ICON_ID;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_AURALIS_TRAY_ICON;
    // SAFETY: LoadIconW retrieves the standard system application icon.
    nid.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };

    let tip = wide_string(&format!(
        "Auralis Live Wallpaper Studio v{VERSION} (Right-click for controls)"
    ));
    let copy_len = tip.len().min(nid.szTip.len() - 1);
    nid.szTip[..copy_len].copy_from_slice(&tip[..copy_len]);

    // SAFETY: Shell_NotifyIconW(NIM_ADD) registers the tray icon for `hwnd`.
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) } != 0;
    GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(state) = guard.as_mut() {
                state.tray_added = added;
            }
        }
    });
}

/// Remove the Auralis system tray icon on exit.
fn remove_tray_icon(hwnd: HWND) {
    // SAFETY: NOTIFYICONDATAW is a plain C struct safe to zero-initialize.
    let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ICON_ID;
    // SAFETY: Shell_NotifyIconW(NIM_DELETE) removes the tray icon registered for `hwnd`.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// Display the right-click context menu for the system tray icon.
fn show_tray_context_menu(hwnd: HWND) {
    // SAFETY: CreatePopupMenu allocates a native Win32 popup menu handle.
    let hmenu = unsafe { CreatePopupMenu() };
    if hmenu.is_null() {
        return;
    }

    let lbl_open = wide_string("Open Auralis Studio");
    let lbl_pause = wide_string("Pause / Resume Wallpaper");
    let lbl_mute = wide_string("Mute / Unmute Audio");
    let lbl_stop = wide_string("Stop Wallpaper");
    let lbl_quit = wide_string("Quit Auralis && Restore Desktop");

    // SAFETY: AppendMenuW populates the valid popup menu with command items and separators.
    unsafe {
        AppendMenuW(hmenu, MF_STRING, IDM_TRAY_OPEN, lbl_open.as_ptr());
        AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(hmenu, MF_STRING, IDM_TRAY_PAUSE_RESUME, lbl_pause.as_ptr());
        AppendMenuW(hmenu, MF_STRING, IDM_TRAY_MUTE_UNMUTE, lbl_mute.as_ptr());
        AppendMenuW(hmenu, MF_STRING, IDM_TRAY_STOP, lbl_stop.as_ptr());
        AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(hmenu, MF_STRING, IDM_TRAY_QUIT, lbl_quit.as_ptr());
    }

    // SAFETY: POINT is a plain C struct safe to zero-initialize.
    let mut pt: POINT = unsafe { mem::zeroed() };
    // SAFETY: GetCursorPos retrieves current screen coordinates; TrackPopupMenu displays the modal menu.
    let cmd = unsafe {
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        TrackPopupMenu(
            hmenu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD,
            pt.x,
            pt.y,
            0,
            hwnd,
            std::ptr::null(),
        )
    };
    // SAFETY: DestroyMenu frees the popup menu handle after TrackPopupMenu returns.
    unsafe {
        DestroyMenu(hmenu);
    }

    match cmd as usize {
        IDM_TRAY_OPEN => {
            // SAFETY: Restores and focuses the main Studio window.
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
            update_status_readout();
        }
        IDM_TRAY_PAUSE_RESUME => {
            handle_command(hwnd, ID_BTN_PAUSE_RESUME, 0);
        }
        IDM_TRAY_MUTE_UNMUTE => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let display_id = selected_display_id(state);
                        let (cur_muted, cur_vol) = state
                            .engine
                            .controller(&display_id)
                            .map_or((true, 50), |c| (c.muted, c.volume));
                        let new_muted = !cur_muted;
                        let new_vol = if !new_muted && cur_vol == 0 {
                            50
                        } else {
                            cur_vol
                        };
                        let _ = state
                            .engine
                            .set_display_audio(&display_id, new_muted, new_vol);
                        let _ = state.engine.config.save_to_file(&state.config_path);
                    }
                }
            });
            sync_controls_from_state();
            update_status_readout();
        }
        IDM_TRAY_STOP => {
            handle_command(hwnd, ID_BTN_STOP, 0);
        }
        IDM_TRAY_QUIT => {
            handle_command(hwnd, ID_BTN_QUIT, 0);
        }
        _ => {}
    }
}

// SAFETY: Standard Win32 window procedure callback invoked on the GUI STA thread.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            create_child_controls(hwnd);
            add_tray_icon(hwnd);
            // SAFETY: DragAcceptFiles enables drag-and-drop of video files onto `hwnd`.
            unsafe { DragAcceptFiles(hwnd, 1) };
            sync_controls_from_state();
            update_status_readout();
            // SAFETY: SetTimer schedules periodic engine policy & seamless loop ticks on hwnd.
            unsafe { SetTimer(hwnd, ID_TIMER_TICK, TIMER_INTERVAL_MS, None) };
            0
        }
        WM_AURALIS_SHOW_UI => {
            // SAFETY: Shows and brings the existing main configuration window to the foreground.
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
            update_status_readout();
            0
        }
        WM_AURALIS_RELOAD_CONFIG => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let (new_cfg, warn) = AppConfig::load_or_recover(&state.config_path);
                        state.engine.config = new_cfg;
                        state.last_error = warn;
                        state.engine.restore_saved_wallpapers();
                    }
                }
            });
            sync_controls_from_state();
            update_status_readout();
            0
        }
        WM_AURALIS_TRAY_ICON => {
            let mouse_msg = (lparam as u32) & 0xFFFF;
            match mouse_msg {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    // SAFETY: Restores and focuses the main Studio window when clicking the tray icon.
                    unsafe {
                        ShowWindow(hwnd, SW_SHOW);
                        ShowWindow(hwnd, SW_RESTORE);
                        SetForegroundWindow(hwnd);
                    }
                    update_status_readout();
                }
                WM_RBUTTONUP => {
                    show_tray_context_menu(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_DROPFILES => {
            let hdrop = wparam as HDROP;
            if !hdrop.is_null() {
                let mut file_buf = vec![0u16; 1024];
                // SAFETY: DragQueryFileW retrieves the first dropped file path into file_buf, and DragFinish releases hdrop.
                let copied = unsafe {
                    let c = DragQueryFileW(hdrop, 0, file_buf.as_mut_ptr(), file_buf.len() as u32);
                    DragFinish(hdrop);
                    c
                };
                if copied > 0 {
                    let path_str = string_from_wide(&file_buf);
                    if !path_str.is_empty() {
                        let edt_hwnd = GUI_STATE.with(|cell| {
                            cell.try_borrow()
                                .ok()
                                .and_then(|g| g.as_ref().map(|s| s.edt_path))
                                .unwrap_or(std::ptr::null_mut())
                        });
                        if !edt_hwnd.is_null() {
                            let wide = wide_string(&path_str);
                            // SAFETY: Populates the EDIT control with the dropped file path outside any borrow.
                            unsafe { SetWindowTextW(edt_hwnd, wide.as_ptr()) };
                        }
                        start_in_app_preview(PathBuf::from(path_str));
                        update_status_readout();
                    }
                }
            }
            0
        }
        WM_DISPLAYCHANGE => {
            let mut combo_hwnd = std::ptr::null_mut();
            let mut labels = Vec::new();
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let _ = state
                            .engine
                            .handle_event(PlatformEvent::DisplayConfigurationChanged);
                        combo_hwnd = state.cmb_display;
                        labels = build_display_combo_labels(state);
                    }
                }
            });
            apply_display_combo_labels(combo_hwnd, &labels);
            sync_controls_from_state();
            update_status_readout();
            0
        }
        WM_POWERBROADCAST => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        match wparam as u32 {
                            PBT_APMSUSPEND => {
                                let _ =
                                    state.engine.handle_event(PlatformEvent::SystemSleepEntered);
                            }
                            PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                                let _ = state.engine.handle_event(PlatformEvent::SystemWoke);
                            }
                            PBT_APMPOWERSTATUSCHANGE => {
                                let _ = state.engine.evaluate_policy();
                            }
                            _ => {}
                        }
                    }
                }
            });
            update_status_readout();
            1
        }
        WM_TIMER => {
            if wparam == ID_TIMER_TICK {
                let ticked = GUI_STATE.with(|cell| {
                    if let Ok(mut guard) = cell.try_borrow_mut() {
                        if let Some(state) = guard.as_mut() {
                            let _ = state.engine.tick();
                            if let Some(preview) = &mut state.preview_player {
                                let _ = preview.tick(true);
                            }
                            return true;
                        }
                    }
                    false
                });
                if ticked {
                    update_status_readout();
                }
            }
            0
        }
        WM_COMMAND => {
            let control_id = wparam & 0xFFFF;
            let notify_code = (wparam >> 16) & 0xFFFF;
            handle_command(hwnd, control_id, notify_code);
            0
        }
        WM_CLOSE => {
            // AGENTS.md Principle 9: Closing the configuration window must not unexpectedly
            // stop an active wallpaper. If a wallpaper is playing, stop the in-app preview to save
            // resources and hide the window to the system tray; otherwise close and exit cleanly.
            stop_in_app_preview();
            let has_active = GUI_STATE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .and_then(|g| g.as_ref().map(|s| s.engine.has_active_wallpaper()))
                    .unwrap_or(false)
            });

            if has_active {
                // SAFETY: Hides hwnd while keeping the Win32 message loop, tray icon, and wallpaper running.
                unsafe { ShowWindow(hwnd, SW_HIDE) };
            } else {
                // SAFETY: Destroys hwnd and triggers WM_DESTROY to exit the application.
                unsafe { DestroyWindow(hwnd) };
            }
            0
        }
        WM_DESTROY => {
            remove_tray_icon(hwnd);
            // SAFETY: Stops the periodic timer and posts WM_QUIT to exit the message loop.
            unsafe {
                KillTimer(hwnd, ID_TIMER_TICK);
                PostQuitMessage(0);
            }
            0
        }
        // SAFETY: DefWindowProcW handles all standard unprocessed window messages.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn build_display_combo_labels(state: &GuiState) -> Vec<String> {
    state
        .engine
        .displays()
        .iter()
        .map(|d| {
            let tag = if d.is_primary { " [Primary]" } else { "" };
            format!("{} — {} ({}x{}){}", d.id, d.name, d.width, d.height, tag)
        })
        .collect()
}

fn apply_display_combo_labels(cmb_display: HWND, labels: &[String]) {
    if cmb_display.is_null() {
        return;
    }
    // SAFETY: Clears existing items in the valid combo box control outside any RefCell borrow.
    unsafe {
        SendMessageW(cmb_display, CB_RESETCONTENT, 0, 0);
    }
    for label in labels {
        let wide = wide_string(label);
        // SAFETY: Appends null-terminated UTF-16 display description to the combo box.
        unsafe {
            SendMessageW(cmb_display, CB_ADDSTRING, 0, wide.as_ptr() as LPARAM);
        }
    }
    // SAFETY: Selects the first display item in the combo box.
    unsafe {
        SendMessageW(cmb_display, CB_SETCURSEL, 0, 0);
    }
}

fn create_child_controls(parent: HWND) {
    let (_, scale) = query_dpi_scale();
    // SAFETY: GetModuleHandleW retrieves the current process module handle.
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let hfont = GUI_STATE.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|g| g.as_ref().map(|s| s.ui_font))
            .filter(|f| !f.is_null())
            // SAFETY: GetStockObject(DEFAULT_GUI_FONT) returns the system stock GUI font handle.
            .unwrap_or_else(|| unsafe { GetStockObject(DEFAULT_GUI_FONT) })
    });

    let mk_ctrl =
        |class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: usize| -> HWND {
            let cls = wide_string(class);
            let txt = wide_string(text);
            // SAFETY: Creates a DPI-scaled child Win32 control inside `parent` and applies Segoe UI font.
            unsafe {
                let ctrl = CreateWindowExW(
                    0,
                    cls.as_ptr(),
                    txt.as_ptr(),
                    WS_CHILD | WS_VISIBLE | style,
                    scale_px(x, scale),
                    scale_px(y, scale),
                    scale_px(w, scale),
                    scale_px(h, scale),
                    parent,
                    id as *mut _,
                    hinstance,
                    std::ptr::null(),
                );
                if !ctrl.is_null() {
                    SendMessageW(ctrl, WM_SETFONT, hfont as WPARAM, 1);
                }
                ctrl
            }
        };

    mk_ctrl(
        "STATIC",
        "Auralis Live Wallpaper Studio — Native Hardware-Decoded Video Engine (Drag & Drop Video Supported)",
        SS_NOPREFIX,
        20,
        14,
        860,
        24,
        0,
    );

    mk_ctrl("STATIC", "Target Monitor:", SS_NOPREFIX, 20, 48, 115, 24, 0);
    let cmb_display = mk_ctrl(
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
        140,
        44,
        440,
        220,
        ID_CMB_DISPLAY,
    );

    mk_ctrl(
        "STATIC",
        "Video File Path:",
        SS_NOPREFIX,
        20,
        84,
        115,
        24,
        0,
    );
    let edt_path = mk_ctrl(
        "EDIT",
        "",
        WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        140,
        80,
        310,
        28,
        ID_EDT_PATH,
    );
    mk_ctrl(
        "BUTTON",
        "Browse Video...",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        460,
        79,
        120,
        30,
        ID_BTN_BROWSE,
    );

    // Note: Double ampersand `&&` escapes Win32 accelerator prefix on BUTTON controls so `&` displays literally.
    mk_ctrl(
        "BUTTON",
        "Apply && Play Wallpaper",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        20,
        122,
        195,
        36,
        ID_BTN_APPLY,
    );
    mk_ctrl(
        "BUTTON",
        "Pause / Resume",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        225,
        122,
        170,
        36,
        ID_BTN_PAUSE_RESUME,
    );
    mk_ctrl(
        "BUTTON",
        "Stop && Clear",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        405,
        122,
        175,
        36,
        ID_BTN_STOP,
    );

    let chk_mute = mk_ctrl(
        "BUTTON",
        "Mute Audio",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        20,
        172,
        125,
        26,
        ID_CHK_MUTE,
    );
    let chk_loop = mk_ctrl(
        "BUTTON",
        "Seamless Loop",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        155,
        172,
        140,
        26,
        ID_CHK_LOOP,
    );
    mk_ctrl(
        "BUTTON",
        "Vol -",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        310,
        170,
        65,
        28,
        ID_BTN_VOL_DOWN,
    );
    let lbl_volume = mk_ctrl("STATIC", "Volume: 0%", SS_NOPREFIX, 385, 175, 115, 24, 0);
    mk_ctrl(
        "BUTTON",
        "Vol +",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        515,
        170,
        65,
        28,
        ID_BTN_VOL_UP,
    );

    let chk_fullscreen = mk_ctrl(
        "BUTTON",
        "Pause wallpaper when a Full-Screen App or Game is active",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        20,
        206,
        560,
        24,
        ID_CHK_FULLSCREEN,
    );
    let chk_battery = mk_ctrl(
        "BUTTON",
        "Pause wallpaper on Battery Power / Battery Saver",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        20,
        234,
        560,
        24,
        ID_CHK_BATTERY,
    );
    let chk_startup = mk_ctrl(
        "BUTTON",
        "Launch Auralis in System Tray when Windows starts (HKCU\\...\\Run)",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        20,
        262,
        560,
        24,
        ID_CHK_STARTUP,
    );

    // Right column: In-App Video Preview Panel (AGENTS.md Section 1)
    mk_ctrl(
        "STATIC",
        "In-App Video Preview (16:9):",
        SS_NOPREFIX,
        600,
        48,
        280,
        22,
        0,
    );
    let hwnd_preview = mk_ctrl(
        PREVIEW_CLASS_NAME,
        "",
        WS_BORDER | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        600,
        74,
        280,
        158,
        0,
    );
    mk_ctrl(
        "BUTTON",
        "Preview in App",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        600,
        240,
        135,
        32,
        ID_BTN_PREVIEW,
    );
    mk_ctrl(
        "BUTTON",
        "Stop Preview",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        745,
        240,
        135,
        32,
        ID_BTN_STOP_PREVIEW,
    );

    let lbl_status = mk_ctrl(
        "STATIC",
        "Initializing engine telemetry...",
        WS_BORDER | SS_NOPREFIX,
        20,
        298,
        860,
        118,
        0,
    );

    mk_ctrl(
        "BUTTON",
        "Minimize to System Tray (Keep Playing)",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        20,
        430,
        415,
        36,
        ID_BTN_HIDE,
    );
    mk_ctrl(
        "BUTTON",
        "Quit Auralis && Restore Desktop",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        465,
        430,
        415,
        36,
        ID_BTN_QUIT,
    );

    let mut labels = Vec::new();
    GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(state) = guard.as_mut() {
                state.cmb_display = cmb_display;
                state.edt_path = edt_path;
                state.chk_mute = chk_mute;
                state.chk_loop = chk_loop;
                state.lbl_volume = lbl_volume;
                state.chk_fullscreen = chk_fullscreen;
                state.chk_battery = chk_battery;
                state.chk_startup = chk_startup;
                state.hwnd_preview = hwnd_preview;
                state.lbl_status = lbl_status;
                labels = build_display_combo_labels(state);
            }
        }
    });
    apply_display_combo_labels(cmb_display, &labels);
}

fn start_in_app_preview(path: PathBuf) {
    let (hwnd_preview, existing_player) = GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(state) = guard.as_mut() {
                return (state.hwnd_preview, state.preview_player.take());
            }
        }
        (std::ptr::null_mut(), None)
    });
    if hwnd_preview.is_null() {
        return;
    }

    // Load or create the Media Foundation preview player OUTSIDE any GUI_STATE RefCell borrow
    // so message pumping during topology setup can never conflict with GUI_STATE.
    let result = if let Some(mut player) = existing_player {
        match player.load_and_play(&path, true, 0) {
            Ok(()) => Ok(player),
            Err(e) => {
                // Keep the previous preview player alive if loading the new preview failed
                GUI_STATE.with(|cell| {
                    if let Ok(mut guard) = cell.try_borrow_mut() {
                        if let Some(state) = guard.as_mut() {
                            state.preview_player = Some(player);
                        }
                    }
                });
                Err(e)
            }
        }
    } else {
        MfVideoPlayer::create_and_play(hwnd_preview as isize, &path, true, 0)
    };

    GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(state) = guard.as_mut() {
                match result {
                    Ok(active_preview) => {
                        state.preview_player = Some(active_preview);
                        state.preview_path = Some(path);
                        state.last_error = None;
                    }
                    Err(e) => {
                        state.last_error = Some(format!("In-app preview failed: {e}"));
                    }
                }
            }
        }
    });
}

fn stop_in_app_preview() {
    let old_preview = GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(state) = guard.as_mut() {
                state.preview_path = None;
                return state.preview_player.take();
            }
        }
        None
    });
    if let Some(mut preview) = old_preview {
        preview.shutdown();
    }
}

fn selected_display_id(state: &GuiState) -> String {
    // SAFETY: Queries the selected item index from the valid combo box control handle.
    let idx = unsafe { SendMessageW(state.cmb_display, CB_GETCURSEL, 0, 0) };
    let index = if idx >= 0 { idx as usize } else { 0 };
    state
        .engine
        .displays()
        .get(index)
        .or_else(|| state.engine.displays().first())
        .map(|d| d.id.clone())
        .unwrap_or_else(|| "DISPLAY1".to_string())
}

fn sync_controls_from_state() {
    struct ControlSnapshot {
        edt_path: HWND,
        chk_mute: HWND,
        chk_loop: HWND,
        lbl_volume: HWND,
        chk_fullscreen: HWND,
        chk_battery: HWND,
        chk_startup: HWND,
        media_path: Option<String>,
        muted: bool,
        loop_playback: bool,
        volume: u8,
        pause_on_fullscreen: bool,
        pause_on_battery: bool,
        launch_at_startup: bool,
    }

    let snapshot = GUI_STATE.with(|cell| {
        let Ok(guard) = cell.try_borrow() else {
            return None;
        };
        let state = guard.as_ref()?;
        let display_id = selected_display_id(state);
        let media_path = state
            .engine
            .config
            .per_display
            .get(&display_id)
            .or(state.engine.config.default_wallpaper.as_ref())
            .map(|cfg| cfg.media_path.to_string_lossy().to_string());

        let (muted, loop_playback, volume) = state
            .engine
            .controller(&display_id)
            .map(|c| (c.muted, c.loop_playback, c.volume))
            .unwrap_or((true, true, 0));

        Some(ControlSnapshot {
            edt_path: state.edt_path,
            chk_mute: state.chk_mute,
            chk_loop: state.chk_loop,
            lbl_volume: state.lbl_volume,
            chk_fullscreen: state.chk_fullscreen,
            chk_battery: state.chk_battery,
            chk_startup: state.chk_startup,
            media_path,
            muted,
            loop_playback,
            volume,
            pause_on_fullscreen: state.engine.config.pause_on_fullscreen,
            pause_on_battery: state.engine.config.pause_on_battery,
            launch_at_startup: state.engine.config.launch_at_startup,
        })
    });

    let Some(snap) = snapshot else {
        return;
    };

    // Perform all Win32 SetWindowTextW / SendMessageW calls outside the GUI_STATE borrow
    // so any synchronous child notifications (like EN_CHANGE) never collide with a held borrow.
    if let Some(path_str) = snap.media_path {
        let current_edit = read_edit_text(snap.edt_path);
        if current_edit != path_str {
            let wide_path = wide_string(&path_str);
            // SAFETY: Updates the EDIT box text with a valid null-terminated UTF-16 string.
            unsafe { SetWindowTextW(snap.edt_path, wide_path.as_ptr()) };
        }
    }

    // SAFETY: Updates checkbox states and volume label text on valid child control handles.
    unsafe {
        SendMessageW(
            snap.chk_mute,
            BM_SETCHECK,
            if snap.muted {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
            0,
        );
        SendMessageW(
            snap.chk_loop,
            BM_SETCHECK,
            if snap.loop_playback {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
            0,
        );
        SendMessageW(
            snap.chk_fullscreen,
            BM_SETCHECK,
            if snap.pause_on_fullscreen {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
            0,
        );
        SendMessageW(
            snap.chk_battery,
            BM_SETCHECK,
            if snap.pause_on_battery {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
            0,
        );
        SendMessageW(
            snap.chk_startup,
            BM_SETCHECK,
            if snap.launch_at_startup {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
            0,
        );

        let vol_txt = wide_string(&format!("Volume: {}%", snap.volume));
        SetWindowTextW(snap.lbl_volume, vol_txt.as_ptr());
    }
}

fn read_edit_text(hwnd_edit: HWND) -> String {
    if hwnd_edit.is_null() {
        return String::new();
    }
    // SAFETY: Queries the length of the EDIT box text and copies UTF-16 characters into buffer.
    unsafe {
        let len = GetWindowTextLengthW(hwnd_edit);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len as usize) + 2];
        GetWindowTextW(hwnd_edit, buf.as_mut_ptr(), (len + 1) as i32);
        string_from_wide(&buf)
    }
}

fn browse_video_file(owner: HWND) -> Option<PathBuf> {
    let mut file_buf = vec![0u16; 1024];
    // Filter string must be double-null terminated with '\0' separating label and pattern
    let filter: Vec<u16> =
        "Supported Video Files (*.mp4;*.mov;*.mkv;*.webm;*.m4v)\0*.mp4;*.mov;*.mkv;*.webm;*.m4v\0All Files (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
    let title = wide_string("Select Live Wallpaper Video");

    // SAFETY: OPENFILENAMEW is a plain C struct initialized with valid pointers for GetOpenFileNameW.
    let mut ofn: OPENFILENAMEW = unsafe { mem::zeroed() };
    ofn.lStructSize = mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = file_buf.as_mut_ptr();
    ofn.nMaxFile = file_buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;

    // SAFETY: Invokes the standard Windows modal file-open dialog with a valid OPENFILENAMEW struct.
    let chosen = unsafe { GetOpenFileNameW(&mut ofn) };
    if chosen != 0 {
        let selected = string_from_wide(&file_buf);
        if !selected.is_empty() {
            return Some(PathBuf::from(selected));
        }
    }
    None
}

fn handle_command(hwnd: HWND, control_id: usize, notify_code: usize) {
    match control_id {
        ID_CMB_DISPLAY => {
            if notify_code == CBN_SELCHANGE as usize {
                sync_controls_from_state();
                update_status_readout();
            }
        }
        ID_BTN_BROWSE => {
            if let Some(chosen) = browse_video_file(hwnd) {
                let edt_hwnd = GUI_STATE.with(|cell| {
                    cell.try_borrow()
                        .ok()
                        .and_then(|g| g.as_ref().map(|s| s.edt_path))
                        .unwrap_or(std::ptr::null_mut())
                });
                if !edt_hwnd.is_null() {
                    let wide = wide_string(&chosen.to_string_lossy());
                    // SAFETY: Sets the selected file path into the valid EDIT control outside any borrow.
                    unsafe { SetWindowTextW(edt_hwnd, wide.as_ptr()) };
                }
                start_in_app_preview(chosen);
                update_status_readout();
            }
        }
        ID_BTN_PREVIEW => {
            let path_opt = GUI_STATE.with(|cell| {
                cell.try_borrow().ok().and_then(|g| {
                    g.as_ref().and_then(|state| {
                        let txt = read_edit_text(state.edt_path);
                        if txt.trim().is_empty() {
                            None
                        } else {
                            Some(PathBuf::from(txt.trim()))
                        }
                    })
                })
            });
            if let Some(p) = path_opt {
                start_in_app_preview(p);
            } else {
                GUI_STATE.with(|cell| {
                    if let Ok(mut guard) = cell.try_borrow_mut() {
                        if let Some(state) = guard.as_mut() {
                            state.last_error =
                                Some("Select or enter a video file path to preview.".to_string());
                        }
                    }
                });
            }
            update_status_readout();
        }
        ID_BTN_STOP_PREVIEW => {
            stop_in_app_preview();
            update_status_readout();
        }
        ID_BTN_APPLY => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let display_id = selected_display_id(state);
                        let path_text = read_edit_text(state.edt_path);
                        if path_text.trim().is_empty() {
                            state.last_error =
                                Some("Please select or enter a valid video file path.".to_string());
                            return;
                        }
                        match state
                            .engine
                            .apply_wallpaper(&display_id, PathBuf::from(path_text.trim()))
                        {
                            Ok(()) => {
                                state.last_error = None;
                                let _ = state.engine.config.save_to_file(&state.config_path);
                            }
                            Err(err) => {
                                state.last_error = Some(err);
                            }
                        }
                    }
                }
            });
            sync_controls_from_state();
            update_status_readout();
        }
        ID_BTN_PAUSE_RESUME => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let display_id = selected_display_id(state);
                        let is_playing = state
                            .engine
                            .controller(&display_id)
                            .is_some_and(|c| matches!(c.state, PlaybackState::Playing { .. }));
                        let res = if is_playing {
                            state.engine.pause_display(&display_id)
                        } else {
                            state.engine.resume_display(&display_id)
                        };
                        if let Err(e) = res {
                            state.last_error = Some(e.to_string());
                        } else {
                            state.last_error = None;
                        }
                    }
                }
            });
            update_status_readout();
        }
        ID_BTN_STOP => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let display_id = selected_display_id(state);
                        let _ = state.engine.stop_display(&display_id);
                        state.last_error = None;
                    }
                }
            });
            update_status_readout();
        }
        ID_CHK_MUTE | ID_BTN_VOL_DOWN | ID_BTN_VOL_UP => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let display_id = selected_display_id(state);
                        // SAFETY: Queries the current check state of the mute checkbox control.
                        let checked = unsafe { SendMessageW(state.chk_mute, BM_GETCHECK, 0, 0) }
                            == BST_CHECKED as isize;
                        let cur_vol = state.engine.controller(&display_id).map_or(0, |c| c.volume);
                        let new_vol = match control_id {
                            ID_BTN_VOL_DOWN => cur_vol.saturating_sub(10),
                            ID_BTN_VOL_UP => (cur_vol + 10).min(100),
                            _ => cur_vol,
                        };
                        let new_mute = if control_id == ID_BTN_VOL_UP && new_vol > 0 && checked {
                            false
                        } else {
                            checked
                        };
                        let _ = state
                            .engine
                            .set_display_audio(&display_id, new_mute, new_vol);
                        let _ = state.engine.config.save_to_file(&state.config_path);
                    }
                }
            });
            sync_controls_from_state();
            update_status_readout();
        }
        ID_CHK_LOOP => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let display_id = selected_display_id(state);
                        // SAFETY: Queries the current check state of the loop checkbox control.
                        let checked = unsafe { SendMessageW(state.chk_loop, BM_GETCHECK, 0, 0) }
                            == BST_CHECKED as isize;
                        state.engine.set_display_loop(&display_id, checked);
                        let _ = state.engine.config.save_to_file(&state.config_path);
                    }
                }
            });
            update_status_readout();
        }
        ID_CHK_FULLSCREEN | ID_CHK_BATTERY => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        // SAFETY: Queries the current check state of the full-screen policy checkbox.
                        let fs_checked =
                            unsafe { SendMessageW(state.chk_fullscreen, BM_GETCHECK, 0, 0) }
                                == BST_CHECKED as isize;
                        // SAFETY: Queries the current check state of the battery policy checkbox.
                        let bat_checked =
                            unsafe { SendMessageW(state.chk_battery, BM_GETCHECK, 0, 0) }
                                == BST_CHECKED as isize;
                        state.engine.config.pause_on_fullscreen = fs_checked;
                        state.engine.config.pause_on_battery = bat_checked;
                        let _ = state.engine.config.save_to_file(&state.config_path);
                        let _ = state.engine.evaluate_policy();
                    }
                }
            });
            update_status_readout();
        }
        ID_CHK_STARTUP => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        // SAFETY: Queries the current check state of the Windows login startup checkbox.
                        let startup_checked =
                            unsafe { SendMessageW(state.chk_startup, BM_GETCHECK, 0, 0) }
                                == BST_CHECKED as isize;
                        match state.engine.set_launch_at_startup(startup_checked) {
                            Ok(()) => {
                                state.last_error = None;
                                let _ = state.engine.config.save_to_file(&state.config_path);
                            }
                            Err(e) => {
                                state.last_error =
                                    Some(format!("Startup registration failed: {e}"));
                            }
                        }
                    }
                }
            });
            sync_controls_from_state();
            update_status_readout();
        }
        ID_BTN_HIDE => {
            stop_in_app_preview();
            // SAFETY: Hides the main configuration window to the system tray while keeping playback active.
            unsafe { ShowWindow(hwnd, SW_HIDE) };
        }
        ID_BTN_QUIT => {
            stop_in_app_preview();
            // SAFETY: Destroys the main configuration window to trigger clean shutdown.
            unsafe { DestroyWindow(hwnd) };
        }
        _ => {}
    }
}

fn format_mm_ss(ms: u64) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins:02}:{secs:02}")
}

fn update_status_readout() {
    let update_target: Option<(HWND, String)> = GUI_STATE.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return None;
        };
        let state = guard.as_mut()?;

        let display_id = selected_display_id(state);
        let state_desc = match state.engine.controller(&display_id).map(|c| &c.state) {
            Some(PlaybackState::Playing { path, .. }) => {
                format!("PLAYING ({})", path.display())
            }
            Some(PlaybackState::Paused { path, .. }) => {
                format!("PAUSED ({})", path.display())
            }
            Some(PlaybackState::Stopped { .. }) => "STOPPED".to_string(),
            Some(PlaybackState::Loading { path }) => {
                format!("LOADING ({})", path.display())
            }
            Some(PlaybackState::Failed { error, .. }) => format!("FAILED: {error}"),
            _ => "IDLE (Select or drop a video file and click 'Apply & Play Wallpaper')"
                .to_string(),
        };

        let hwnd_desc = state
            .engine
            .host_window(&display_id)
            .map_or_else(|| "Detached".to_string(), |h| format!("WorkerW HWND 0x{h:X}"));

        let video_meta = if let Some(tel) = state.engine.video_telemetry(&display_id) {
            let res_str = tel
                .native_resolution
                .map_or_else(|| "Auto".to_string(), |(w, h)| format!("{w}x{h}"));
            let time_str = match (tel.position_ms, tel.duration_ms) {
                (Some(pos), Some(dur)) if dur > 0 => {
                    format!("{} / {}", format_mm_ss(pos), format_mm_ss(dur))
                }
                _ => "Live".to_string(),
            };
            format!("Video: {res_str} ({time_str})")
        } else {
            "Video: Inactive".to_string()
        };

        let power_desc = match state.engine.query_power_status() {
            Ok(PowerStatus::OnAcPower) => "AC Mains".to_string(),
            Ok(PowerStatus::OnBattery {
                percent,
                battery_saver,
            }) => {
                let pct = percent.map_or_else(|| "?%".to_string(), |p| format!("{p}%"));
                format!("Battery ({pct}, Saver: {battery_saver})")
            }
            _ => "Unknown".to_string(),
        };

        let perf_desc = if let Some(metrics) = state.engine.process_metrics() {
            let ram_mb = (metrics.working_set_bytes as f64) / (1024.0 * 1024.0);
            let cpu_sec = (metrics.cpu_time_ms as f64) / 1000.0;
            format!("RAM: {ram_mb:.1} MB | CPU Time: {cpu_sec:.2}s")
        } else {
            "RAM: N/A".to_string()
        };

        let preview_desc = if let Some(p) = &state.preview_path {
            let fname = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "active".to_string());
            format!("Previewing '{fname}'")
        } else {
            "Preview Idle".to_string()
        };

        let tray_hint = if state.tray_added {
            "System Tray Icon Active (Right-click tray icon for quick controls)."
        } else {
            "Closing or hiding this window keeps your active wallpaper playing."
        };

        let status_line = if let Some(err) = &state.last_error {
            format!(
                "Status: {state_desc}\r\nLayering: {hwnd_desc} | {video_meta} | {preview_desc}\r\nPower: {power_desc} | Measured Resource Usage: {perf_desc}\r\nNotice: {err}"
            )
        } else {
            format!(
                "Status: {state_desc}\r\nLayering: {hwnd_desc} | {video_meta} | {preview_desc}\r\nPower: {power_desc} | Measured Resource Usage: {perf_desc}\r\nTip: {tray_hint}"
            )
        };

        if status_line != state.last_status_text {
            state.last_status_text.clone_from(&status_line);
            Some((state.lbl_status, status_line))
        } else {
            None
        }
    });

    if let Some((lbl_hwnd, status_line)) = update_target {
        if !lbl_hwnd.is_null() {
            let wide = wide_string(&status_line);
            // SAFETY: Updates the status STATIC control text outside any RefCell borrow.
            unsafe { SetWindowTextW(lbl_hwnd, wide.as_ptr()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reentrant_gui_state_does_not_panic() {
        // Verify that calling update_status_readout() and sync_controls_from_state()
        // while GUI_STATE is already mutably borrowed never panics (regression test for
        // Win32 WM_TIMER / SetMediaItem message-pump re-entrancy).
        GUI_STATE.with(|cell| {
            let _held_guard = cell.try_borrow_mut().unwrap();
            sync_controls_from_state();
            update_status_readout();
            stop_in_app_preview();
        });
    }
}
