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
use crate::config::{AppConfig, VideoAspectMode};
use crate::engine::WallpaperEngine;
use crate::media::PlaybackState;
use crate::platform::{create_platform_adapter, PowerStatus};
use crate::{PlatformEvent, VERSION};
use std::cell::RefCell;
use std::mem;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawFocusRect, DrawTextW,
    EndPaint, FillRect, GetDC, GetDeviceCaps, GetStockObject, LineTo, MoveToEx, ReleaseDC,
    RoundRect, SelectObject, SetBkColor, SetBkMode, SetTextColor, BLACK_BRUSH, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_GUI_FONT, DEFAULT_PITCH, DT_CENTER, DT_LEFT,
    DT_SINGLELINE, DT_VCENTER, FF_SWISS, FW_BOLD, FW_NORMAL, HBRUSH, HFONT, HGDIOBJ, HPEN,
    LOGPIXELSY, OUT_TT_PRECIS, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_FOCUS, ODS_HOTLIGHT, ODS_SELECTED,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::Shell::{
    DragAcceptFiles, DragFinish, DragQueryFileW, Shell_NotifyIconW, HDROP, NIF_ICON, NIF_MESSAGE,
    NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, GetWindowTextLengthW, GetWindowTextW,
    KillTimer, LoadCursorW, LoadIconW, PostQuitMessage, RegisterClassW, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowTextW, ShowWindow, SystemParametersInfoW,
    TrackPopupMenu, TranslateMessage, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_OWNERDRAW,
    BS_PUSHBUTTON, CBN_SELCHANGE, CBS_DROPDOWNLIST, CBS_HASSTRINGS, CBS_OWNERDRAWFIXED,
    CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, ES_AUTOHSCROLL, IDC_ARROW, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG,
    PBT_APMPOWERSTATUSCHANGE, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
    SPI_GETCLIENTAREAANIMATION, SW_HIDE, SW_RESTORE, SW_SHOW, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
    TPM_RETURNCMD, WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DISPLAYCHANGE, WM_DRAWITEM, WM_DROPFILES,
    WM_ERASEBKGND, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_MEASUREITEM, WM_PAINT, WM_POWERBROADCAST,
    WM_RBUTTONUP, WM_SETFONT, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_TRANSPARENT, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE,
};

pub const WINDOW_CLASS_NAME: &str = "AuralisConfigStudioWindowClass";
const PREVIEW_CLASS_NAME: &str = "AuralisPreviewSurfaceWindowClass";

const WM_AURALIS_SHOW_UI: u32 = WM_APP + 1;
const WM_AURALIS_RELOAD_CONFIG: u32 = WM_APP + 2;
const WM_AURALIS_TRAY_ICON: u32 = WM_APP + 3;

/// `SS_NOPREFIX` prevents `STATIC` controls from interpreting `&` as an underline accelerator.
const SS_NOPREFIX: u32 = 0x0080;
const SS_CENTER: u32 = 0x0001;
const EN_CHANGE: usize = 0x0300;

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
const ID_BTN_SPEED_DOWN: usize = 117;
const ID_BTN_SPEED_UP: usize = 118;
const ID_BTN_ASPECT: usize = 119;
const ID_BTN_NEXT_RECENT: usize = 120;
const ID_CHK_REDUCE_MOTION: usize = 121;

const TRAY_ICON_ID: u32 = 1;
const IDM_TRAY_OPEN: usize = 2001;
const IDM_TRAY_PAUSE_RESUME: usize = 2002;
const IDM_TRAY_MUTE_UNMUTE: usize = 2003;
const IDM_TRAY_STOP: usize = 2004;
const IDM_TRAY_QUIT: usize = 2005;
const IDM_TRAY_NEXT_RECENT: usize = 2006;

const ID_TIMER_TICK: usize = 1;
/// 100ms tick interval ensures pre-EOS seamless loop seeking and responsive policy updates.
const TIMER_INTERVAL_MS: u32 = 100;

const BST_CHECKED: usize = 1;
const BST_UNCHECKED: usize = 0;

struct GuiState {
    engine: WallpaperEngine,
    config_path: PathBuf,
    ui_font: HFONT,
    section_font: HFONT,
    heading_font: HFONT,
    background_brush: HBRUSH,
    card_brush: HBRUSH,
    input_brush: HBRUSH,
    border_pen: HPEN,
    cmb_display: HWND,
    edt_path: HWND,
    btn_apply: HWND,
    btn_next_recent: HWND,
    btn_stop: HWND,
    btn_preview: HWND,
    btn_stop_preview: HWND,
    btn_vol_down: HWND,
    btn_vol_up: HWND,
    btn_speed_down: HWND,
    btn_speed_up: HWND,
    chk_mute: HWND,
    chk_loop: HWND,
    lbl_volume: HWND,
    lbl_speed: HWND,
    btn_pause_resume: HWND,
    btn_aspect: HWND,
    chk_fullscreen: HWND,
    chk_battery: HWND,
    chk_startup: HWND,
    chk_reduce_motion: HWND,
    hwnd_preview: HWND,
    lbl_preview_empty: HWND,
    preview_player: Option<MfVideoPlayer>,
    preview_path: Option<PathBuf>,
    lbl_status: HWND,
    lbl_status_details: HWND,
    last_error: Option<String>,
    last_status_text: String,
    tray_added: bool,
    tick_counter: u32,
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

const fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
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
    engine.trim_working_set();

