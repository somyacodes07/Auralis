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

use super::windows::{
    read_startup_command, startup_command_for, startup_command_matches, string_from_wide,
    wide_string, WindowsPlatformAdapter,
};
use super::windows_player::MfVideoPlayer;
use crate::config::{AppConfig, DisplayWallpaperConfig, VideoAspectMode};
use crate::engine::WallpaperEngine;
use crate::media::PlaybackState;
use crate::platform::{create_platform_adapter, PowerStatus};
use crate::PlatformEvent;
use std::cell::{Cell, RefCell};
use std::mem;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT,
    WAIT_OBJECT_0, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawFocusRect, DrawTextW,
    EndPaint, FillRect, GetDC, GetDeviceCaps, GetStockObject, LineTo, MoveToEx, ReleaseDC,
    RoundRect, SelectObject, SetBkColor, SetBkMode, SetTextColor, BLACK_BRUSH, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_GUI_FONT, DEFAULT_PITCH, DT_CALCRECT, DT_CENTER,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, FF_SWISS,
    FW_NORMAL, FW_SEMIBOLD, HBRUSH, HFONT, HGDIOBJ, HPEN, LOGPIXELSY, NULL_BRUSH, OUT_TT_PRECIS,
    TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenMutexW, OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    SYNCHRONIZATION_SYNCHRONIZE,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, EM_SETSEL, MEASUREITEMSTRUCT, ODS_FOCUS, ODS_HOTLIGHT, ODS_SELECTED,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, IsWindowEnabled, SetFocus,
};
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, DragAcceptFiles, DragFinish, DragQueryFileW, RemoveWindowSubclass,
    SetWindowSubclass, Shell_NotifyIconW, HDROP, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD,
    NIM_DELETE, NOTIFYICONDATAW, SUBCLASSPROC,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, FindWindowW, GetCursorPos,
    GetDlgCtrlID, GetMessageW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsChild, IsDialogMessageW, IsWindow, KillTimer, LoadCursorW,
    LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowPos, SetWindowTextW, ShowWindow, SystemParametersInfoW,
    TrackPopupMenu, TranslateMessage, BM_CLICK, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX,
    BS_OWNERDRAW, BS_PUSHBUTTON, CBN_SELCHANGE, CBS_DROPDOWNLIST, CBS_HASSTRINGS,
    CBS_OWNERDRAWFIXED, CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL,
    CB_SETITEMHEIGHT, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, ES_AUTOHSCROLL, IDC_ARROW,
    IDI_APPLICATION, IDOK, MF_SEPARATOR, MF_STRING, MSG, PBT_APMPOWERSTATUSCHANGE,
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, SPI_GETCLIENTAREAANIMATION,
    SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_RESTORE, SW_SHOW, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_RETURNCMD, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_DRAWITEM, WM_DROPFILES, WM_ENDSESSION, WM_ERASEBKGND,
    WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_MEASUREITEM, WM_NCDESTROY, WM_PAINT, WM_POWERBROADCAST,
    WM_QUERYENDSESSION, WM_RBUTTONUP, WM_SETFONT, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CAPTION,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CONTROLPARENT, WS_EX_TRANSPARENT,
    WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

pub const WINDOW_CLASS_NAME: &str = "AuralisConfigStudioWindowClass";
const PREVIEW_CLASS_NAME: &str = "AuralisPreviewSurfaceWindowClass";

const WM_AURALIS_SHOW_UI: u32 = WM_APP + 1;
const WM_AURALIS_RELOAD_CONFIG: u32 = WM_APP + 2;
const WM_AURALIS_TRAY_ICON: u32 = WM_APP + 3;

/// `SS_NOPREFIX` prevents `STATIC` controls from interpreting `&` as an underline accelerator.
const SS_NOPREFIX: u32 = 0x0080;
const SS_CENTER: u32 = 0x0001;
/// `SS_ENDELLIPSIS` truncates long `STATIC` text with an ellipsis instead of clipping it.
const SS_ENDELLIPSIS: u32 = 0x4000;
const EN_CHANGE: usize = 0x0300;

/// Session-local name of the mutex that keeps one Studio per signed-in user.
const SINGLE_INSTANCE_MUTEX_NAME: &str = r"Local\AuralisStudio";
/// How long a second launch waits for the first instance's window to appear.
const EXISTING_WINDOW_WAIT: Duration = Duration::from_secs(3);
/// Timer ticks between checks that the wallpaper hosts are still inside the desktop shell.
const HOST_CHECK_TICKS: u32 = 20;

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

const fn is_checkbox_control_id(control_id: usize) -> bool {
    matches!(
        control_id,
        ID_CHK_MUTE
            | ID_CHK_LOOP
            | ID_CHK_FULLSCREEN
            | ID_CHK_BATTERY
            | ID_CHK_STARTUP
            | ID_CHK_REDUCE_MOTION
    )
}

struct GuiState {
    engine: WallpaperEngine,
    config_path: PathBuf,
    /// DPI the fonts and child layout are currently built for.
    dpi: u32,
    /// Every child control with the layout entry it was created from.
    controls: Vec<(HWND, &'static ControlSpec)>,
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
    /// "On this display" readout: file name and resolution/state lines.
    lbl_now_file: HWND,
    lbl_now_meta: HWND,
    /// Static labels painted in the secondary text color (field labels, hints, details).
    secondary_labels: Vec<HWND>,
    last_error: Option<String>,
    last_status_text: String,
    tray_added: bool,
    brand_icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON,
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

thread_local! {
    /// DPI of the Studio window once it exists. Painting code reads this instead of the system
    /// DPI so a window on a monitor with a different scale factor draws at that monitor's DPI.
    static STUDIO_DPI: Cell<u32> = const { Cell::new(0) };
}

/// System DPI, used only before the Studio window exists.
fn system_dpi() -> u32 {
    // SAFETY: GetDC(null) retrieves the screen DC, GetDeviceCaps queries logical DPI, and ReleaseDC frees it.
    unsafe {
        let hdc = GetDC(std::ptr::null_mut());
        if hdc.is_null() {
            return 96;
        }
        let dpi = GetDeviceCaps(hdc, LOGPIXELSY as i32).max(96);
        ReleaseDC(std::ptr::null_mut(), hdc);
        dpi as u32
    }
}

/// Per-window DPI through `GetDpiForWindow` (Windows 10 1607+), resolved at runtime so older
/// systems fall back to the system DPI instead of failing to start.
fn window_dpi(hwnd: HWND) -> Option<u32> {
    type FnGetDpiForWindow = unsafe extern "system" fn(hwnd: HWND) -> u32;
    let user32_name = wide_string("user32.dll");
    // SAFETY: user32.dll is loaded in every GUI process; the handle is not freed.
    let user32 = unsafe { GetModuleHandleW(user32_name.as_ptr()) };
    if user32.is_null() || hwnd.is_null() {
        return None;
    }
    // SAFETY: Looks up an exported symbol by its null-terminated ANSI name.
    let address = unsafe { GetProcAddress(user32, c"GetDpiForWindow".as_ptr().cast()) }?;
    // SAFETY: The signature matches GetDpiForWindow in WinUser.h.
    let get_dpi: FnGetDpiForWindow = unsafe { mem::transmute(address) };
    // SAFETY: hwnd is a live window owned by this thread.
    let dpi = unsafe { get_dpi(hwnd) };
    (dpi >= 96).then_some(dpi)
}

/// Current Studio DPI and its scale factor relative to 96 DPI (e.g. `1.25` at 125% scaling).
fn query_dpi_scale() -> (i32, f32) {
    let cached = STUDIO_DPI.with(Cell::get);
    let dpi = if cached >= 96 { cached } else { system_dpi() };
    (dpi as i32, dpi_scale(dpi))
}

fn dpi_scale(dpi: u32) -> f32 {
    (dpi.max(96) as f32) / 96.0
}

#[inline]
fn scale_px(px: i32, scale: f32) -> i32 {
    ((px as f32) * scale).round() as i32
}

/// Convert a logical `(x, y, width, height)` layout rectangle to physical pixels.
fn scale_rect(rect: LogicalRect, scale: f32) -> LogicalRect {
    let (x, y, width, height) = rect;
    (
        scale_px(x, scale),
        scale_px(y, scale),
        scale_px(width, scale),
        scale_px(height, scale),
    )
}

/// Font height for `CreateFontW` (negative = character height) at `points` and `dpi`.
const fn font_height(points: i32, dpi: u32) -> i32 {
    -((points * dpi as i32) / 72)
}

/// The three Studio typefaces, built for one DPI.
#[derive(Clone, Copy)]
struct StudioFonts {
    ui: HFONT,
    section: HFONT,
    heading: HFONT,
}

fn create_studio_font(points: i32, weight: u32, dpi: u32) -> HFONT {
    let face = wide_string("Segoe UI");
    // SAFETY: CreateFontW creates a ClearType Segoe UI font; the caller owns and deletes it.
    unsafe {
        CreateFontW(
            font_height(points, dpi),
            0,
            0,
            0,
            weight as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_TT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_SWISS) as u32,
            face.as_ptr(),
        )
    }
}

fn create_studio_fonts(dpi: u32) -> StudioFonts {
    StudioFonts {
        ui: create_studio_font(11, FW_NORMAL, dpi),
        section: create_studio_font(11, FW_SEMIBOLD, dpi),
        heading: create_studio_font(14, FW_SEMIBOLD, dpi),
    }
}

fn delete_studio_fonts(fonts: StudioFonts) {
    for font in [fonts.ui, fonts.section, fonts.heading] {
        if !font.is_null() {
            // SAFETY: Each font was created by create_studio_font and is no longer selected
            // into any control (callers switch controls to the replacement first).
            unsafe { DeleteObject(font as HGDIOBJ) };
        }
    }
}

/// Release the console window Windows creates when the Studio is started from Explorer, the
/// Start menu, or the sign-in Run key.
///
/// `auralis.exe` is a console-subsystem binary so CLI commands print to the terminal. When the
/// Studio is the only process attached to its console, that console exists solely for this
/// launch: leaving it open shows a stray black window, and selecting text in it suspends
/// writes to stdout/stderr, which would block the GUI thread on the next log line. A console
/// shared with a parent shell is left attached so terminal output keeps working.
fn detach_private_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut processes = [0u32; 2];
    // SAFETY: GetConsoleProcessList writes at most `processes.len()` ids into the local buffer.
    let attached = unsafe { GetConsoleProcessList(processes.as_mut_ptr(), processes.len() as u32) };
    if attached == 1 {
        // SAFETY: Detaching from a console this process alone owns closes only that console.
        unsafe { FreeConsole() };
    }
}

/// Logical (96 DPI) client size of the Studio layout. Every control position below lives on
/// this grid and is scaled with `scale_px`.
const CLIENT_WIDTH: i32 = 1160;
const CLIENT_HEIGHT: i32 = 748;
/// Height of the header strip that carries the wordmark and the live-status marker.
const HEADER_HEIGHT: i32 = 56;

/// Center a frame of `width` x `height` inside `work`, shrinking it if the work area is smaller.
fn center_in_rect(work: RECT, width: i32, height: i32) -> (i32, i32, i32, i32) {
    let work_width = work.right - work.left;
    let work_height = work.bottom - work.top;
    let width = width.min(work_width);
    let height = height.min(work_height);
    (
        work.left + (work_width - width) / 2,
        work.top + (work_height - height) / 2,
        width,
        height,
    )
}

/// Place the Studio frame in the centre of the primary monitor's work area (excludes the taskbar).
fn centered_in_work_area(width: i32, height: i32) -> Option<(i32, i32, i32, i32)> {
    let mut work: RECT = unsafe { mem::zeroed() };
    // SAFETY: SPI_GETWORKAREA writes one RECT into the correctly sized local value.
    let ok =
        unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0) };
    (ok != 0 && work.right > work.left && work.bottom > work.top)
        .then(|| center_in_rect(work, width, height))
}

fn point_segment_distance(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let (px, py) = point;
    let (sx, sy) = start;
    let (ex, ey) = end;
    let dx = ex - sx;
    let dy = ey - sy;
    let length_squared = dx * dx + dy * dy;
    let projection = if length_squared <= f32::EPSILON {
        0.0
    } else {
        (((px - sx) * dx + (py - sy) * dy) / length_squared).clamp(0.0, 1.0)
    };
    let nearest_x = sx + projection * dx;
    let nearest_y = sy + projection * dy;
    ((px - nearest_x).powi(2) + (py - nearest_y).powi(2)).sqrt()
}

/// Rasterize the monochrome Auralis mark into a small icon shared by the title bar and tray.
fn brand_icon_pixels() -> (Vec<u8>, Vec<u8>) {
    const SIDE: usize = 32;
    let mut color = vec![0u8; SIDE * SIDE * 4];
    // CreateIcon expects each 1-bit mask scanline to be padded to a 16-bit boundary.
    let mask_stride = SIDE.div_ceil(16) * 2;
    let mask = vec![0u8; SIDE * mask_stride];
    let wave = [
        (11.0, 20.0),
        (14.0, 18.0),
        (16.0, 22.0),
        (18.0, 17.0),
        (21.0, 20.0),
    ];

    for y in 0..SIDE {
        for x in 0..SIDE {
            let xf = x as f32 + 0.5;
            let yf = y as f32 + 0.5;
            let rounded_x = xf.clamp(6.0, 26.0) - xf;
            let rounded_y = yf.clamp(6.0, 26.0) - yf;
            if rounded_x * rounded_x + rounded_y * rounded_y > 36.0 {
                continue;
            }

            let on_a = point_segment_distance((xf, yf), (8.0, 25.0), (16.0, 7.0)) <= 1.6
                || point_segment_distance((xf, yf), (16.0, 7.0), (24.0, 25.0)) <= 1.6;
            let on_wave = wave.windows(2).any(|segment| {
                point_segment_distance(
                    (xf, yf),
                    (segment[0].0, segment[0].1),
                    (segment[1].0, segment[1].1),
                ) <= 1.35
            });
            let offset = ((SIDE - 1 - y) * SIDE + x) * 4;
            let shade = if on_a || on_wave { 242 } else { 24 };
            color[offset..offset + 4].copy_from_slice(&[shade, shade, shade, 255]);
        }
    }
    (color, mask)
}

fn create_brand_icon(
    hinstance: *mut std::ffi::c_void,
) -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    let (color, mask) = brand_icon_pixels();
    // SAFETY: The icon buffers have correctly sized 32x32 BGRA pixels and padded AND-mask rows.
    unsafe { CreateIcon(hinstance, 32, 32, 1, 32, mask.as_ptr(), color.as_ptr()) }
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
        lpfnWndProc: Some(preview_surface_proc),
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

/// Paint a restrained play mark over the idle preview without starting a decoder.
///
/// The preview hint remains a separate native static control so its text stays available to UI
/// Automation. This surface paint is intentionally static and only runs when the preview is idle.
unsafe extern "system" fn preview_surface_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let idle = GUI_STATE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .and_then(|guard| guard.as_ref().map(|state| state.preview_player.is_none()))
                    .unwrap_or(true)
            });
            if !idle {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }

            let mut paint: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT =
                unsafe { mem::zeroed() };
            let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
            if hdc.is_null() {
                return 0;
            }

            let mut client: RECT = unsafe { mem::zeroed() };
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client)
            };
            let (_, scale) = query_dpi_scale();
            let center_x = (client.left + client.right) / 2;
            let center_y = scale_px(104, scale);
            let radius = scale_px(23, scale).max(18);
            let background = unsafe { CreateSolidBrush(rgb(0, 0, 0)) };
            if !background.is_null() {
                unsafe {
                    FillRect(hdc, &client, background);
                    DeleteObject(background as HGDIOBJ);
                }
            }

            let disc_brush = unsafe { CreateSolidBrush(rgb(20, 20, 21)) };
            let disc_pen = unsafe { CreatePen(0, scale_px(1, scale).max(1), rgb(66, 66, 69)) };
            if !disc_brush.is_null() && !disc_pen.is_null() {
                unsafe {
                    let old_brush = SelectObject(hdc, disc_brush as HGDIOBJ);
                    let old_pen = SelectObject(hdc, disc_pen as HGDIOBJ);
                    windows_sys::Win32::Graphics::Gdi::Ellipse(
                        hdc,
                        center_x - radius,
                        center_y - radius,
                        center_x + radius,
                        center_y + radius,
                    );
                    SelectObject(hdc, old_brush);
                    SelectObject(hdc, old_pen);
                    DeleteObject(disc_brush as HGDIOBJ);
                    DeleteObject(disc_pen as HGDIOBJ);
                }
            } else {
                if !disc_brush.is_null() {
                    unsafe { DeleteObject(disc_brush as HGDIOBJ) };
                }
                if !disc_pen.is_null() {
                    unsafe { DeleteObject(disc_pen as HGDIOBJ) };
                }
            }

            let glyph_pen = unsafe { CreatePen(0, scale_px(2, scale).max(1), rgb(210, 210, 212)) };
            if !glyph_pen.is_null() {
                unsafe {
                    let old_pen = SelectObject(hdc, glyph_pen as HGDIOBJ);
                    let left = center_x - scale_px(5, scale);
                    let tip = center_x + scale_px(8, scale);
                    let top = center_y - scale_px(8, scale);
                    let bottom = center_y + scale_px(8, scale);
                    MoveToEx(hdc, left, top, std::ptr::null_mut());
                    LineTo(hdc, tip, center_y);
                    LineTo(hdc, left, bottom);
                    LineTo(hdc, left, top);
                    SelectObject(hdc, old_pen);
                    DeleteObject(glyph_pen as HGDIOBJ);
                    EndPaint(hwnd, &paint);
                }
            } else {
                unsafe { EndPaint(hwnd, &paint) };
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Launch the native Windows Configuration UI and enter the Win32 message & playback loop.
pub fn run_windows_gui(
    initial_video: Option<PathBuf>,
    start_minimized: bool,
) -> Result<(), String> {
    detach_private_console();
    WindowsPlatformAdapter::ensure_dpi_awareness();
    WindowsPlatformAdapter::register_wallpaper_host_class();

    // One Studio per signed-in user. The named mutex is created atomically, so two launches
    // racing each other cannot both pass this check the way a window-class lookup could.
    let Some(instance_guard) = SingleInstanceGuard::acquire() else {
        forward_to_running_instance(initial_video.as_deref());
        return Ok(());
    };

    let class_wide = wide_string(WINDOW_CLASS_NAME);
    let config_path = AppConfig::default_config_path();
    let (mut config, load_warning) = AppConfig::load_or_recover(&config_path);
    if let Some(warn) = &load_warning {
        eprintln!("Warning: {warn}");
    }
    let adapter = create_platform_adapter();
    let startup_actual = adapter.is_launch_at_startup_enabled();
    config.launch_at_startup = startup_actual;

    let mut engine = WallpaperEngine::new(config, adapter).map_err(|e| e.to_string())?;
    let mut startup_warning = None;
    if startup_actual {
        startup_warning = refresh_startup_entry(&mut engine);
    }

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

    // The window does not exist yet, so size it for the system DPI; WM_CREATE and
    // WM_DPICHANGED switch to the window's own DPI once Windows places it on a monitor.
    let dpi = system_dpi();
    let scale = dpi_scale(dpi);
    let fonts = create_studio_fonts(dpi);
    let (ui_font, section_font, heading_font) = (fonts.ui, fonts.section, fonts.heading);
    STUDIO_DPI.with(|cell| cell.set(dpi));
    // SAFETY: Theme brushes and border pen are owned GDI objects released after the UI closes.
    let (background_brush, card_brush, input_brush, border_pen) = unsafe {
        (
            CreateSolidBrush(rgb(13, 13, 14)),
            CreateSolidBrush(rgb(23, 23, 24)),
            CreateSolidBrush(rgb(30, 30, 32)),
            CreatePen(0, 1, rgb(48, 48, 50)),
        )
    };
    UI_FONT.with(|font| font.set(ui_font));
    GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            *guard = Some(GuiState {
                engine,
                config_path,
                dpi,
                controls: Vec::new(),
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
                lbl_now_file: std::ptr::null_mut(),
                lbl_now_meta: std::ptr::null_mut(),
                secondary_labels: Vec::new(),
                last_error: load_warning.or(startup_warning),
                last_status_text: String::new(),
                tray_added: false,
                brand_icon: std::ptr::null_mut(),
                tick_counter: 0,
            });
        }
    });

    // SAFETY: GetModuleHandleW(null) retrieves the current process module handle.
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    register_preview_surface_class(hinstance);

    let brand_icon = create_brand_icon(hinstance);
    GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(state) = guard.as_mut() {
                state.brand_icon = brand_icon;
            }
        }
    });
    // SAFETY: LoadIconW/LoadCursorW supply system fallbacks when icon rasterization fails.
    let (fallback_icon, hcursor) = unsafe {
        (
            LoadIconW(std::ptr::null_mut(), IDI_APPLICATION),
            LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        )
    };
    let hicon = if brand_icon.is_null() {
        fallback_icon
    } else {
        brand_icon
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

    let title = wide_string(&format!("Auralis Studio · v{}", crate::version_label()));
    // WS_CLIPCHILDREN prevents the parent window from erasing over child buttons/labels/preview on repaint.
    let mut style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    if !start_minimized {
        style |= WS_VISIBLE;
    }

    // Size the frame from the client layout so the Studio never extends under the taskbar.
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: scale_px(CLIENT_WIDTH, scale),
        bottom: scale_px(CLIENT_HEIGHT, scale),
    };
    // SAFETY: AdjustWindowRectEx only writes the frame rectangle for the given style.
    unsafe { AdjustWindowRectEx(&mut frame, style & !WS_VISIBLE, 0, WS_EX_CONTROLPARENT) };
    // Explorer broadcasts this message after it (re)creates the taskbar, for example after a
    // crash or restart; the Studio re-adds its tray icon and re-layers wallpapers in response.
    let taskbar_created = wide_string("TaskbarCreated");
    // SAFETY: RegisterWindowMessageW takes a valid null-terminated UTF-16 message name.
    let taskbar_message = unsafe { RegisterWindowMessageW(taskbar_created.as_ptr()) };
    TASKBAR_CREATED_MESSAGE.with(|cell| cell.set(taskbar_message));
    let (x, y, width, height) =
        centered_in_work_area(frame.right - frame.left, frame.bottom - frame.top).unwrap_or((
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            frame.right - frame.left,
            frame.bottom - frame.top,
        ));

    // SAFETY: CreateWindowExW creates the main configuration window and triggers WM_CREATE.
    // WS_EX_CONTROLPARENT lets IsDialogMessageW Tab through the child controls.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class_wide.as_ptr(),
            title.as_ptr(),
            style,
            x,
            y,
            width,
            height,
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

    // The frame was sized for the system DPI. If Windows placed the window on a monitor with
    // another scale factor, rebuild fonts and layout for it and resize the client area to match.
    if let Some(actual_dpi) = window_dpi(hwnd).filter(|actual| *actual != dpi) {
        apply_studio_dpi(hwnd, actual_dpi);
        resize_frame_for_dpi(hwnd, actual_dpi);
    }

    // SAFETY: Standard Win32 message pump driving UI events, timers, and Media Foundation EVR updates.
    // IsDialogMessageW gives the Studio dialog-style keyboard navigation (Tab, Shift+Tab, Space,
    // Enter, arrow keys in the selector); messages it handles must not be dispatched again.
    unsafe {
        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if IsWindow(hwnd) != 0 && IsDialogMessageW(hwnd, &msg) != 0 {
                continue;
            }
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
                UI_FONT.with(|font| font.set(std::ptr::null_mut()));
                // The window and its controls are destroyed, so no control still uses the fonts.
                delete_studio_fonts(StudioFonts {
                    ui: state.ui_font,
                    section: state.section_font,
                    heading: state.heading_font,
                });
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
                if !state.brand_icon.is_null() {
                    // SAFETY: The app-owned tray icon was removed in WM_DESTROY before shutdown.
                    unsafe { DestroyIcon(state.brand_icon) };
                }
            }
        }
    });
    STUDIO_DPI.with(|cell| cell.set(0));
    drop(instance_guard);

    Ok(())
}