    let (dpi, scale) = query_dpi_scale();
    let font_height = -((11 * dpi) / 72);
    let section_font_height = -((11 * dpi) / 72);
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
    // SAFETY: CreateFontW creates a semibold section-label font at the current system DPI.
    let section_font = unsafe {
        CreateFontW(
            section_font_height,
            0,
            0,
            0,
            FW_BOLD as i32,
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
    // SAFETY: Creates a larger, bold Segoe UI font for the Auralis wordmark.
    let heading_font = unsafe {
        CreateFontW(
            -((17 * dpi) / 72),
            0,
            0,
            0,
            FW_BOLD as i32,
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
    // SAFETY: Theme brushes and border pen are owned GDI objects released after the UI closes.
    let (background_brush, card_brush, input_brush, border_pen) = unsafe {
        (
            CreateSolidBrush(rgb(13, 13, 14)),
            CreateSolidBrush(rgb(23, 23, 24)),
            CreateSolidBrush(rgb(30, 30, 32)),
            CreatePen(0, 1, rgb(48, 48, 50)),
        )
    };

    GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            *guard = Some(GuiState {
                engine,
                config_path,
                ui_font,
                section_font,
                heading_font,
                background_brush,
                card_brush,
                input_brush,
                border_pen,
                cmb_display: std::ptr::null_mut(),
                edt_path: std::ptr::null_mut(),
                btn_apply: std::ptr::null_mut(),
                btn_next_recent: std::ptr::null_mut(),
                btn_stop: std::ptr::null_mut(),
                btn_preview: std::ptr::null_mut(),
                btn_stop_preview: std::ptr::null_mut(),
                btn_vol_down: std::ptr::null_mut(),
                btn_vol_up: std::ptr::null_mut(),
                btn_speed_down: std::ptr::null_mut(),
                btn_speed_up: std::ptr::null_mut(),
                chk_mute: std::ptr::null_mut(),
                chk_loop: std::ptr::null_mut(),
                lbl_volume: std::ptr::null_mut(),
                lbl_speed: std::ptr::null_mut(),
                btn_pause_resume: std::ptr::null_mut(),
                btn_aspect: std::ptr::null_mut(),
                chk_fullscreen: std::ptr::null_mut(),
                chk_battery: std::ptr::null_mut(),
                chk_startup: std::ptr::null_mut(),
                chk_reduce_motion: std::ptr::null_mut(),
                hwnd_preview: std::ptr::null_mut(),
                lbl_preview_empty: std::ptr::null_mut(),
                preview_player: None,
                preview_path: None,
                lbl_status: std::ptr::null_mut(),
                lbl_status_details: std::ptr::null_mut(),
                last_error: load_warning,
                last_status_text: String::new(),
                tray_added: false,
                tick_counter: 0,
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
        hbrBackground: background_brush,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_wide.as_ptr(),
    };

    // SAFETY: RegisterClassW registers the main window class for this process.
    unsafe { RegisterClassW(&wnd_class) };

    let title = wide_string(&format!("Auralis Studio · v{VERSION}"));
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
            scale_px(1180, scale),
            scale_px(770, scale),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    };

    if hwnd.is_null() {
        return Err("Failed to create Auralis Configuration Studio window".to_string());
    }

    // Honor Windows dark mode for the native title bar when the OS supports the DWM attribute.
    let dark_titlebar = 1i32;
    // SAFETY: DWM reads one BOOL-sized value; unsupported Windows versions return an error harmlessly.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            20,
            (&dark_titlebar as *const i32).cast(),
            mem::size_of_val(&dark_titlebar) as u32,
        );
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
                if !state.heading_font.is_null() {
                    // SAFETY: DeleteObject frees the heading font handle created in run_windows_gui.
                    unsafe {
                        DeleteObject(state.heading_font);
                    }
                }
                if !state.section_font.is_null() {
                    // SAFETY: DeleteObject frees the section font created in run_windows_gui.
                    unsafe {
                        DeleteObject(state.section_font);
                    }
                }
                for object in [
                    state.background_brush as HGDIOBJ,
                    state.card_brush as HGDIOBJ,
                    state.input_brush as HGDIOBJ,
                    state.border_pen as HGDIOBJ,
                ] {
                    if !object.is_null() {
                        // SAFETY: Each theme GDI object was created by this window and is no longer selected.
                        unsafe { DeleteObject(object) };
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

/// Respect the Windows accessibility setting that disables nonessential client animations.
fn client_area_animations_enabled() -> bool {
    let mut enabled = 1u32;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL to the correctly sized local value.
    let succeeded = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut u32).cast(),
            0,
        )
    } != 0;
    !succeeded || enabled != 0
}

fn ui_font_handle() -> HFONT {
    GUI_STATE
        .with(|cell| {
            cell.try_borrow()
                .ok()
                .and_then(|guard| guard.as_ref().map(|state| state.ui_font))
                .filter(|font| !font.is_null())
        })
        // SAFETY: GetStockObject returns a process-owned stock font handle.
        .unwrap_or_else(|| unsafe { GetStockObject(DEFAULT_GUI_FONT) as HFONT })
}

/// Paint the monochrome app canvas and its soft-edged section surfaces.
fn paint_main_window(hwnd: HWND) {
    let mut paint: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = unsafe { mem::zeroed() };
    // SAFETY: BeginPaint/EndPaint delimit this window's WM_PAINT handling.
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    if hdc.is_null() {
        return;
    }

    let (background, card, border, ui_font, heading_font, phase, live) = GUI_STATE
        .with(|cell| {
            cell.try_borrow().ok().and_then(|guard| {
                let state = guard.as_ref()?;
                let display_id = selected_display_id(state);
                let live = state
                    .engine
                    .controller(&display_id)
                    .is_some_and(|controller| {
                        matches!(controller.state, PlaybackState::Playing { .. })
                    });
                Some((
                    state.background_brush,
                    state.card_brush,
                    state.border_pen,
                    state.ui_font,
                    state.heading_font,
                    state.tick_counter % 10,
                    live,
                ))
            })
        })
        .unwrap_or((
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            false,
        ));

    if background.is_null() || card.is_null() || border.is_null() {
        // SAFETY: Completes the paint cycle even if the window is shutting down.
        unsafe { EndPaint(hwnd, &paint) };
        return;
    }

    // SAFETY: Fills only the invalid client region; WS_CLIPCHILDREN keeps child controls untouched.
    unsafe {
        FillRect(hdc, &paint.rcPaint, background);
        let old_brush = SelectObject(hdc, card as HGDIOBJ);
        let old_pen = SelectObject(hdc, border as HGDIOBJ);
        let (_, scale) = query_dpi_scale();
        for (left, top, right, bottom) in [
            (18, 9, 1142, 57),
            (18, 64, 748, 306),
            (758, 64, 1142, 364),
            (18, 320, 748, 468),
            (758, 376, 1142, 510),
            (18, 526, 1142, 656),
            (18, 658, 1142, 714),
        ] {
            RoundRect(
                hdc,
                scale_px(left, scale),
                scale_px(top, scale),
                scale_px(right, scale),
                scale_px(bottom, scale),
                scale_px(14, scale),
                scale_px(14, scale),
            );
        }
        // Fine inset rules align the controls into a consistent card rhythm without adding color.
        for (left, top, right) in [
            (42, 101, 724),
            (782, 101, 1122),
            (42, 359, 724),
            (782, 413, 1122),
            (42, 564, 1122),
        ] {
            MoveToEx(
                hdc,
                scale_px(left, scale),
                scale_px(top, scale),
                std::ptr::null_mut(),
            );
            LineTo(hdc, scale_px(right, scale), scale_px(top, scale));
        }
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);

        // A compact geometric A mark echoes the standalone SVG wordmark.
        let mark_pen = CreatePen(0, scale_px(2, scale).max(1), rgb(232, 232, 234));
        if !mark_pen.is_null() {
            let old_mark_pen = SelectObject(hdc, mark_pen as HGDIOBJ);
            MoveToEx(
                hdc,
                scale_px(24, scale),
                scale_px(45, scale),
                std::ptr::null_mut(),
            );
            LineTo(hdc, scale_px(38, scale), scale_px(16, scale));
            LineTo(hdc, scale_px(52, scale), scale_px(45, scale));
            MoveToEx(
                hdc,
                scale_px(30, scale),
                scale_px(34, scale),
                std::ptr::null_mut(),
            );
            LineTo(hdc, scale_px(46, scale), scale_px(34, scale));
            SelectObject(hdc, old_mark_pen);
            DeleteObject(mark_pen as HGDIOBJ);
        }

        SetBkMode(hdc, TRANSPARENT as i32);
        let old_font = if heading_font.is_null() {
            std::ptr::null_mut()
        } else {
            SelectObject(hdc, heading_font as HGDIOBJ)
        };
        SetTextColor(hdc, rgb(240, 240, 242));
        let mut brand_rect = RECT {
            left: scale_px(68, scale),
            top: scale_px(13, scale),
            right: scale_px(244, scale),
            bottom: scale_px(51, scale),
        };
        let brand = wide_string("AURALIS");
        DrawTextW(
            hdc,
            brand.as_ptr(),
            -1,
            &mut brand_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        if !old_font.is_null() {
            SelectObject(hdc, old_font);
        }
        let old_ui_font = if ui_font.is_null() {
            std::ptr::null_mut()
        } else {
            SelectObject(hdc, ui_font as HGDIOBJ)
        };
        SetTextColor(hdc, rgb(164, 164, 168));
        let mut tagline_rect = RECT {
            left: scale_px(258, scale),
            top: scale_px(16, scale),
            right: scale_px(680, scale),
            bottom: scale_px(50, scale),
        };
        let tagline = wide_string("WALLPAPER STUDIO");
        DrawTextW(
            hdc,
            tagline.as_ptr(),
            -1,
            &mut tagline_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        SetTextColor(hdc, rgb(205, 205, 207));
        let mut engine_rect = RECT {
            left: scale_px(1032, scale),
            top: scale_px(16, scale),
            right: scale_px(1132, scale),
            bottom: scale_px(50, scale),
        };
        let engine_label = wide_string(if live {
            "WALLPAPER LIVE"
        } else {
            "ENGINE READY"
        });
        DrawTextW(
            hdc,
            engine_label.as_ptr(),
            -1,
            &mut engine_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        // Quiet context labels make each panel's purpose legible before the user scans controls.
        SetTextColor(hdc, rgb(133, 133, 137));
        for (left, top, right, bottom, label) in [
            (210, 79, 710, 103, "DISPLAY  ·  VIDEO SOURCE"),
            (889, 79, 1122, 103, "16:9  ·  LOCAL PREVIEW"),
            (164, 337, 700, 361, "AUDIO  ·  LOOP  ·  PLAYBACK"),
            (890, 391, 1122, 415, "AUTOMATIC PAUSE RULES"),
            (195, 542, 600, 566, "PLAYBACK  ·  RESOURCE USAGE"),
        ] {
            let mut rect = RECT {
                left: scale_px(left, scale),
                top: scale_px(top, scale),
                right: scale_px(right, scale),
                bottom: scale_px(bottom, scale),
            };
            let caption = wide_string(label);
            DrawTextW(
                hdc,
                caption.as_ptr(),
                -1,
                &mut rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
        }
        if !old_ui_font.is_null() {
            SelectObject(hdc, old_ui_font);
        }

        // A low-frequency grayscale pulse communicates live playback without distracting color.
        let brightness = if live {
            if phase <= 5 {
                110 + (phase as u8 * 24)
            } else {
                110 + ((10 - phase) as u8 * 24)
            }
        } else {
            92
        };
        let dot_color = rgb(brightness, brightness, brightness);
        let dot_brush = CreateSolidBrush(dot_color);
        let dot_pen = CreatePen(0, 1, dot_color);
        if !dot_brush.is_null() && !dot_pen.is_null() {
            let old_dot_brush = SelectObject(hdc, dot_brush as HGDIOBJ);
            let old_dot_pen = SelectObject(hdc, dot_pen as HGDIOBJ);
            let dot_x = scale_px(1018, scale);
            let dot_y = scale_px(29, scale);
            windows_sys::Win32::Graphics::Gdi::Ellipse(
                hdc,
                dot_x,
                dot_y,
                dot_x + scale_px(9, scale),
                dot_y + scale_px(9, scale),
            );
            SelectObject(hdc, old_dot_brush);
            SelectObject(hdc, old_dot_pen);
        }
        if !dot_brush.is_null() {
            DeleteObject(dot_brush as HGDIOBJ);
        }
        if !dot_pen.is_null() {
            DeleteObject(dot_pen as HGDIOBJ);
        }
        EndPaint(hwnd, &paint);
    }
}

/// Draw the flat, rounded monochrome treatment for action buttons.
unsafe fn draw_action_button(item: &DRAWITEMSTRUCT) {
    let primary = item.CtlID as usize == ID_BTN_APPLY;
    let pressed = item.itemState & ODS_SELECTED != 0;
    let hot = item.itemState & ODS_HOTLIGHT != 0;
    let focused = item.itemState & ODS_FOCUS != 0;
    let disabled = item.itemState & 0x0004 != 0;
    let face = if primary {
        if pressed {
            rgb(195, 195, 197)
        } else if hot {
            rgb(250, 250, 251)
        } else {
            rgb(238, 238, 240)
        }
    } else if pressed {
        rgb(55, 55, 57)
    } else if hot {
        rgb(47, 47, 49)
    } else {
        rgb(35, 35, 37)
    };
    let edge = if primary {
        face
    } else if hot {
        rgb(78, 78, 81)
    } else {
        rgb(60, 60, 62)
    };
    let brush = CreateSolidBrush(face);
    let pen = CreatePen(0, 1, edge);
    if brush.is_null() || pen.is_null() {
        if !brush.is_null() {
            DeleteObject(brush as HGDIOBJ);
        }
        if !pen.is_null() {
            DeleteObject(pen as HGDIOBJ);
        }
        return;
    }

    let old_brush = SelectObject(item.hDC, brush as HGDIOBJ);
    let old_pen = SelectObject(item.hDC, pen as HGDIOBJ);
    RoundRect(
        item.hDC,
        item.rcItem.left,
        item.rcItem.top,
        item.rcItem.right,
        item.rcItem.bottom,
        12,
        12,
    );
    SelectObject(item.hDC, old_brush);
    SelectObject(item.hDC, old_pen);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);

    SetBkMode(item.hDC, TRANSPARENT as i32);
    let font = ui_font_handle();
    let old_font = SelectObject(item.hDC, font as HGDIOBJ);
    SetTextColor(
        item.hDC,
        if disabled {
            rgb(115, 115, 118)
        } else if primary {
            rgb(18, 18, 19)
        } else {
            rgb(232, 232, 234)
        },
    );
    let mut text = vec![0u16; 512];
    let text_len = GetWindowTextW(item.hwndItem, text.as_mut_ptr(), text.len() as i32);
    let mut text_rect = item.rcItem;
    text_rect.left += 8;
    text_rect.right -= 8;
    DrawTextW(
        item.hDC,
        text.as_ptr(),
        text_len,
        &mut text_rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    if !old_font.is_null() {
        SelectObject(item.hDC, old_font);
    }
    if focused {
        let mut focus_rect = item.rcItem;
        focus_rect.left += 5;
        focus_rect.top += 4;
        focus_rect.right -= 5;
        focus_rect.bottom -= 4;
        DrawFocusRect(item.hDC, &focus_rect);
    }
}

/// Paint the owner-drawn display selector in the same graphite palette as the Studio.
unsafe fn draw_display_item(item: &DRAWITEMSTRUCT) {
    let selected = item.itemState & ODS_SELECTED != 0;
    let background = if selected {
        rgb(52, 52, 54)
    } else {
        rgb(30, 30, 32)
    };
    let brush = CreateSolidBrush(background);
    let pen = CreatePen(0, 1, rgb(60, 60, 62));
    if brush.is_null() || pen.is_null() {
        if !brush.is_null() {
            DeleteObject(brush as HGDIOBJ);
        }
        if !pen.is_null() {
            DeleteObject(pen as HGDIOBJ);
        }
        return;
    }

    let old_brush = SelectObject(item.hDC, brush as HGDIOBJ);
    let old_pen = SelectObject(item.hDC, pen as HGDIOBJ);
    RoundRect(
        item.hDC,
        item.rcItem.left,
        item.rcItem.top,
        item.rcItem.right,
        item.rcItem.bottom,
        8,
        8,
    );
    SelectObject(item.hDC, old_brush);
    SelectObject(item.hDC, old_pen);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);

    let mut text = vec![0u16; 512];
    let text_len = if item.itemID == u32::MAX {
        GetWindowTextW(item.hwndItem, text.as_mut_ptr(), text.len() as i32)
    } else {
        let len = SendMessageW(
            item.hwndItem,
            windows_sys::Win32::UI::WindowsAndMessaging::CB_GETLBTEXTLEN,
            item.itemID as WPARAM,
            0,
        );
        if len <= 0 {
            0
        } else {
            SendMessageW(
                item.hwndItem,
                windows_sys::Win32::UI::WindowsAndMessaging::CB_GETLBTEXT,
                item.itemID as WPARAM,
                text.as_mut_ptr() as LPARAM,
            ) as i32
        }
    };
    SetBkMode(item.hDC, TRANSPARENT as i32);
    let font = ui_font_handle();
    let old_font = SelectObject(item.hDC, font as HGDIOBJ);
    SetTextColor(item.hDC, rgb(238, 238, 240));
    let mut text_rect = item.rcItem;
    text_rect.left += 10;
    text_rect.right -= 8;
    DrawTextW(
        item.hDC,
        text.as_ptr(),
        text_len,
        &mut text_rect,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    if !old_font.is_null() {
        SelectObject(item.hDC, old_font);
    }
    if item.itemState & ODS_FOCUS != 0 {
        let mut focus_rect = item.rcItem;
        focus_rect.left += 4;
        focus_rect.top += 2;
        focus_rect.right -= 4;
        focus_rect.bottom -= 2;
        DrawFocusRect(item.hDC, &focus_rect);
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
    let lbl_next = wide_string("Next Recent Wallpaper");
    let lbl_mute = wide_string("Mute / Unmute Audio");
    let lbl_stop = wide_string("Stop Wallpaper");
    let lbl_quit = wide_string("Quit Auralis && Restore Desktop");

    // SAFETY: AppendMenuW populates the valid popup menu with command items and separators.
    unsafe {
        AppendMenuW(hmenu, MF_STRING, IDM_TRAY_OPEN, lbl_open.as_ptr());
        AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(hmenu, MF_STRING, IDM_TRAY_PAUSE_RESUME, lbl_pause.as_ptr());
        AppendMenuW(hmenu, MF_STRING, IDM_TRAY_NEXT_RECENT, lbl_next.as_ptr());
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
        IDM_TRAY_NEXT_RECENT => {
            handle_command(hwnd, ID_BTN_NEXT_RECENT, 0);
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
        WM_ERASEBKGND => {
            let background = GUI_STATE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .and_then(|guard| guard.as_ref().map(|state| state.background_brush))
                    .unwrap_or(std::ptr::null_mut())
            });
            if !background.is_null() {
                let mut client: RECT = unsafe { mem::zeroed() };
                // SAFETY: The paint DC and client rectangle are valid for the receiving window.
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
                    FillRect(wparam as _, &client, background);
                }
                1
            } else {
                0
            }
        }
        WM_PAINT => {
            paint_main_window(hwnd);
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let (brush, preview_empty) = GUI_STATE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .and_then(|guard| {
                        guard
                            .as_ref()
                            .map(|state| (state.card_brush, state.lbl_preview_empty))
                    })
                    .unwrap_or((std::ptr::null_mut(), std::ptr::null_mut()))
            });
            if msg == WM_CTLCOLORSTATIC && lparam as HWND == preview_empty {
                // Keep the preview hint legible without a visible label rectangle over the video.
                unsafe {
                    SetBkMode(wparam as _, 2);
                    SetBkColor(wparam as _, rgb(0, 0, 0));
                    SetTextColor(wparam as _, rgb(168, 168, 171));
                }
                // SAFETY: GetStockObject returns the process-owned black stock brush.
                return unsafe { GetStockObject(BLACK_BRUSH) as LRESULT };
            }
            if !brush.is_null() {
                // SAFETY: wparam is the child control's HDC for this color request.
                unsafe {
                    SetBkMode(wparam as _, TRANSPARENT as i32);
                    SetTextColor(wparam as _, rgb(218, 218, 220));
                }
            }
            brush as LRESULT
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            let brush = GUI_STATE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .and_then(|guard| guard.as_ref().map(|state| state.input_brush))
                    .unwrap_or(std::ptr::null_mut())
            });
            if !brush.is_null() {
                // SAFETY: wparam is the input control's HDC for this color request.
                unsafe {
                    SetBkMode(wparam as _, 2);
                    SetBkColor(wparam as _, rgb(30, 30, 32));
                    SetTextColor(wparam as _, rgb(238, 238, 240));
                }
            }
            brush as LRESULT
        }
        WM_DRAWITEM => {
            if lparam != 0 {
                // SAFETY: WM_DRAWITEM provides a live DRAWITEMSTRUCT pointer in lparam.
                let item = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
                if item.CtlID as usize == ID_CMB_DISPLAY {
                    unsafe { draw_display_item(item) };
                } else {
                    unsafe { draw_action_button(item) };
                }
                1
            } else {
                0
            }
        }
        WM_MEASUREITEM => {
            if lparam != 0 {
                // SAFETY: WM_MEASUREITEM provides a live MEASUREITEMSTRUCT pointer in lparam.
                let item = unsafe { &mut *(lparam as *mut MEASUREITEMSTRUCT) };
                if item.CtlID as usize == ID_CMB_DISPLAY {
                    let (_, scale) = query_dpi_scale();
                    item.itemHeight = scale_px(30, scale) as u32;
                    1
                } else {
                    0
                }
            } else {
                0
            }
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
            stop_in_app_preview();
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let (new_cfg, warn) = AppConfig::load_or_recover(&state.config_path);
                        state.engine.config = new_cfg;
                        state.last_error = warn;
                        state.engine.restore_saved_wallpapers();
                        state.engine.trim_working_set();
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
                let (ticked, animate_live, refresh_status) = GUI_STATE.with(|cell| {
                    if let Ok(mut guard) = cell.try_borrow_mut() {
                        if let Some(state) = guard.as_mut() {
                            let _ = state.engine.tick();
                            if let Some(preview) = &mut state.preview_player {
                                let _ = preview.tick(true);
                            }
                            state.tick_counter = state.tick_counter.wrapping_add(1);
                            if state.tick_counter.is_multiple_of(50)
                                && state.preview_player.is_none()
                            {
                                state.engine.trim_working_set();
                            }
                            let display_id = selected_display_id(state);
                            let live = state
                                .engine
                                .controller(&display_id)
                                .is_some_and(|c| matches!(c.state, PlaybackState::Playing { .. }));
                            let refresh_status = state.tick_counter.is_multiple_of(10);
                            return (
                                true,
                                live && !state.engine.config.reduce_motion,
                                refresh_status,
                            );
                        }
                    }
                    (false, false, false)
                });
                if ticked && refresh_status {
                    update_enabled_controls();
                    update_status_readout();
                }
                if animate_live && client_area_animations_enabled() {
                    let (_, scale) = query_dpi_scale();
                    let header = RECT {
                        left: 0,
                        top: 0,
                        right: scale_px(1148, scale),
                        bottom: scale_px(54, scale),
                    };
                    // SAFETY: Invalidates only the shallow header strip for the subtle playback pulse.
                    unsafe { windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, &header, 0) };
                }
            }
            0
        }
        WM_COMMAND => {
            let control_id = wparam & 0xFFFF;
            let notify_code = (wparam >> 16) & 0xFFFF;
            if control_id == ID_EDT_PATH && notify_code == EN_CHANGE {
                update_enabled_controls();
            } else {
                handle_command(hwnd, control_id, notify_code);
            }
            let (_, scale) = query_dpi_scale();
            let header = RECT {
                left: 0,
                top: 0,
                right: scale_px(1148, scale),
                bottom: scale_px(54, scale),
            };
            // SAFETY: Refreshes the animated playback marker after user commands change state.
            unsafe { windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, &header, 0) };
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
                WindowsPlatformAdapter::trim_process_working_set();
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
    let (hfont, hsection_font) = GUI_STATE.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|g| g.as_ref().map(|s| (s.ui_font, s.section_font)))
            .map(|(ui, section)| {
                let fallback = || unsafe { GetStockObject(DEFAULT_GUI_FONT) };
                (
                    if ui.is_null() { fallback() } else { ui },
                    if section.is_null() {
                        fallback()
                    } else {
                        section
                    },
                )
            })
            .unwrap_or_else(|| unsafe {
                let fallback = GetStockObject(DEFAULT_GUI_FONT);
                (fallback, fallback)
            })
    });
    let mk_ctrl = |class: &str,
                   text: &str,
                   mut style: u32,
                   x: i32,
                   y: i32,
                   w: i32,
                   h: i32,
                   id: usize|
     -> HWND {
        if class == "BUTTON" && style & 0x000F == BS_PUSHBUTTON as u32 {
            style = (style & !(BS_PUSHBUTTON as u32)) | BS_OWNERDRAW as u32;
        }
        let ex_style = if class == "STATIC" || style & 0x000F == BS_AUTOCHECKBOX as u32 {
            WS_EX_TRANSPARENT
        } else {
            0
        };
        let cls = wide_string(class);
        let txt = wide_string(text);
        // SAFETY: Creates a DPI-scaled child Win32 control and applies the application font.
        unsafe {
            let ctrl = CreateWindowExW(
                ex_style,
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
    let mk_section = |text: &str, x: i32, y: i32, w: i32| -> HWND {
        let control = mk_ctrl("STATIC", text, SS_NOPREFIX, x, y, w, 24, 0);
        if !control.is_null() {
            // SAFETY: Applies the long-lived section font to this valid static control.
            unsafe { SendMessageW(control, WM_SETFONT, hsection_font as WPARAM, 1) };
        }
        control
    };

    mk_section("Wallpaper source", 42, 79, 180);
    mk_ctrl("STATIC", "Display", SS_NOPREFIX, 42, 112, 84, 24, 0);
    let cmb_display = mk_ctrl(
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | CBS_OWNERDRAWFIXED as u32 | CBS_HASSTRINGS as u32 | WS_TABSTOP,
        132,
        105,
        574,
        30,
        ID_CMB_DISPLAY,
    );
    mk_ctrl("STATIC", "Video file", SS_NOPREFIX, 42, 153, 84, 24, 0);
    let edt_path = mk_ctrl(
        "EDIT",
        "",
        WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        132,
        146,
        385,
        32,
        ID_EDT_PATH,
    );
    mk_ctrl(
        "BUTTON",
        "Browse",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        527,
        145,
        88,
        34,
        ID_BTN_BROWSE,
    );
    let btn_next_recent = mk_ctrl(
        "BUTTON",
        "Next recent",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        625,
        145,
        82,
        34,
        ID_BTN_NEXT_RECENT,
    );
    let btn_apply = mk_ctrl(
        "BUTTON",
        "Set wallpaper",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        42,
        202,
        210,
        44,
        ID_BTN_APPLY,
    );
    let btn_pause_resume = mk_ctrl(
        "BUTTON",
        "Pause wallpaper",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        264,
        202,
        182,
        44,
        ID_BTN_PAUSE_RESUME,
    );
    let btn_stop = mk_ctrl(
        "BUTTON",
        "Stop",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        458,
        202,
        170,
        44,
        ID_BTN_STOP,
    );

    mk_section("Preview", 782, 79, 96);
    let hwnd_preview = mk_ctrl(
        PREVIEW_CLASS_NAME,
        "",
        WS_BORDER | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        778,
        111,
        344,
        194,
        0,
    );
    let lbl_preview_empty = mk_ctrl(
        "STATIC",
        "Choose a video to preview",
        SS_CENTER | SS_NOPREFIX,
        814,
        194,
        272,
        28,
        0,
    );
    let btn_preview = mk_ctrl(
        "BUTTON",
        "Preview",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        778,
        319,
        164,
        34,
        ID_BTN_PREVIEW,
    );
    let btn_stop_preview = mk_ctrl(
        "BUTTON",
        "Stop preview",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        954,
        319,
        168,
        34,
        ID_BTN_STOP_PREVIEW,
    );

    mk_section("Playback", 42, 337, 116);
    let chk_mute = mk_ctrl(
        "BUTTON",
        "Mute audio",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        42,
        365,
        110,
        26,
        ID_CHK_MUTE,
    );
    let chk_loop = mk_ctrl(
        "BUTTON",
        "Seamless loop",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        168,
        365,
        142,
        26,
        ID_CHK_LOOP,
    );
    let btn_vol_down = mk_ctrl(
        "BUTTON",
        "Vol -",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        332,
        361,
        74,
        32,
        ID_BTN_VOL_DOWN,
    );
    let lbl_volume = mk_ctrl("STATIC", "0% volume", SS_NOPREFIX, 416, 366, 102, 24, 0);
    let btn_vol_up = mk_ctrl(
        "BUTTON",
        "Vol +",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        530,
        361,
        74,
        32,
        ID_BTN_VOL_UP,
    );
    let btn_speed_down = mk_ctrl(
        "BUTTON",
        "Speed -",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        42,
        410,
        82,
        32,
        ID_BTN_SPEED_DOWN,
    );
    let lbl_speed = mk_ctrl("STATIC", "1.00× speed", SS_NOPREFIX, 136, 415, 104, 24, 0);
    let btn_speed_up = mk_ctrl(
        "BUTTON",
        "Speed +",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        250,
        410,
        82,
        32,
        ID_BTN_SPEED_UP,
    );
    let btn_aspect = mk_ctrl(
        "BUTTON",
        "Scale: Fill",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        356,
        410,
        350,
        34,
        ID_BTN_ASPECT,
    );

    mk_section("Behavior", 782, 391, 108);
    let chk_fullscreen = mk_ctrl(
        "BUTTON",
        "Pause in full-screen apps",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        782,
        419,
        336,
        24,
        ID_CHK_FULLSCREEN,
    );
    let chk_battery = mk_ctrl(
        "BUTTON",
        "Pause on battery power",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        782,
        446,
        336,
        24,
        ID_CHK_BATTERY,
    );
    let chk_startup = mk_ctrl(
        "BUTTON",
        "Launch Auralis at sign-in",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        782,
        473,
        336,
        24,
        ID_CHK_STARTUP,
    );

    mk_section("Engine status", 42, 542, 150);
    let lbl_status = mk_ctrl(
        "STATIC",
        "Wallpaper engine starting",
        SS_NOPREFIX,
        42,
        568,
        1074,
        24,
        0,
    );
    let lbl_status_details = mk_ctrl("STATIC", "", SS_NOPREFIX, 42, 592, 1074, 60, 0);
    let chk_reduce_motion = mk_ctrl(
        "BUTTON",
        "Reduce motion",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        42,
        668,
        204,
        26,
        ID_CHK_REDUCE_MOTION,
    );
    mk_ctrl(
        "BUTTON",
        "Hide to tray",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        772,
        662,
        158,
        42,
        ID_BTN_HIDE,
    );
    mk_ctrl(
        "BUTTON",
        "Quit & restore desktop",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        942,
        662,
        200,
        42,
        ID_BTN_QUIT,
    );

    let mut labels = Vec::new();
    GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(state) = guard.as_mut() {
                state.cmb_display = cmb_display;
                state.edt_path = edt_path;
                state.btn_apply = btn_apply;
                state.btn_next_recent = btn_next_recent;
                state.btn_stop = btn_stop;
                state.btn_preview = btn_preview;
                state.btn_stop_preview = btn_stop_preview;
                state.btn_vol_down = btn_vol_down;
                state.btn_vol_up = btn_vol_up;
                state.btn_speed_down = btn_speed_down;
                state.btn_speed_up = btn_speed_up;
                state.chk_mute = chk_mute;
                state.chk_loop = chk_loop;
                state.lbl_volume = lbl_volume;
                state.lbl_speed = lbl_speed;
                state.btn_pause_resume = btn_pause_resume;
                state.btn_aspect = btn_aspect;
                state.chk_fullscreen = chk_fullscreen;
                state.chk_battery = chk_battery;
                state.chk_startup = chk_startup;
                state.chk_reduce_motion = chk_reduce_motion;
                state.hwnd_preview = hwnd_preview;
                state.lbl_preview_empty = lbl_preview_empty;
                state.lbl_status = lbl_status;
                state.lbl_status_details = lbl_status_details;
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
    let (preview_active, empty_state) = GUI_STATE.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|guard| {
                guard
                    .as_ref()
                    .map(|state| (state.preview_player.is_some(), state.lbl_preview_empty))
            })
            .unwrap_or((false, std::ptr::null_mut()))
    });
    if !empty_state.is_null() {
        // SAFETY: Toggles the noninteractive empty-state hint over the live preview surface.
        unsafe { ShowWindow(empty_state, if preview_active { SW_HIDE } else { SW_SHOW }) };
    }
    update_enabled_controls();
}