thread_local! {
    /// Registered `TaskbarCreated` message id (0 until registered).
    static TASKBAR_CREATED_MESSAGE: Cell<u32> = const { Cell::new(0) };
    /// Child control that had keyboard focus when the Studio was last deactivated.
    static LAST_FOCUS: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
}

/// Owns the named single-instance mutex for the lifetime of the Studio.
struct SingleInstanceGuard(HANDLE);

impl SingleInstanceGuard {
    /// Create the session mutex. Returns `None` when another Studio already owns it.
    fn acquire() -> Option<Self> {
        let name = wide_string(SINGLE_INSTANCE_MUTEX_NAME);
        // SAFETY: Creates (or opens) a named mutex with default security; the name is a valid
        // null-terminated UTF-16 string. GetLastError is read immediately after the call.
        let (handle, already_exists) = unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            (handle, GetLastError() == ERROR_ALREADY_EXISTS)
        };
        if already_exists {
            if !handle.is_null() {
                // SAFETY: Releases this process's handle to the other instance's mutex.
                unsafe { CloseHandle(handle) };
            }
            return None;
        }
        // If the mutex could not be created at all, run without the guard rather than refuse
        // to start; the window-class lookup below still finds an obvious duplicate.
        Some(Self(handle))
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: Closes the mutex handle this guard created exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Whether another process currently holds the Studio's single-instance mutex.
fn studio_mutex_exists() -> bool {
    let name = wide_string(SINGLE_INSTANCE_MUTEX_NAME);
    // SAFETY: Opens the named mutex for SYNCHRONIZE only, to test for its existence.
    let handle = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    // SAFETY: Closes the handle opened just above.
    unsafe { CloseHandle(handle) };
    true
}

/// Find the running Studio's main window, waiting up to `wait` while it is still starting.
fn find_studio_window(wait: Duration) -> Option<HWND> {
    let class_wide = wide_string(WINDOW_CLASS_NAME);
    let deadline = Instant::now() + wait;
    loop {
        // SAFETY: FindWindowW looks up a top-level window by its null-terminated class name.
        let existing = unsafe { FindWindowW(class_wide.as_ptr(), std::ptr::null()) };
        if !existing.is_null() {
            return Some(existing);
        }
        if Instant::now() >= deadline || !studio_mutex_exists() {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Hand a second launch over to the Studio that is already running.
///
/// A plain launch brings the existing window forward. A launch with a video (for example a file
/// dropped onto `auralis.exe`) saves it as the primary display's wallpaper and asks the running
/// Studio to reload, the same way `auralis apply` does, instead of starting a second engine.
fn forward_to_running_instance(initial_video: Option<&Path>) {
    let Some(existing) = find_studio_window(EXISTING_WINDOW_WAIT) else {
        eprintln!("Auralis is already starting. Try again in a few seconds.");
        return;
    };
    if let Some(video) = initial_video {
        match save_video_for_primary_display(video) {
            // Reload synchronously, exactly like `auralis apply`, before asking for the window.
            // SAFETY: Sends the reload request to the live Studio window; no pointers are passed.
            Ok(()) => unsafe {
                SendMessageW(existing, WM_AURALIS_RELOAD_CONFIG, 0, 0);
            },
            Err(error) => eprintln!("Auralis could not use this video: {error}"),
        }
    }
    // SAFETY: Shows and focuses the existing Studio window owned by the other process.
    unsafe {
        PostMessageW(existing, WM_AURALIS_SHOW_UI, 0, 0);
        ShowWindow(existing, SW_RESTORE);
        SetForegroundWindow(existing);
    }
}

/// Validate `video` and store it as the primary display's wallpaper in the saved configuration.
fn save_video_for_primary_display(video: &Path) -> Result<(), String> {
    let mut wallpaper =
        DisplayWallpaperConfig::new(video.to_path_buf()).map_err(|e| e.to_string())?;
    let config_path = AppConfig::default_config_path();
    let (mut config, _) = AppConfig::load_or_recover(&config_path);
    let displays = create_platform_adapter()
        .enumerate_displays()
        .map_err(|e| e.to_string())?;
    let primary = displays
        .iter()
        .find(|d| d.is_primary)
        .or_else(|| displays.first())
        .ok_or_else(|| "No active display was found.".to_string())?;
    if let Some(existing) = config
        .per_display
        .get(&primary.id)
        .or(config.default_wallpaper.as_ref())
    {
        wallpaper.loop_playback = existing.loop_playback;
        wallpaper.muted = existing.muted;
        wallpaper.volume = existing.volume;
        wallpaper.speed_percent = existing.speed_percent;
        wallpaper.aspect_mode = existing.aspect_mode;
    }
    config.record_recent_wallpaper(video.to_path_buf());
    config
        .per_display
        .insert(primary.id.clone(), wallpaper.clone());
    config.default_wallpaper = Some(wallpaper);
    config.save_to_file(&config_path).map_err(|e| e.to_string())
}

/// Result of asking a running Studio to quit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitOutcome {
    /// No Studio was running.
    NotRunning,
    /// The Studio quit and its wallpaper hosts were removed.
    Quit,
    /// The Studio was asked to quit but was still running when the wait ended.
    TimedOut,
}

/// Ask a running Studio to quit and restore the desktop, then wait up to `timeout` for it to
/// exit. This performs the same action as the Studio's "Quit and restore desktop" button:
/// wallpaper playback stops, host windows are destroyed, and the configuration is saved.
#[must_use]
pub fn quit_running_instance(timeout: Duration) -> QuitOutcome {
    WindowsPlatformAdapter::ensure_dpi_awareness();
    // Returns at once when no Studio exists; waits briefly only if one is still starting.
    let Some(window) = find_studio_window(Duration::from_secs(1)) else {
        return QuitOutcome::NotRunning;
    };
    let mut process_id = 0u32;
    // SAFETY: Reads the owning process id of a window handle into a local value.
    unsafe { GetWindowThreadProcessId(window, &mut process_id) };
    let process = if process_id == 0 {
        std::ptr::null_mut()
    } else {
        // SAFETY: Opens the Studio process for SYNCHRONIZE only, so this call can wait on it.
        unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, process_id) }
    };

    // WM_COMMAND with the quit button's id is the button's own action. Studio builds that
    // predate this command handle it identically, so an installer can close any version.
    // SAFETY: Posts a command message to the live Studio window; no pointers are passed.
    let posted = unsafe { PostMessageW(window, WM_COMMAND, ID_BTN_QUIT, 0) } != 0;
    if !posted {
        if !process.is_null() {
            // SAFETY: Closes the process handle opened above.
            unsafe { CloseHandle(process) };
        }
        return QuitOutcome::TimedOut;
    }

    let exited = if process.is_null() {
        // Without a process handle, wait for the window itself to disappear.
        let deadline = Instant::now() + timeout;
        // SAFETY: IsWindow only checks whether the handle is still in the window table.
        while unsafe { IsWindow(window) } != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        // SAFETY: Same window-table query as above.
        unsafe { IsWindow(window) == 0 }
    } else {
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: Waits on the process handle opened above, then closes it.
        unsafe {
            let result = WaitForSingleObject(process, millis);
            CloseHandle(process);
            result == WAIT_OBJECT_0
        }
    };
    if exited {
        QuitOutcome::Quit
    } else {
        QuitOutcome::TimedOut
    }
}

/// Keep the sign-in entry pointing at this executable, so moving or reinstalling Auralis does
/// not leave a `Run` command that launches a missing or outdated copy. Returns a notice when
/// the entry could not be updated.
fn refresh_startup_entry(engine: &mut WallpaperEngine) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let expected = startup_command_for(&exe);
    let current = read_startup_command();
    if current
        .as_deref()
        .is_some_and(|stored| startup_command_matches(stored, &expected))
    {
        return None;
    }
    engine.set_launch_at_startup(true).err().map(|error| {
        format!(
            "Could not update the sign-in entry to this copy of Auralis ({error}). Turn \
             \"Start Auralis when I sign in\" off and on again to retry."
        )
    })
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
    nid.hIcon = GUI_STATE
        .with(|cell| {
            cell.try_borrow()
                .ok()
                .and_then(|guard| guard.as_ref().map(|state| state.brand_icon))
                .filter(|icon| !icon.is_null())
        })
        // SAFETY: LoadIconW returns a shared system icon if CreateIcon could not allocate our mark.
        .unwrap_or_else(|| unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) });

    let tip = wide_string(&format!(
        "Auralis Live Wallpaper Studio v{} (Right-click for controls)",
        crate::version_label()
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

thread_local! {
    /// Studio UI font, cached outside `GUI_STATE` so owner-draw painting that happens while the
    /// state is mutably borrowed (for example a repaint triggered inside a command handler)
    /// still uses Segoe UI instead of falling back to the small stock GUI font.
    static UI_FONT: std::cell::Cell<HFONT> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

fn ui_font_handle() -> HFONT {
    let cached = UI_FONT.with(std::cell::Cell::get);
    if cached.is_null() {
        // SAFETY: GetStockObject returns a process-owned stock font handle.
        unsafe { GetStockObject(DEFAULT_GUI_FONT) as HFONT }
    } else {
        cached
    }
}

/// Paint the monochrome app canvas and its soft-edged section surfaces.
fn paint_main_window(hwnd: HWND) {
    let mut paint: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = unsafe { mem::zeroed() };
    // SAFETY: BeginPaint/EndPaint delimit this window's WM_PAINT handling.
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    if hdc.is_null() {
        return;
    }

    // A WM_PAINT can arrive from a nested message pump while GUI_STATE is mutably borrowed.
    let state_busy = GUI_STATE.with(|cell| cell.try_borrow().is_err());
    let snapshot = GUI_STATE.with(|cell| {
        cell.try_borrow().ok().and_then(|guard| {
            let state = guard.as_ref()?;
            let display_id = selected_display_id(state);
            let playback = state
                .engine
                .controller(&display_id)
                .map(|controller| &controller.state);
            Some(PaintSnapshot {
                background: state.background_brush,
                card: state.card_brush,
                input: state.input_brush,
                border: state.border_pen,
                ui_font: state.ui_font,
                heading_font: state.heading_font,
                phase: state.tick_counter % 10,
                live: matches!(playback, Some(PlaybackState::Playing { .. })),
                status: header_status_caption(playback, &display_id),
            })
        })
    });
    let Some(snap) = snapshot.filter(|s| {
        !s.background.is_null() && !s.card.is_null() && !s.input.is_null() && !s.border.is_null()
    }) else {
        // SAFETY: Completes the paint cycle even if the window is shutting down.
        unsafe { EndPaint(hwnd, &paint) };
        if state_busy {
            // Ask for this region again once the borrow is released; otherwise the cards would
            // stay unpainted until something else invalidates them.
            // SAFETY: Re-invalidates part of this live window; WM_PAINT is low priority.
            unsafe { windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, &paint.rcPaint, 0) };
        }
        return;
    };

    let (_, scale) = query_dpi_scale();
    let px = |value: i32| scale_px(value, scale);
    // SAFETY: Fills only the invalid client region; WS_CLIPCHILDREN keeps child controls untouched.
    // Every GDI object selected here is restored, and temporary objects are deleted.
    unsafe {
        FillRect(hdc, &paint.rcPaint, snap.background);
        let old_brush = SelectObject(hdc, snap.card as HGDIOBJ);
        let old_pen = SelectObject(hdc, snap.border as HGDIOBJ);
        // Cards share one 12px radius and sit on a 16px gutter grid.
        for (left, top, right, bottom) in CARD_RECTS {
            RoundRect(
                hdc,
                px(left),
                px(top),
                px(right),
                px(bottom),
                px(12),
                px(12),
            );
        }
        // The path field is a borderless EDIT inset inside this drawn frame so its text is
        // vertically centred and the frame matches the display selector.
        SelectObject(hdc, snap.input as HGDIOBJ);
        let (left, top, right, bottom) = PATH_FIELD_RECT;
        RoundRect(hdc, px(left), px(top), px(right), px(bottom), px(8), px(8));
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);

        // A compact geometric A mark echoes the standalone SVG wordmark.
        let mark_pen = CreatePen(0, px(2).max(1), rgb(232, 232, 234));
        if !mark_pen.is_null() {
            let old_mark_pen = SelectObject(hdc, mark_pen as HGDIOBJ);
            MoveToEx(hdc, px(24), px(40), std::ptr::null_mut());
            LineTo(hdc, px(36), px(15));
            LineTo(hdc, px(48), px(40));
            MoveToEx(hdc, px(28), px(32), std::ptr::null_mut());
            LineTo(hdc, px(32), px(32));
            LineTo(hdc, px(36), px(25));
            LineTo(hdc, px(40), px(36));
            LineTo(hdc, px(44), px(29));
            LineTo(hdc, px(47), px(29));
            SelectObject(hdc, old_mark_pen);
            DeleteObject(mark_pen as HGDIOBJ);
        }

        SetBkMode(hdc, TRANSPARENT as i32);
        let old_font = if snap.heading_font.is_null() {
            std::ptr::null_mut()
        } else {
            SelectObject(hdc, snap.heading_font as HGDIOBJ)
        };
        SetTextColor(hdc, rgb(240, 240, 242));
        let brand = wide_string("Auralis");
        let mut brand_rect = RECT {
            left: px(60),
            top: 0,
            right: px(300),
            bottom: px(HEADER_HEIGHT),
        };
        DrawTextW(
            hdc,
            brand.as_ptr(),
            -1,
            &mut brand_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        // Measure the wordmark so "Studio" follows it at a fixed 8px gap at any DPI.
        let mut brand_measure = brand_rect;
        DrawTextW(
            hdc,
            brand.as_ptr(),
            -1,
            &mut brand_measure,
            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_CALCRECT,
        );
        if !old_font.is_null() {
            SelectObject(hdc, old_font);
        }
        let old_ui_font = if snap.ui_font.is_null() {
            std::ptr::null_mut()
        } else {
            SelectObject(hdc, snap.ui_font as HGDIOBJ)
        };
        SetTextColor(hdc, rgb(140, 140, 145));
        let tagline = wide_string("Studio");
        let mut tagline_rect = RECT {
            left: brand_measure.right + px(8),
            top: px(1),
            right: px(400),
            bottom: px(HEADER_HEIGHT),
        };
        DrawTextW(
            hdc,
            tagline.as_ptr(),
            -1,
            &mut tagline_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );

        // Live status is right-aligned to the card edge so long captions never clip.
        SetTextColor(
            hdc,
            if snap.live {
                rgb(232, 232, 234)
            } else {
                rgb(160, 160, 164)
            },
        );
        let status_wide = wide_string(&snap.status);
        let mut status_rect = RECT {
            left: px(600),
            top: px(1),
            right: px(CLIENT_WIDTH - 24),
            bottom: px(HEADER_HEIGHT),
        };
        DrawTextW(
            hdc,
            status_wide.as_ptr(),
            -1,
            &mut status_rect,
            DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        // Measure the caption so the marker sits 8px before its first glyph.
        let mut measure = status_rect;
        DrawTextW(
            hdc,
            status_wide.as_ptr(),
            -1,
            &mut measure,
            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_CALCRECT,
        );
        let text_width = (measure.right - measure.left).min(status_rect.right - status_rect.left);
        if !old_ui_font.is_null() {
            SelectObject(hdc, old_ui_font);
        }

        // A low-frequency grayscale pulse communicates live playback without distracting color.
        let brightness = if snap.live {
            let step = if snap.phase <= 5 {
                snap.phase
            } else {
                10 - snap.phase
            };
            130 + (step as u8 * 20)
        } else {
            84
        };
        let dot_color = rgb(brightness, brightness, brightness);
        let dot_brush = CreateSolidBrush(dot_color);
        let dot_pen = CreatePen(0, 1, dot_color);
        if !dot_brush.is_null() && !dot_pen.is_null() {
            let old_dot_brush = SelectObject(hdc, dot_brush as HGDIOBJ);
            let old_dot_pen = SelectObject(hdc, dot_pen as HGDIOBJ);
            let dot_size = px(8);
            let dot_x = status_rect.right - text_width - px(8) - dot_size;
            let dot_y = (px(HEADER_HEIGHT) - dot_size) / 2;
            windows_sys::Win32::Graphics::Gdi::Ellipse(
                hdc,
                dot_x,
                dot_y,
                dot_x + dot_size,
                dot_y + dot_size,
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

/// Values read from `GUI_STATE` before painting so no borrow is held across GDI calls.
struct PaintSnapshot {
    background: HBRUSH,
    card: HBRUSH,
    input: HBRUSH,
    border: HPEN,
    ui_font: HFONT,
    heading_font: HFONT,
    phase: u32,
    live: bool,
    status: String,
}

/// Card surfaces in logical pixels: preview, source, playback, behavior, status.
const CARD_RECTS: [(i32, i32, i32, i32); 5] = [
    (24, 56, 584, 464),
    (600, 56, 1136, 464),
    (24, 480, 584, 632),
    (600, 480, 1136, 632),
    (24, 648, 1136, 732),
];
/// Drawn frame for the borderless video-path EDIT control.
const PATH_FIELD_RECT: (i32, i32, i32, i32) = (624, 192, 880, 228);

/// Short header caption for the selected display's wallpaper state.
fn header_status_caption(state: Option<&PlaybackState>, display_id: &str) -> String {
    match state {
        Some(PlaybackState::Playing { .. }) => format!("Live on {display_id}"),
        Some(PlaybackState::Paused { .. }) => format!("Paused on {display_id}"),
        Some(PlaybackState::Loading { .. }) => "Loading wallpaper".to_string(),
        Some(PlaybackState::Failed { .. }) => "Wallpaper failed to load".to_string(),
        Some(PlaybackState::Stopped { .. } | PlaybackState::Unloaded) | None => {
            "No wallpaper running".to_string()
        }
    }
}

/// Draw a solid 2px rounded keyboard-focus ring (replaces the dotted XOR `DrawFocusRect`).
///
/// # Safety
/// `hdc` must be a valid device context for the duration of the call.
unsafe fn draw_focus_ring(hdc: windows_sys::Win32::Graphics::Gdi::HDC, rect: &RECT, color: u32) {
    let scale = query_dpi_scale().1;
    let pen = CreatePen(0, scale_px(2, scale).max(1), color);
    if pen.is_null() {
        DrawFocusRect(hdc, rect);
        return;
    }
    let old_pen = SelectObject(hdc, pen as HGDIOBJ);
    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
    RoundRect(
        hdc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        scale_px(6, scale),
        scale_px(6, scale),
    );
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    DeleteObject(pen as HGDIOBJ);
}

/// Glyph painted on the volume/speed step buttons; their accessible names stay descriptive.
const fn step_button_glyph(control_id: usize) -> Option<&'static str> {
    match control_id {
        ID_BTN_VOL_DOWN | ID_BTN_SPEED_DOWN => Some("\u{2212}"),
        ID_BTN_VOL_UP | ID_BTN_SPEED_UP => Some("+"),
        _ => None,
    }
}

/// Draw the flat, rounded monochrome treatment for action buttons.
unsafe fn draw_action_button(item: &DRAWITEMSTRUCT) {
    let primary = item.CtlID as usize == ID_BTN_APPLY;
    let pressed = item.itemState & ODS_SELECTED != 0;
    let hot = item.itemState & ODS_HOTLIGHT != 0;
    let focused = item.itemState & ODS_FOCUS != 0;
    let disabled = item.itemState & 0x0004 != 0;
    let face = if disabled {
        rgb(33, 33, 35)
    } else if primary {
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
    let edge = if disabled {
        rgb(46, 46, 48)
    } else if primary {
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
        scale_px(8, query_dpi_scale().1),
        scale_px(8, query_dpi_scale().1),
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
    // Step buttons keep a descriptive window text for UI Automation but paint a compact glyph.
    let mut text = vec![0u16; 512];
    let text_len = match step_button_glyph(item.CtlID as usize) {
        Some(glyph) => {
            let wide: Vec<u16> = glyph.encode_utf16().collect();
            text[..wide.len()].copy_from_slice(&wide);
            wide.len() as i32
        }
        None => GetWindowTextW(item.hwndItem, text.as_mut_ptr(), text.len() as i32),
    };
    let mut text_rect = item.rcItem;
    text_rect.left += scale_px(8, query_dpi_scale().1);
    text_rect.right -= scale_px(8, query_dpi_scale().1);
    DrawTextW(
        item.hDC,
        text.as_ptr(),
        text_len,
        &mut text_rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );
    if !old_font.is_null() {
        SelectObject(item.hDC, old_font);
    }
    if focused {
        let inset = scale_px(3, query_dpi_scale().1);
        let mut focus_rect = item.rcItem;
        focus_rect.left += inset;
        focus_rect.top += inset;
        focus_rect.right -= inset;
        focus_rect.bottom -= inset;
        // Dark ring on the light primary button, light ring on graphite buttons.
        draw_focus_ring(
            item.hDC,
            &focus_rect,
            if primary {
                rgb(40, 40, 42)
            } else {
                rgb(200, 200, 204)
            },
        );
    }
}

/// Apply custom dark painting while leaving the native auto-checkbox style intact for UI Automation.
unsafe extern "system" fn checkbox_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_checkbox(hwnd);
            0
        }
        WM_NCDESTROY => {
            // SAFETY: The subclass is removed from the same child window during its destruction.
            unsafe { RemoveWindowSubclass(hwnd, Some(checkbox_subclass_proc), subclass_id) };
            // SAFETY: DefSubclassProc forwards the final lifecycle message to the native button.
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        _ => {
            // SAFETY: All input, keyboard, and automation messages retain the native checkbox behavior.
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
    }
}

/// Paint a monochrome checkbox without replacing its native checked state or accessibility role.
fn paint_checkbox(hwnd: HWND) {
    let mut paint: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = unsafe { mem::zeroed() };
    // SAFETY: BeginPaint/EndPaint delimit this checkbox's WM_PAINT handling.
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    if hdc.is_null() {
        return;
    }

    let card_brush = GUI_STATE
        .with(|cell| {
            cell.try_borrow()
                .ok()
                .and_then(|guard| guard.as_ref().map(|state| state.card_brush))
        })
        .unwrap_or(std::ptr::null_mut());
    // The cached font keeps the label visible even when GUI_STATE is mutably borrowed.
    let ui_font = ui_font_handle();
    let (_, scale) = query_dpi_scale();
    let mut client: RECT = unsafe { mem::zeroed() };
    // SAFETY: hwnd is the live checkbox child receiving WM_PAINT.
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client) };
    if !card_brush.is_null() {
        // SAFETY: Fills this checkbox's client area with the surrounding card surface color.
        unsafe { FillRect(hdc, &client, card_brush) };
    }

    // SAFETY: BM_GETCHECK/BM_GETSTATE read state maintained by the native auto-checkbox control.
    let checked = unsafe { SendMessageW(hwnd, BM_GETCHECK, 0, 0) } == BST_CHECKED as isize;
    let pressed = unsafe {
        SendMessageW(
            hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::BM_GETSTATE,
            0,
            0,
        )
    } & 0x0004
        != 0;
    let disabled = unsafe { IsWindowEnabled(hwnd) } == 0;
    let focused = unsafe { GetFocus() } == hwnd;
    let size = scale_px(16, scale).max(14);
    let left = client.left + scale_px(1, scale);
    let top = client.top + ((client.bottom - client.top - size) / 2);
    let face = if checked {
        if pressed {
            rgb(202, 202, 204)
        } else {
            rgb(232, 232, 234)
        }
    } else if pressed {
        rgb(40, 40, 42)
    } else {
        rgb(29, 29, 31)
    };
    let edge = if checked { face } else { rgb(76, 76, 80) };
    let brush = unsafe { CreateSolidBrush(face) };
    let pen = unsafe { CreatePen(0, scale_px(1, scale).max(1), edge) };
    if !brush.is_null() && !pen.is_null() {
        // SAFETY: The temporary pen and brush are restored and deleted before EndPaint.
        unsafe {
            let old_brush = SelectObject(hdc, brush as HGDIOBJ);
            let old_pen = SelectObject(hdc, pen as HGDIOBJ);
            RoundRect(
                hdc,
                left,
                top,
                left + size,
                top + size,
                scale_px(5, scale),
                scale_px(5, scale),
            );
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            DeleteObject(brush as HGDIOBJ);
            DeleteObject(pen as HGDIOBJ);
        }
    } else {
        if !brush.is_null() {
            unsafe { DeleteObject(brush as HGDIOBJ) };
        }
        if !pen.is_null() {
            unsafe { DeleteObject(pen as HGDIOBJ) };
        }
    }

    if checked {
        let check_pen = unsafe { CreatePen(0, scale_px(2, scale).max(1), rgb(20, 20, 21)) };
        if !check_pen.is_null() {
            // SAFETY: Draws the check glyph and restores/deletes its temporary pen.
            unsafe {
                let old_check_pen = SelectObject(hdc, check_pen as HGDIOBJ);
                MoveToEx(
                    hdc,
                    left + scale_px(4, scale),
                    top + scale_px(8, scale),
                    std::ptr::null_mut(),
                );
                LineTo(hdc, left + scale_px(7, scale), top + scale_px(11, scale));
                LineTo(hdc, left + scale_px(13, scale), top + scale_px(5, scale));
                SelectObject(hdc, old_check_pen);
                DeleteObject(check_pen as HGDIOBJ);
            }
        }
    }

    // SAFETY: Selects the app font and writes the label inside this checkbox's client rectangle.
    unsafe {
        SetBkMode(hdc, TRANSPARENT as i32);
        let mut label_right = client.right;
        if !ui_font.is_null() {
            let old_font = SelectObject(hdc, ui_font as HGDIOBJ);
            SetTextColor(
                hdc,
                if disabled {
                    rgb(124, 124, 128)
                } else {
                    rgb(220, 220, 222)
                },
            );
            let mut text = vec![0u16; 512];
            let text_len = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32);
            let mut text_rect = client;
            text_rect.left = left + size + scale_px(10, scale);
            DrawTextW(
                hdc,
                text.as_ptr(),
                text_len,
                &mut text_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            let mut measure = text_rect;
            DrawTextW(
                hdc,
                text.as_ptr(),
                text_len,
                &mut measure,
                DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_CALCRECT,
            );
            label_right = (measure.right + scale_px(4, scale)).min(client.right);
            if !old_font.is_null() {
                SelectObject(hdc, old_font);
            }
        }
        if focused {
            // Ring only the box and label, not the full control width.
            let focus_rect = RECT {
                left: client.left,
                top: client.top,
                right: label_right,
                bottom: client.bottom,
            };
            draw_focus_ring(hdc, &focus_rect, rgb(200, 200, 204));
        }
        EndPaint(hwnd, &paint);
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
        scale_px(8, query_dpi_scale().1),
        scale_px(8, query_dpi_scale().1),
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
    text_rect.left += scale_px(10, query_dpi_scale().1);
    text_rect.right -= scale_px(8, query_dpi_scale().1);
    DrawTextW(
        item.hDC,
        text.as_ptr(),
        text_len,
        &mut text_rect,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
    );
    if !old_font.is_null() {
        SelectObject(item.hDC, old_font);
    }
    if item.itemState & ODS_FOCUS != 0 {
        let inset = scale_px(2, query_dpi_scale().1);
        let mut focus_rect = item.rcItem;
        focus_rect.left += inset;
        focus_rect.top += inset;
        focus_rect.right -= inset;
        focus_rect.bottom -= inset;
        draw_focus_ring(item.hDC, &focus_rect, rgb(200, 200, 204));
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
    let taskbar_created = TASKBAR_CREATED_MESSAGE.with(Cell::get);
    if taskbar_created != 0 && msg == taskbar_created {
        // Explorer restarted: its tray forgot our icon and the desktop layer that held the
        // wallpaper hosts was destroyed with it.
        add_tray_icon(hwnd);
        recover_wallpaper_hosts();
        return 0;
    }
    match msg {
        WM_CREATE => {
            // Controls are laid out for the creation DPI; run_windows_gui re-lays them out if
            // the window landed on a monitor with a different DPI.
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
            let (brush, preview_empty, secondary) = GUI_STATE.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .and_then(|guard| {
                        guard.as_ref().map(|state| {
                            (
                                state.card_brush,
                                state.lbl_preview_empty,
                                state.secondary_labels.contains(&(lparam as HWND)),
                            )
                        })
                    })
                    .unwrap_or((std::ptr::null_mut(), std::ptr::null_mut(), false))
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
                    // Field labels, hints and details use the secondary tone (7:1 on the card).
                    SetTextColor(
                        wparam as _,
                        if secondary {
                            rgb(150, 150, 155)
                        } else {
                            rgb(218, 218, 220)
                        },
                    );
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
                    item.itemHeight = scale_px(DISPLAY_ITEM_HEIGHT, scale) as u32;
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
                        // Populates the EDIT control with the dropped file path outside any borrow.
                        set_edit_text_showing_end(edt_hwnd, &path_str);
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
                let check_hosts = GUI_STATE.with(|cell| {
                    cell.try_borrow()
                        .ok()
                        .and_then(|guard| {
                            guard
                                .as_ref()
                                .map(|state| state.tick_counter % HOST_CHECK_TICKS == 0)
                        })
                        .unwrap_or(false)
                });
                if check_hosts {
                    recover_wallpaper_hosts();
                }
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
                        right: scale_px(CLIENT_WIDTH, scale),
                        bottom: scale_px(HEADER_HEIGHT, scale),
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
            } else if control_id == IDOK as usize && lparam == 0 {
                // IsDialogMessageW turns Enter into IDOK. Owner-drawn buttons do not claim
                // Enter themselves, so press the focused one.
                press_focused_button(hwnd);
            } else {
                handle_command(hwnd, control_id, notify_code);
            }
            let (_, scale) = query_dpi_scale();
            let header = RECT {
                left: 0,
                top: 0,
                right: scale_px(CLIENT_WIDTH, scale),
                bottom: scale_px(HEADER_HEIGHT, scale),
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
        WM_DPICHANGED => {
            let new_dpi = (wparam & 0xFFFF) as u32;
            apply_studio_dpi(hwnd, new_dpi);
            if lparam != 0 {
                // SAFETY: For WM_DPICHANGED, lparam points to the RECT Windows suggests for the
                // window at the new DPI; it is valid for the duration of this message.
                let suggested = unsafe { *(lparam as *const RECT) };
                // SAFETY: Moves/resizes this top-level window to the suggested frame.
                unsafe {
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            // SAFETY: Repaints cards, header, and children at the new scale.
            unsafe { windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 1) };
            0
        }
        WM_ACTIVATE => {
            if (wparam & 0xFFFF) as u32 == WA_INACTIVE {
                // SAFETY: GetFocus/IsChild only query focus and window relationships.
                let focused = unsafe { GetFocus() };
                if !focused.is_null() && unsafe { IsChild(hwnd, focused) } != 0 {
                    LAST_FOCUS.with(|cell| cell.set(focused));
                }
            } else {
                let saved = LAST_FOCUS.with(Cell::get);
                // SAFETY: Restores focus only to a live child of this window, like a dialog.
                if !saved.is_null() && unsafe { IsChild(hwnd, saved) } != 0 {
                    unsafe { SetFocus(saved) };
                    return 0;
                }
            }
            // SAFETY: Default activation handling for every other case.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_QUERYENDSESSION => {
            // Windows is signing out or shutting down: persist settings now, while it is safe.
            save_config_now();
            1
        }
        WM_ENDSESSION => {
            if wparam != 0 {
                // The session is ending and the process may be terminated after this returns,
                // so stop playback, remove the hosts and tray icon, and save.
                stop_in_app_preview();
                GUI_STATE.with(|cell| {
                    if let Ok(mut guard) = cell.try_borrow_mut() {
                        if let Some(state) = guard.as_mut() {
                            let _ = state.engine.config.save_to_file(&state.config_path);
                            state.engine.shutdown();
                        }
                    }
                });
                remove_tray_icon(hwnd);
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
        .map(|d| display_combo_label(&d.id, &d.name, d.width, d.height, d.is_primary))
        .collect()
}

/// Human-readable display selector entry. The device name is omitted when it only repeats the
/// display id (for example `\\.\DISPLAY1` for `DISPLAY1`).
fn display_combo_label(id: &str, name: &str, width: u32, height: u32, primary: bool) -> String {
    let mut label = id.to_string();
    if !name.is_empty() && !name.ends_with(id) {
        label.push_str(&format!(" ({name})"));
    }
    label.push_str(&format!(", {width} \u{d7} {height}"));
    if primary {
        label.push_str(", primary");
    }
    label
}

/// Replace an EDIT control's text and scroll it so the end (the file name) stays visible.
fn set_edit_text_showing_end(hwnd_edit: HWND, text: &str) {
    if hwnd_edit.is_null() {
        return;
    }
    let wide = wide_string(text);
    let end = wide.len().saturating_sub(1);
    // SAFETY: hwnd_edit is a live EDIT child; EM_SETSEL moves the caret to the end of its text.
    unsafe {
        SetWindowTextW(hwnd_edit, wide.as_ptr());
        SendMessageW(hwnd_edit, EM_SETSEL, end, end as LPARAM);
    }
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

/// Logical (96 DPI) `(x, y, width, height)` rectangle on the Studio layout grid.
type LogicalRect = (i32, i32, i32, i32);

/// Which Studio font a control uses; fonts are recreated whenever the DPI changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FontRole {
    Body,
    Section,
}

/// Which `GuiState` handle a control fills, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    None,
    Preview,
    PreviewEmpty,
    BtnPreview,
    BtnStopPreview,
    CmbDisplay,
    EdtPath,
    BtnNextRecent,
    NowFile,
    NowMeta,
    BtnApply,
    BtnPauseResume,
    BtnStop,
    ChkMute,
    ChkLoop,
    BtnAspect,
    BtnVolDown,
    LblVolume,
    BtnVolUp,
    BtnSpeedDown,
    LblSpeed,
    BtnSpeedUp,
    ChkFullscreen,
    ChkBattery,
    ChkStartup,
    ChkReduceMotion,
    LblStatus,
    LblStatusDetails,
}

/// One child control of the Studio window. The table below is the single source of truth for
/// creation, keyboard (Tab) order, and DPI re-layout.
#[derive(Debug)]
struct ControlSpec {
    class: &'static str,
    text: &'static str,
    style: u32,
    rect: LogicalRect,
    id: usize,
    slot: Slot,
    font: FontRole,
    /// Painted in the secondary text tone.
    secondary: bool,
}

const PUSH: u32 = BS_PUSHBUTTON as u32 | WS_TABSTOP;
const CHECK: u32 = BS_AUTOCHECKBOX as u32 | WS_TABSTOP;
const LABEL: u32 = SS_NOPREFIX;

const fn control(
    class: &'static str,
    text: &'static str,
    style: u32,
    rect: LogicalRect,
    id: usize,
    slot: Slot,
) -> ControlSpec {
    ControlSpec {
        class,
        text,
        style,
        rect,
        id,
        slot,
        font: FontRole::Body,
        secondary: false,
    }
}

const fn section(text: &'static str, rect: LogicalRect) -> ControlSpec {
    ControlSpec {
        font: FontRole::Section,
        ..control("STATIC", text, LABEL, rect, 0, Slot::None)
    }
}

const fn secondary(text: &'static str, style: u32, rect: LogicalRect, slot: Slot) -> ControlSpec {
    ControlSpec {
        secondary: true,
        ..control("STATIC", text, style, rect, 0, slot)
    }
}

/// Studio controls in creation order. Windows moves keyboard focus in creation (z) order, so
/// this order is also the Tab order: preview, wallpaper source, playback, behavior, status;
/// top to bottom and left to right inside each card.
static CONTROL_LAYOUT: &[ControlSpec] = &[
    // Preview card (24, 56)..(584, 464). Inner column x 48..560.
    section("Preview", (48, 72, 200, 24)),
    // 512 x 288 is an exact 16:9 surface.
    control(
        PREVIEW_CLASS_NAME,
        "",
        WS_BORDER | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        (48, 104, 512, 288),
        0,
        Slot::Preview,
    ),
    control(
        "STATIC",
        "Choose a video to see it in motion",
        SS_CENTER | SS_NOPREFIX,
        (64, 240, 480, 48),
        0,
        Slot::PreviewEmpty,
    ),
    control(
        "BUTTON",
        "Preview",
        PUSH,
        (48, 408, 252, 36),
        ID_BTN_PREVIEW,
        Slot::BtnPreview,
    ),
    control(
        "BUTTON",
        "Stop preview",
        PUSH,
        (308, 408, 252, 36),
        ID_BTN_STOP_PREVIEW,
        Slot::BtnStopPreview,
    ),
    // Wallpaper card (600, 56)..(1136, 464). Inner column x 624..1112.
    section("Wallpaper", (624, 72, 200, 24)),
    secondary("Display", LABEL, (624, 104, 200, 20), Slot::None),
    control(
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | CBS_OWNERDRAWFIXED as u32 | CBS_HASSTRINGS as u32 | WS_TABSTOP,
        (624, 128, 488, 32),
        ID_CMB_DISPLAY,
        Slot::CmbDisplay,
    ),
    secondary("Video file", LABEL, (624, 168, 200, 20), Slot::None),
    // Borderless EDIT inset inside PATH_FIELD_RECT (624, 192)..(880, 228).
    control(
        "EDIT",
        "",
        WS_TABSTOP | ES_AUTOHSCROLL as u32,
        (636, 200, 232, 20),
        ID_EDT_PATH,
        Slot::EdtPath,
    ),
    control(
        "BUTTON",
        "Browse",
        PUSH,
        (888, 192, 108, 36),
        ID_BTN_BROWSE,
        Slot::None,
    ),
    control(
        "BUTTON",
        "Next recent",
        PUSH,
        (1004, 192, 108, 36),
        ID_BTN_NEXT_RECENT,
        Slot::BtnNextRecent,
    ),
    secondary(
        "Or drop a video file anywhere on this window.",
        LABEL,
        (624, 240, 488, 20),
        Slot::None,
    ),
    // What the selected display is showing right now.
    secondary("On this display", LABEL, (624, 272, 488, 20), Slot::None),
    secondary(
        "",
        LABEL | SS_ENDELLIPSIS,
        (624, 296, 488, 20),
        Slot::NowFile,
    ),
    secondary(
        "",
        LABEL | SS_ENDELLIPSIS,
        (624, 320, 488, 20),
        Slot::NowMeta,
    ),
    control(
        "BUTTON",
        "Set as wallpaper",
        PUSH,
        (624, 356, 488, 44),
        ID_BTN_APPLY,
        Slot::BtnApply,
    ),
    control(
        "BUTTON",
        "Pause wallpaper",
        PUSH,
        (624, 408, 240, 36),
        ID_BTN_PAUSE_RESUME,
        Slot::BtnPauseResume,
    ),
    control(
        "BUTTON",
        "Stop wallpaper",
        PUSH,
        (872, 408, 240, 36),
        ID_BTN_STOP,
        Slot::BtnStop,
    ),
    // Playback card (24, 480)..(584, 632).
    section("Playback", (48, 496, 200, 24)),
    control(
        "BUTTON",
        "Mute audio",
        CHECK,
        (48, 532, 128, 28),
        ID_CHK_MUTE,
        Slot::ChkMute,
    ),
    control(
        "BUTTON",
        "Loop video",
        CHECK,
        (184, 532, 128, 28),
        ID_CHK_LOOP,
        Slot::ChkLoop,
    ),
    control(
        "BUTTON",
        "Scale: Fill",
        PUSH,
        (400, 528, 160, 36),
        ID_BTN_ASPECT,
        Slot::BtnAspect,
    ),
    control(
        "BUTTON",
        "Volume down",
        PUSH,
        (48, 580, 36, 36),
        ID_BTN_VOL_DOWN,
        Slot::BtnVolDown,
    ),
    control(
        "STATIC",
        "Volume 0%",
        SS_CENTER | SS_NOPREFIX,
        (88, 588, 112, 20),
        0,
        Slot::LblVolume,
    ),
    control(
        "BUTTON",
        "Volume up",
        PUSH,
        (204, 580, 36, 36),
        ID_BTN_VOL_UP,
        Slot::BtnVolUp,
    ),
    control(
        "BUTTON",
        "Slower",
        PUSH,
        (368, 580, 36, 36),
        ID_BTN_SPEED_DOWN,
        Slot::BtnSpeedDown,
    ),
    control(
        "STATIC",
        "Speed 1.00×",
        SS_CENTER | SS_NOPREFIX,
        (408, 588, 112, 20),
        0,
        Slot::LblSpeed,
    ),
    control(
        "BUTTON",
        "Faster",
        PUSH,
        (524, 580, 36, 36),
        ID_BTN_SPEED_UP,
        Slot::BtnSpeedUp,
    ),
    // Behavior card (600, 480)..(1136, 632).
    section("Behavior", (624, 496, 200, 24)),
    control(
        "BUTTON",
        "Pause while a full-screen app is open",
        CHECK,
        (624, 528, 488, 24),
        ID_CHK_FULLSCREEN,
        Slot::ChkFullscreen,
    ),
    control(
        "BUTTON",
        "Pause on battery power",
        CHECK,
        (624, 552, 488, 24),
        ID_CHK_BATTERY,
        Slot::ChkBattery,
    ),
    control(
        "BUTTON",
        "Start Auralis when I sign in",
        CHECK,
        (624, 576, 488, 24),
        ID_CHK_STARTUP,
        Slot::ChkStartup,
    ),
    control(
        "BUTTON",
        "Reduce motion",
        CHECK,
        (624, 600, 488, 24),
        ID_CHK_REDUCE_MOTION,
        Slot::ChkReduceMotion,
    ),
    // Status card (24, 648)..(1136, 732).
    ControlSpec {
        font: FontRole::Section,
        ..control(
            "STATIC",
            "Wallpaper engine starting",
            LABEL,
            (48, 660, 720, 20),
            0,
            Slot::LblStatus,
        )
    },
    secondary("", LABEL, (48, 680, 720, 40), Slot::LblStatusDetails),
    control(
        "BUTTON",
        "Hide to tray",
        PUSH,
        (784, 668, 144, 36),
        ID_BTN_HIDE,
        Slot::None,
    ),
    control(
        "BUTTON",
        "Quit and restore desktop",
        PUSH,
        (936, 668, 176, 36),
        ID_BTN_QUIT,
        Slot::None,
    ),
];

fn assign_slot(state: &mut GuiState, slot: Slot, hwnd: HWND) {
    let target = match slot {
        Slot::None => return,
        Slot::Preview => &mut state.hwnd_preview,
        Slot::PreviewEmpty => &mut state.lbl_preview_empty,
        Slot::BtnPreview => &mut state.btn_preview,
        Slot::BtnStopPreview => &mut state.btn_stop_preview,
        Slot::CmbDisplay => &mut state.cmb_display,
        Slot::EdtPath => &mut state.edt_path,
        Slot::BtnNextRecent => &mut state.btn_next_recent,
        Slot::NowFile => &mut state.lbl_now_file,
        Slot::NowMeta => &mut state.lbl_now_meta,
        Slot::BtnApply => &mut state.btn_apply,
        Slot::BtnPauseResume => &mut state.btn_pause_resume,
        Slot::BtnStop => &mut state.btn_stop,
        Slot::ChkMute => &mut state.chk_mute,
        Slot::ChkLoop => &mut state.chk_loop,
        Slot::BtnAspect => &mut state.btn_aspect,
        Slot::BtnVolDown => &mut state.btn_vol_down,
        Slot::LblVolume => &mut state.lbl_volume,
        Slot::BtnVolUp => &mut state.btn_vol_up,
        Slot::BtnSpeedDown => &mut state.btn_speed_down,
        Slot::LblSpeed => &mut state.lbl_speed,
        Slot::BtnSpeedUp => &mut state.btn_speed_up,
        Slot::ChkFullscreen => &mut state.chk_fullscreen,
        Slot::ChkBattery => &mut state.chk_battery,
        Slot::ChkStartup => &mut state.chk_startup,
        Slot::ChkReduceMotion => &mut state.chk_reduce_motion,
        Slot::LblStatus => &mut state.lbl_status,
        Slot::LblStatusDetails => &mut state.lbl_status_details,
    };
    *target = hwnd;
}

/// Height of each display-selector row and of its closed selection field, in logical pixels.
const DISPLAY_ITEM_HEIGHT: i32 = 30;

fn font_for(role: FontRole, ui: HFONT, section: HFONT) -> HFONT {
    match role {
        FontRole::Body => ui,
        FontRole::Section => section,
    }
}

fn create_child_controls(parent: HWND) {
    let (_, scale) = query_dpi_scale();
    // SAFETY: GetModuleHandleW retrieves the current process module handle.
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let (ui_font, section_font) = GUI_STATE
        .with(|cell| {
            cell.try_borrow()
                .ok()
                .and_then(|g| g.as_ref().map(|s| (s.ui_font, s.section_font)))
        })
        .unwrap_or((std::ptr::null_mut(), std::ptr::null_mut()));
    // SAFETY: GetStockObject returns a process-owned stock font used only as a fallback.
    let fallback = unsafe { GetStockObject(DEFAULT_GUI_FONT) } as HFONT;
    let ui_font = if ui_font.is_null() { fallback } else { ui_font };
    let section_font = if section_font.is_null() {
        fallback
    } else {
        section_font
    };

    let mut created: Vec<(HWND, &'static ControlSpec)> = Vec::with_capacity(CONTROL_LAYOUT.len());
    for spec in CONTROL_LAYOUT {
        let mut style = spec.style;
        if spec.class == "BUTTON" && style & 0x000F == BS_PUSHBUTTON as u32 {
            style = (style & !(BS_PUSHBUTTON as u32)) | BS_OWNERDRAW as u32;
        }
        let ex_style = if spec.class == "STATIC" || style & 0x000F == BS_AUTOCHECKBOX as u32 {
            WS_EX_TRANSPARENT
        } else {
            0
        };
        let class = wide_string(spec.class);
        let text = wide_string(spec.text);
        let (x, y, width, height) = scale_rect(spec.rect, scale);
        // SAFETY: Creates a DPI-scaled child control of the live Studio window and applies the
        // application font. The id is passed through the HMENU parameter as Win32 requires.
        let ctrl = unsafe {
            CreateWindowExW(
                ex_style,
                class.as_ptr(),
                text.as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                x,
                y,
                width,
                height,
                parent,
                spec.id as *mut _,
                hinstance,
                std::ptr::null(),
            )
        };
        if ctrl.is_null() {
            continue;
        }
        let font = font_for(spec.font, ui_font, section_font);
        // SAFETY: Applies a long-lived Studio font to the new child control.
        unsafe { SendMessageW(ctrl, WM_SETFONT, font as WPARAM, 1) };
        if is_checkbox_control_id(spec.id) {
            let subclass_proc: SUBCLASSPROC = Some(checkbox_subclass_proc);
            // SAFETY: The subclass only custom-paints this native checkbox; all input and
            // accessibility messages continue through DefSubclassProc.
            if unsafe { SetWindowSubclass(ctrl, subclass_proc, spec.id, 0) } == 0 {
                eprintln!(
                    "Auralis could not apply checkbox styling to control {}.",
                    spec.id
                );
            }
        }
        created.push((ctrl, spec));
    }

    let mut combo = std::ptr::null_mut();
    let mut labels = Vec::new();
    GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(state) = guard.as_mut() {
                for (hwnd, spec) in &created {
                    assign_slot(state, spec.slot, *hwnd);
                }
                state.secondary_labels = created
                    .iter()
                    .filter(|(_, spec)| spec.secondary)
                    .map(|(hwnd, _)| *hwnd)
                    .collect();
                state.controls = created;
                combo = state.cmb_display;
                labels = build_display_combo_labels(state);
            }
        }
    });
    apply_display_combo_labels(combo, &labels);
}

/// Move every child control to its layout rectangle at `scale` and give it the current fonts.
fn layout_controls(controls: &[(HWND, &'static ControlSpec)], scale: f32, fonts: StudioFonts) {
    for (hwnd, spec) in controls {
        let (x, y, width, height) = scale_rect(spec.rect, scale);
        let font = font_for(spec.font, fonts.ui, fonts.section);
        // SAFETY: Each handle is a live child of the Studio window; SetWindowPos only moves it
        // and WM_SETFONT swaps in a font that outlives the control.
        unsafe {
            SetWindowPos(
                *hwnd,
                std::ptr::null_mut(),
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            SendMessageW(*hwnd, WM_SETFONT, font as WPARAM, 1);
        }
        if spec.id == ID_CMB_DISPLAY {
            let item_height = scale_px(DISPLAY_ITEM_HEIGHT, scale) as LPARAM;
            // SAFETY: CB_SETITEMHEIGHT resizes the closed field (-1) and the list rows (0) of
            // the owner-drawn display selector.
            unsafe {
                SendMessageW(*hwnd, CB_SETITEMHEIGHT, usize::MAX, item_height);
                SendMessageW(*hwnd, CB_SETITEMHEIGHT, 0, item_height);
            }
        }
    }
}

/// Rebuild fonts and child layout for `new_dpi` (from `WM_DPICHANGED` or after creation).
fn apply_studio_dpi(hwnd: HWND, new_dpi: u32) {
    let new_dpi = new_dpi.max(96);
    let fonts = create_studio_fonts(new_dpi);
    let swapped = GUI_STATE.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return None;
        };
        let state = guard.as_mut()?;
        let old = StudioFonts {
            ui: state.ui_font,
            section: state.section_font,
            heading: state.heading_font,
        };
        state.ui_font = fonts.ui;
        state.section_font = fonts.section;
        state.heading_font = fonts.heading;
        state.dpi = new_dpi;
        Some((old, state.controls.clone()))
    });
    let Some((old_fonts, controls)) = swapped else {
        // The state is busy or gone; keep the current fonts and discard the new set.
        delete_studio_fonts(fonts);
        return;
    };
    STUDIO_DPI.with(|cell| cell.set(new_dpi));
    UI_FONT.with(|font| font.set(fonts.ui));
    layout_controls(&controls, dpi_scale(new_dpi), fonts);
    // Every control now uses the new fonts, so the previous set can be released.
    delete_studio_fonts(old_fonts);
    // SAFETY: Repaints the whole Studio at the new scale.
    unsafe { windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 1) };
}

/// Resize the Studio frame so its client area is the logical layout at `dpi`, keeping the
/// window's top-left corner.
fn resize_frame_for_dpi(hwnd: HWND, dpi: u32) {
    let scale = dpi_scale(dpi);
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: scale_px(CLIENT_WIDTH, scale),
        bottom: scale_px(CLIENT_HEIGHT, scale),
    };
    let mut current: RECT = unsafe { mem::zeroed() };
    // SAFETY: Both calls only write into the local RECT values.
    unsafe {
        AdjustWindowRectEx(&mut frame, style, 0, WS_EX_CONTROLPARENT);
        GetWindowRect(hwnd, &mut current);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            current.left,
            current.top,
            frame.right - frame.left,
            frame.bottom - frame.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Press the focused owner-drawn push button, as Enter does in a dialog.
fn press_focused_button(hwnd: HWND) {
    // SAFETY: Focus and parent queries only read window-manager state.
    let focused = unsafe { GetFocus() };
    if focused.is_null() || unsafe { IsChild(hwnd, focused) } == 0 {
        return;
    }
    // SAFETY: Reads the control id of a live child window.
    let id = unsafe { GetDlgCtrlID(focused) } as usize;
    let is_push_button = CONTROL_LAYOUT.iter().any(|spec| {
        spec.id == id && spec.class == "BUTTON" && spec.style & 0x000F == BS_PUSHBUTTON as u32
    });
    if is_push_button && unsafe { IsWindowEnabled(focused) } != 0 {
        // SAFETY: BM_CLICK makes the button send its own BN_CLICKED to this window.
        unsafe { SendMessageW(focused, BM_CLICK, 0, 0) };
    }
}

fn save_config_now() {
    GUI_STATE.with(|cell| {
        if let Ok(guard) = cell.try_borrow() {
            if let Some(state) = guard.as_ref() {
                let _ = state.engine.config.save_to_file(&state.config_path);
            }
        }
    });
}

/// Re-layer wallpapers whose desktop host was lost (Explorer restarted) and refresh the UI.
fn recover_wallpaper_hosts() {
    let outcome = GUI_STATE.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return None;
        };
        let state = guard.as_mut()?;
        let outcome = state.engine.recover_detached_wallpaper_hosts();
        if outcome.rebuilt > 0 {
            state.engine.trim_working_set();
        }
        Some(outcome)
    });
    if outcome.is_some_and(|o| o.rebuilt > 0 || o.pending > 0) {
        sync_controls_from_state();
        update_status_readout();
    }
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
    // SAFETY: Redraws the static idle glyph when preview state changes; this does not start a decoder.
    unsafe {
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd_preview, std::ptr::null(), 1);
    }
    update_enabled_controls();
}

fn stop_in_app_preview() {
    let (old_preview, empty_state, hwnd_preview) = GUI_STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(state) = guard.as_mut() {
                state.preview_path = None;
                return (
                    state.preview_player.take(),
                    state.lbl_preview_empty,
                    state.hwnd_preview,
                );
            }
        }
        (None, std::ptr::null_mut(), std::ptr::null_mut())
    });
    if !empty_state.is_null() {
        // SAFETY: Restores the in-app preview hint after the media player is removed.
        unsafe { ShowWindow(empty_state, SW_SHOW) };
    }
    if let Some(mut preview) = old_preview {
        preview.shutdown();
        WindowsPlatformAdapter::trim_process_working_set();
    }
    if !hwnd_preview.is_null() {
        // SAFETY: The preview surface is a live child and must repaint its resource-saving idle state.
        unsafe {
            windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd_preview, std::ptr::null(), 1);
        }
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
            set_edit_text_showing_end(snap.edt_path, &path_str);
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

        let vol_txt = wide_string(&format!("Volume {}%", snap.volume));
        SetWindowTextW(snap.lbl_volume, vol_txt.as_ptr());

        if !snap.lbl_speed.is_null() {
            let rate = f32::from(snap.speed_percent) / 100.0;
            let spd_txt = wide_string(&format!("Speed {rate:.2}×"));
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
            state.lbl_preview_empty,
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
        preview_empty,
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
    let selected_path = read_edit_text(path_edit);
    let has_path = !selected_path.trim().is_empty();
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
        if !has_preview && !preview_empty.is_null() {
            let hint = preview_empty_hint(has_path);
            let wide = wide_string(hint);
            SetWindowTextW(preview_empty, wide.as_ptr());
        }
    }
}

fn preview_empty_hint(has_selected_path: bool) -> &'static str {
    if has_selected_path {
        "Preview is stopped to save resources\r\nSelect Preview when you are ready"
    } else {
        "Choose a video to see it in motion"
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
                // Sets the selected file path into the EDIT control outside any borrow.
                set_edit_text_showing_end(edt_hwnd, &chosen.to_string_lossy());
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

/// "On this display" readout lines: the file name (never the full path) and a line with the
/// video resolution and playback state of the selected display.
fn display_readout(
    state: Option<&PlaybackState>,
    resolution: Option<(u32, u32)>,
    host_rebuild_pending: bool,
) -> (String, String) {
    let file_name = |path: &Path| {
        path.file_name().map_or_else(
            || "Untitled video".to_string(),
            |name| name.to_string_lossy().to_string(),
        )
    };
    let with_resolution = |label: &str| match resolution {
        Some((width, height)) => {
            format!("{width} \u{d7} {height}, {}", label.to_lowercase())
        }
        None => label.to_string(),
    };
    match state {
        Some(
            PlaybackState::Playing { path, .. }
            | PlaybackState::Paused { path, .. }
            | PlaybackState::Loading { path },
        ) if host_rebuild_pending => (
            file_name(path),
            "Waiting for the desktop to come back, then it resumes".to_string(),
        ),
        Some(PlaybackState::Playing { path, .. }) => (file_name(path), with_resolution("Playing")),
        Some(PlaybackState::Paused { path, .. }) => (file_name(path), with_resolution("Paused")),
        Some(PlaybackState::Loading { path }) => (file_name(path), "Loading".to_string()),
        Some(PlaybackState::Stopped { path }) => (file_name(path), "Stopped".to_string()),
        Some(PlaybackState::Failed { path, .. }) => (
            file_name(path),
            "Could not play this video. Choose another file and set it again.".to_string(),
        ),
        Some(PlaybackState::Unloaded) | None => (
            "No wallpaper".to_string(),
            "Choose a video, then select Set as wallpaper".to_string(),
        ),
    }
}

/// Update a static label only when its text changed, avoiding needless repaints.
fn set_label_if_changed(hwnd: HWND, text: &str) {
    if hwnd.is_null() || read_edit_text(hwnd) == text {
        return;
    }
    let wide = wide_string(text);
    // SAFETY: hwnd is a live STATIC child; the text is updated outside any GUI_STATE borrow.
    unsafe { SetWindowTextW(hwnd, wide.as_ptr()) };
}

fn update_status_readout() {
    let mut readout_target: Option<(HWND, String, HWND, String)> = None;
    let update_target: Option<(HWND, String, HWND, String, HWND, &'static str)> = GUI_STATE.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return None;
        };
        let state = guard.as_mut()?;

        let display_id = selected_display_id(state);
        let playback_state = state.engine.controller(&display_id).map(|c| &c.state);
        let state_desc = playback_status_caption(playback_state);
        let (now_file, now_meta) = display_readout(
            playback_state,
            state
                .engine
                .video_telemetry(&display_id)
                .and_then(|telemetry| telemetry.native_resolution),
            state.engine.host_rebuild_pending(&display_id),
        );
        readout_target = Some((state.lbl_now_file, now_file, state.lbl_now_meta, now_meta));

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
            format!("Tray ready · {recent_count} recent")
        } else {
            format!("Close keeps playback active · {recent_count} recent")
        };

        let details = if let Some(err) = &state.last_error {
            format!(
                "{host_desc} · {video_meta} · {preview_desc}\r\n{perf_desc} · {power_desc} · Notice: {err}"
            )
        } else {
            format!(
                "{host_desc} · {video_meta} · {preview_desc}\r\n{perf_desc} · {power_desc} · {tray_hint}"
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

    if let Some((file_hwnd, file_text, meta_hwnd, meta_text)) = readout_target {
        set_label_if_changed(file_hwnd, &file_text);
        set_label_if_changed(meta_hwnd, &meta_text);
    }
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
    fn display_label_omits_redundant_device_name() {
        assert_eq!(
            display_combo_label("DISPLAY1", r"\\.\DISPLAY1", 1920, 1080, true),
            "DISPLAY1, 1920 \u{d7} 1080, primary"
        );
        assert_eq!(
            display_combo_label("DISPLAY2", "Studio Monitor", 2560, 1440, false),
            "DISPLAY2 (Studio Monitor), 2560 \u{d7} 1440"
        );
    }

    #[test]
    fn header_caption_names_the_live_display() {
        let path = PathBuf::from("clip.mp4");
        assert_eq!(
            header_status_caption(
                Some(&PlaybackState::Playing {
                    path,
                    position_secs: 0.0,
                }),
                "DISPLAY1"
            ),
            "Live on DISPLAY1"
        );
        assert_eq!(
            header_status_caption(None, "DISPLAY1"),
            "No wallpaper running"
        );
    }

    #[test]
    fn frame_is_centred_and_clamped_to_the_work_area() {
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1020,
        };
        assert_eq!(center_in_rect(work, 1456, 970), (232, 25, 1456, 970));
        assert_eq!(center_in_rect(work, 2000, 1200), (0, 0, 1920, 1020));
    }

    #[test]
    fn step_buttons_paint_glyphs_but_keep_their_ids_distinct() {
        assert_eq!(step_button_glyph(ID_BTN_VOL_DOWN), Some("\u{2212}"));
        assert_eq!(step_button_glyph(ID_BTN_SPEED_UP), Some("+"));
        assert_eq!(step_button_glyph(ID_BTN_APPLY), None);
    }

    #[test]
    fn brand_icon_is_a_contrasting_32_bit_monochrome_mark() {
        let (pixels, mask) = brand_icon_pixels();
        assert_eq!(pixels.len(), 32 * 32 * 4);
        assert_eq!(mask.len(), 32 * 4);
        assert!(pixels.iter().step_by(4).any(|shade| *shade == 242));
        assert!(pixels.iter().step_by(4).any(|shade| *shade == 24));
        assert!(pixels.iter().skip(3).step_by(4).any(|alpha| *alpha == 0));
    }

    #[test]
    fn icon_vector_distance_handles_zero_length_segments() {
        assert!(
            (point_segment_distance((2.0, 3.0), (1.0, 1.0), (1.0, 1.0)) - 2.236068).abs() < 0.001
        );
    }

    #[test]
    fn layout_rectangles_scale_with_dpi() {
        assert_eq!(
            scale_rect((624, 356, 488, 44), dpi_scale(96)),
            (624, 356, 488, 44)
        );
        assert_eq!(
            scale_rect((624, 356, 488, 44), dpi_scale(144)),
            (936, 534, 732, 66)
        );
        assert_eq!(
            scale_rect((48, 72, 200, 24), dpi_scale(120)),
            (60, 90, 250, 30)
        );
        // Sub-96 values never shrink the layout below its logical size.
        assert!((dpi_scale(72) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn font_heights_follow_point_size_and_dpi() {
        assert_eq!(font_height(11, 96), -14);
        assert_eq!(font_height(11, 144), -22);
        assert_eq!(font_height(14, 192), -37);
    }

    fn card_of(rect: LogicalRect) -> Option<usize> {
        let (x, y, width, height) = rect;
        CARD_RECTS.iter().position(|&(left, top, right, bottom)| {
            x >= left && y >= top && x + width <= right && y + height <= bottom
        })
    }

    #[test]
    fn every_control_sits_inside_one_card() {
        for spec in CONTROL_LAYOUT {
            assert!(
                card_of(spec.rect).is_some(),
                "{:?} ({}) is outside every card",
                spec.slot,
                spec.text
            );
        }
    }

    #[test]
    fn tab_order_follows_the_visual_order() {
        let stops: Vec<&ControlSpec> = CONTROL_LAYOUT
            .iter()
            .filter(|spec| spec.style & WS_TABSTOP != 0)
            .collect();
        assert!(stops.len() > 20);
        for pair in stops.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (card_a, card_b) = (card_of(a.rect).unwrap(), card_of(b.rect).unwrap());
            assert!(
                card_b >= card_a,
                "{:?} -> {:?} goes back a card",
                a.slot,
                b.slot
            );
            if card_a == card_b {
                let a_bottom = a.rect.1 + a.rect.3;
                let next_row = b.rect.1 >= a_bottom;
                let same_row_right = b.rect.1 < a_bottom && b.rect.0 > a.rect.0;
                assert!(
                    next_row || same_row_right,
                    "{:?} -> {:?} is not top-to-bottom, left-to-right",
                    a.slot,
                    b.slot
                );
            }
        }
        // Preview actions come first and the quit action is last.
        assert_eq!(stops.first().unwrap().id, ID_BTN_PREVIEW);
        assert_eq!(stops.last().unwrap().id, ID_BTN_QUIT);
    }

    #[test]
    fn display_readout_sits_between_the_drop_hint_and_the_apply_button() {
        let rect_of = |slot: Slot| {
            CONTROL_LAYOUT
                .iter()
                .find(|spec| spec.slot == slot)
                .unwrap()
                .rect
        };
        let apply = rect_of(Slot::BtnApply);
        let meta = rect_of(Slot::NowMeta);
        let file = rect_of(Slot::NowFile);
        assert!(file.1 >= 260 && file.1 + file.3 <= meta.1);
        assert!(meta.1 + meta.3 + 8 <= apply.1);
        // Layout positions stay on the 4px grid.
        for spec in CONTROL_LAYOUT {
            assert_eq!(spec.rect.1 % 4, 0, "{:?} is off the 4px grid", spec.slot);
        }
    }

    #[test]
    fn display_readout_shows_file_name_resolution_and_state() {
        let path = PathBuf::from(r"C:\private\clips\ocean.mp4");
        assert_eq!(
            display_readout(
                Some(&PlaybackState::Playing {
                    path: path.clone(),
                    position_secs: 3.0,
                }),
                Some((3840, 2160)),
                false
            ),
            (
                "ocean.mp4".to_string(),
                "3840 \u{d7} 2160, playing".to_string()
            )
        );
        assert_eq!(
            display_readout(
                Some(&PlaybackState::Paused {
                    path: path.clone(),
                    position_secs: 3.0,
                }),
                None,
                false
            )
            .1,
            "Paused"
        );
        assert_eq!(
            display_readout(
                Some(&PlaybackState::Stopped { path: path.clone() }),
                None,
                false
            ),
            ("ocean.mp4".to_string(), "Stopped".to_string())
        );
        assert_eq!(display_readout(None, None, false).0, "No wallpaper");
        let waiting = display_readout(
            Some(&PlaybackState::Playing {
                path: path.clone(),
                position_secs: 0.0,
            }),
            None,
            true,
        );
        assert!(waiting.1.starts_with("Waiting for the desktop"));
        let failed = display_readout(
            Some(&PlaybackState::Failed {
                path,
                error: "decoder".to_string(),
            }),
            None,
            false,
        );
        assert!(!failed.0.contains('\\'));
        assert!(failed.1.contains("Choose another file"));
    }

    #[test]
    fn preview_hint_explains_idle_and_selected_states() {
        assert_eq!(
            preview_empty_hint(false),
            "Choose a video to see it in motion"
        );
        assert_eq!(
            preview_empty_hint(true),
            "Preview is stopped to save resources\r\nSelect Preview when you are ready"
        );
    }

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