fn stop_in_app_preview() {
    let (old_preview, empty_state) = GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(state) = guard.as_mut() {
                state.preview_path = None;
                return (state.preview_player.take(), state.lbl_preview_empty);
            }
        }
        (None, std::ptr::null_mut())
    });
    if !empty_state.is_null() {
        // SAFETY: Restores the in-app preview hint after the media player is removed.
        unsafe { ShowWindow(empty_state, SW_SHOW) };
    }
    if let Some(mut preview) = old_preview {
        preview.shutdown();
        WindowsPlatformAdapter::trim_process_working_set();
    }
    update_enabled_controls();
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
        lbl_speed: HWND,
        btn_pause_resume: HWND,
        btn_aspect: HWND,
        chk_fullscreen: HWND,
        chk_battery: HWND,
        chk_startup: HWND,
        chk_reduce_motion: HWND,
        media_path: Option<String>,
        pause_label: &'static str,
        muted: bool,
        loop_playback: bool,
        volume: u8,
        speed_percent: u16,
        aspect_mode: VideoAspectMode,
        pause_on_fullscreen: bool,
        pause_on_battery: bool,
        launch_at_startup: bool,
        reduce_motion: bool,
    }

    let snapshot = GUI_STATE.with(|cell| {
        let Ok(guard) = cell.try_borrow() else {
            return None;
        };
        let state = guard.as_ref()?;
        let display_id = selected_display_id(state);
        let pause_label =
            playback_pause_caption(state.engine.controller(&display_id).map(|c| &c.state));
        let media_path = state
            .engine
            .config
            .per_display
            .get(&display_id)
            .or(state.engine.config.default_wallpaper.as_ref())
            .map(|cfg| cfg.media_path.to_string_lossy().to_string());

        let (muted, loop_playback, volume, speed_percent, aspect_mode) = state
            .engine
            .controller(&display_id)
            .map(|c| {
                (
                    c.muted,
                    c.loop_playback,
                    c.volume,
                    c.speed_percent,
                    c.aspect_mode,
                )
            })
            .unwrap_or((true, true, 0, 100, VideoAspectMode::Fill));

        Some(ControlSnapshot {
            edt_path: state.edt_path,
            chk_mute: state.chk_mute,
            chk_loop: state.chk_loop,
            lbl_volume: state.lbl_volume,
            lbl_speed: state.lbl_speed,
            btn_pause_resume: state.btn_pause_resume,
            btn_aspect: state.btn_aspect,
            chk_fullscreen: state.chk_fullscreen,
            chk_battery: state.chk_battery,
            chk_startup: state.chk_startup,
            chk_reduce_motion: state.chk_reduce_motion,
            media_path,
            pause_label,
            muted,
            loop_playback,
            volume,
            speed_percent,
            aspect_mode,
            pause_on_fullscreen: state.engine.config.pause_on_fullscreen,
            pause_on_battery: state.engine.config.pause_on_battery,
            launch_at_startup: state.engine.config.launch_at_startup,
            reduce_motion: state.engine.config.reduce_motion,
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

    // SAFETY: Updates checkbox states and volume/speed/aspect label text on valid child control handles.
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
        SendMessageW(
            snap.chk_reduce_motion,
            BM_SETCHECK,
            if snap.reduce_motion {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
            0,
        );

        let vol_txt = wide_string(&format!("{}% volume", snap.volume));
        SetWindowTextW(snap.lbl_volume, vol_txt.as_ptr());

        if !snap.lbl_speed.is_null() {
            let rate = f32::from(snap.speed_percent) / 100.0;
            let spd_txt = wide_string(&format!("{rate:.2}× speed"));
            SetWindowTextW(snap.lbl_speed, spd_txt.as_ptr());
        }
        if !snap.btn_pause_resume.is_null() {
            let pause_txt = wide_string(snap.pause_label);
            SetWindowTextW(snap.btn_pause_resume, pause_txt.as_ptr());
        }
        if !snap.btn_aspect.is_null() {
            let asp_txt = wide_string(&format!("Scale: {}", snap.aspect_mode.label()));
            SetWindowTextW(snap.btn_aspect, asp_txt.as_ptr());
        }
    }

    update_enabled_controls();
}

/// Disable actions that have no current target so the Studio does not present inert controls.
fn update_enabled_controls() {
    let controls = GUI_STATE.with(|cell| {
        let Ok(guard) = cell.try_borrow() else {
            return None;
        };
        let state = guard.as_ref()?;
        let display_id = selected_display_id(state);
        let has_wallpaper = playback_controls_enabled(
            state
                .engine
                .controller(&display_id)
                .map(|controller| &controller.state),
        );
        Some((
            state.edt_path,
            [state.btn_apply, state.btn_preview],
            [state.btn_pause_resume, state.btn_stop],
            [
                state.chk_mute,
                state.chk_loop,
                state.lbl_volume,
                state.btn_vol_down,
                state.btn_vol_up,
                state.lbl_speed,
                state.btn_speed_down,
                state.btn_speed_up,
                state.btn_aspect,
            ],
            state.btn_next_recent,
            state.btn_stop_preview,
            !state.engine.config.recent_wallpapers.is_empty(),
            state.preview_player.is_some(),
            has_wallpaper,
        ))
    });

    let Some((
        path_edit,
        path_actions,
        wallpaper_actions,
        playback_controls,
        next_recent,
        stop_preview,
        has_recent,
        has_preview,
        has_wallpaper,
    )) = controls
    else {
        return;
    };
    let has_path = !read_edit_text(path_edit).trim().is_empty();
    // SAFETY: Every handle belongs to a child control created with the Studio window.
    unsafe {
        for control in path_actions {
            EnableWindow(control, i32::from(has_path));
        }
        for control in wallpaper_actions {
            EnableWindow(control, i32::from(has_wallpaper));
        }
        for control in playback_controls {
            EnableWindow(control, i32::from(has_wallpaper));
        }
        EnableWindow(next_recent, i32::from(has_recent));
        EnableWindow(stop_preview, i32::from(has_preview));
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
        ID_BTN_NEXT_RECENT => {
            stop_in_app_preview();
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let display_id = selected_display_id(state);
                        match state.engine.cycle_next_recent_wallpaper(&display_id) {
                            Ok(_) => {
                                state.last_error = None;
                                let _ = state.engine.config.save_to_file(&state.config_path);
                            }
                            Err(e) => {
                                state.last_error = Some(e);
                            }
                        }
                    }
                }
            });
            sync_controls_from_state();
            update_status_readout();
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
            // Stop the in-app preview before applying a wallpaper so its decoder is not left
            // active alongside the wallpaper player's decoder.
            stop_in_app_preview();
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
            WindowsPlatformAdapter::trim_process_working_set();
            sync_controls_from_state();
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
        ID_BTN_SPEED_DOWN | ID_BTN_SPEED_UP => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let display_id = selected_display_id(state);
                        let cur_speed = state
                            .engine
                            .controller(&display_id)
                            .map_or(100, |c| c.speed_percent);
                        let new_speed = if control_id == ID_BTN_SPEED_DOWN {
                            cur_speed.saturating_sub(25).max(25)
                        } else {
                            (cur_speed + 25).min(200)
                        };
                        match state
                            .engine
                            .set_display_playback_speed(&display_id, new_speed)
                        {
                            Ok(()) => {
                                state.last_error = None;
                                let _ = state.engine.config.save_to_file(&state.config_path);
                            }
                            Err(e) => {
                                state.last_error = Some(e.to_string());
                            }
                        }
                    }
                }
            });
            sync_controls_from_state();
            update_status_readout();
        }
        ID_BTN_ASPECT => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        let display_id = selected_display_id(state);
                        let cur_aspect = state
                            .engine
                            .controller(&display_id)
                            .map_or(VideoAspectMode::Fill, |c| c.aspect_mode);
                        let next_aspect = cur_aspect.toggle();
                        match state
                            .engine
                            .set_display_aspect_mode(&display_id, next_aspect)
                        {
                            Ok(()) => {
                                state.last_error = None;
                                let _ = state.engine.config.save_to_file(&state.config_path);
                            }
                            Err(e) => {
                                state.last_error = Some(e.to_string());
                            }
                        }
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
        ID_CHK_REDUCE_MOTION => {
            GUI_STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    if let Some(state) = guard.as_mut() {
                        // SAFETY: Queries the check state of the motion-accessibility checkbox.
                        state.engine.config.reduce_motion =
                            unsafe { SendMessageW(state.chk_reduce_motion, BM_GETCHECK, 0, 0) }
                                == BST_CHECKED as isize;
                        let _ = state.engine.config.save_to_file(&state.config_path);
                    }
                }
            });
        }
        ID_BTN_HIDE => {
            stop_in_app_preview();
            // SAFETY: Hides the main configuration window to the system tray while keeping playback active.
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            WindowsPlatformAdapter::trim_process_working_set();
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

fn playback_status_caption(state: Option<&PlaybackState>) -> String {
    match state {
        Some(PlaybackState::Playing { path, .. }) => format!(
            "Playing · {}",
            path.file_name()
                .map_or_else(|| "Wallpaper".into(), |name| name.to_string_lossy())
        ),
        Some(PlaybackState::Paused { path, .. }) => format!(
            "Paused · {}",
            path.file_name()
                .map_or_else(|| "Wallpaper".into(), |name| name.to_string_lossy())
        ),
        Some(PlaybackState::Stopped { .. }) => "Wallpaper stopped".to_string(),
        Some(PlaybackState::Loading { path }) => format!(
            "Loading · {}",
            path.file_name()
                .map_or_else(|| "Wallpaper".into(), |name| name.to_string_lossy())
        ),
        Some(PlaybackState::Failed { error, .. }) => format!("Playback failed · {error}"),
        Some(PlaybackState::Unloaded) | None => "Ready for a wallpaper".to_string(),
    }
}

fn playback_pause_caption(state: Option<&PlaybackState>) -> &'static str {
    match state {
        Some(PlaybackState::Paused { .. }) => "Resume wallpaper",
        _ => "Pause wallpaper",
    }
}

fn playback_controls_enabled(state: Option<&PlaybackState>) -> bool {
    matches!(
        state,
        Some(
            PlaybackState::Playing { .. }
                | PlaybackState::Paused { .. }
                | PlaybackState::Loading { .. }
        )
    )
}

fn update_status_readout() {
    let update_target: Option<(HWND, String, HWND, String, HWND, &'static str)> = GUI_STATE.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return None;
        };
        let state = guard.as_mut()?;

        let display_id = selected_display_id(state);
        let playback_state = state.engine.controller(&display_id).map(|c| &c.state);
        let state_desc = playback_status_caption(playback_state);

        let host_desc = if state.engine.host_window(&display_id).is_some() {
            "Desktop surface ready"
        } else {
            "Desktop surface waiting"
        };

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
            format!("{res_str} · {time_str}")
        } else {
            "Video inactive".to_string()
        };

        let power_desc = match state.engine.query_power_status() {
            Ok(PowerStatus::OnAcPower) => "AC power".to_string(),
            Ok(PowerStatus::OnBattery {
                percent,
                battery_saver,
            }) => {
                let pct = percent.map_or_else(|| "?%".to_string(), |p| format!("{p}%"));
                if battery_saver {
                    format!("Battery {pct} · saver on")
                } else {
                    format!("Battery {pct}")
                }
            }
            _ => "Unknown".to_string(),
        };

        let perf_desc = if let Some(metrics) = state.engine.process_metrics() {
            let ws_mb = (metrics.working_set_bytes as f64) / (1024.0 * 1024.0);
            let priv_mb = (metrics.private_bytes as f64) / (1024.0 * 1024.0);
            let cpu_sec = (metrics.cpu_time_ms as f64) / 1000.0;
            format!("Memory {ws_mb:.1} MB resident · {priv_mb:.1} MB private · CPU {cpu_sec:.0}s")
        } else {
            "Resource metrics unavailable".to_string()
        };

        let preview_desc = if let Some(p) = &state.preview_path {
            let fname = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "active".to_string());
            format!("Preview active · {fname}")
        } else {
            "Preview stopped".to_string()
        };

        let recent_count = state.engine.config.recent_wallpapers.len();
        let tray_hint = if state.tray_added {
            format!("System tray ready · {recent_count} recent wallpapers")
        } else {
            format!("Closing this window keeps playback active · {recent_count} recent wallpapers")
        };

        let details = if let Some(err) = &state.last_error {
            format!(
                "{host_desc} · {video_meta} · {preview_desc}\r\n{perf_desc} · {power_desc}\r\nNotice · {err}"
            )
        } else {
            format!(
                "{host_desc} · {video_meta} · {preview_desc}\r\n{perf_desc} · {power_desc}\r\n{tray_hint}"
            )
        };
        let status_line = format!("{state_desc}\r\n{details}");

        if status_line != state.last_status_text {
            state.last_status_text.clone_from(&status_line);
            let pause_label = playback_pause_caption(playback_state);
            Some((
                state.lbl_status,
                state_desc,
                state.lbl_status_details,
                details,
                state.btn_pause_resume,
                pause_label,
            ))
        } else {
            None
        }
    });

    if let Some((lbl_hwnd, status_line, details_hwnd, details, pause_button, pause_label)) =
        update_target
    {
        if !lbl_hwnd.is_null() {
            let wide = wide_string(&status_line);
            // SAFETY: Updates the status STATIC control text outside any RefCell borrow.
            unsafe { SetWindowTextW(lbl_hwnd, wide.as_ptr()) };
        }
        if !details_hwnd.is_null() {
            let wide = wide_string(&details);
            // SAFETY: Updates the concise resource and playback details outside GUI_STATE borrowing.
            unsafe { SetWindowTextW(details_hwnd, wide.as_ptr()) };
        }
        if !pause_button.is_null() {
            let wide = wide_string(pause_label);
            // SAFETY: Updates the state-aware pause action caption outside GUI_STATE borrowing.
            unsafe { SetWindowTextW(pause_button, wide.as_ptr()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_status_shows_file_name_without_exposing_full_path() {
        let state = PlaybackState::Playing {
            path: PathBuf::from(r"C:\private\wallpapers\auralis-test.mp4"),
            position_secs: 1.0,
        };

        assert_eq!(
            playback_status_caption(Some(&state)),
            "Playing · auralis-test.mp4"
        );
        assert_eq!(playback_pause_caption(Some(&state)), "Pause wallpaper");
    }

    #[test]
    fn paused_wallpaper_offers_resume_action() {
        let state = PlaybackState::Paused {
            path: PathBuf::from("auralis-test.mp4"),
            position_secs: 1.0,
        };

        assert_eq!(
            playback_status_caption(Some(&state)),
            "Paused · auralis-test.mp4"
        );
        assert_eq!(playback_pause_caption(Some(&state)), "Resume wallpaper");
    }

    #[test]
    fn playback_controls_follow_wallpaper_lifecycle() {
        let path = PathBuf::from("auralis-test.mp4");
        assert!(!playback_controls_enabled(None));
        assert!(playback_controls_enabled(Some(&PlaybackState::Loading {
            path: path.clone(),
        })));
        assert!(playback_controls_enabled(Some(&PlaybackState::Playing {
            path: path.clone(),
            position_secs: 1.0,
        })));
        assert!(playback_controls_enabled(Some(&PlaybackState::Paused {
            path: path.clone(),
            position_secs: 1.0,
        })));
        assert!(!playback_controls_enabled(Some(&PlaybackState::Stopped {
            path: path.clone(),
        })));
        assert!(!playback_controls_enabled(Some(&PlaybackState::Unloaded)));
    }

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
