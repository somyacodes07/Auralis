//! Native Windows Configuration UI & Event Loop (`windows_gui.rs`)
//!
//! Provides a lightweight, native Win32 Configuration Studio window and message loop
//! for selecting local videos, controlling playback/audio/looping, configuring power
//! and full-screen policies, and running the wallpaper engine behind desktop icons.

use super::windows::{string_from_wide, wide_string};
use crate::config::AppConfig;
use crate::engine::WallpaperEngine;
use crate::media::PlaybackState;
use crate::platform::{create_platform_adapter, PowerStatus};
use crate::VERSION;
use std::cell::RefCell;
use std::mem;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{GetStockObject, COLOR_WINDOW, DEFAULT_GUI_FONT, HBRUSH};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, GetMessageW,
    GetWindowTextLengthW, GetWindowTextW, KillTimer, PostQuitMessage, RegisterClassW, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowTextW, ShowWindow, TranslateMessage, BM_GETCHECK,
    BM_SETCHECK, BS_AUTOCHECKBOX, BS_PUSHBUTTON, CBS_DROPDOWNLIST, CB_ADDSTRING, CB_GETCURSEL,
    CB_SETCURSEL, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, ES_AUTOHSCROLL, MSG, SW_HIDE, SW_RESTORE,
    SW_SHOW, WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_SETFONT, WM_TIMER, WNDCLASSW,
    WS_BORDER, WS_CAPTION, WS_CHILD, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP,
    WS_VISIBLE,
};

const WINDOW_CLASS_NAME: &str = "AuralisConfigStudioWindowClass";
const WM_AURALIS_SHOW_UI: u32 = WM_APP + 1;

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

const ID_TIMER_TICK: usize = 1;
const TIMER_INTERVAL_MS: u32 = 400;

const BST_CHECKED: usize = 1;
const BST_UNCHECKED: usize = 0;

struct GuiState {
    engine: WallpaperEngine,
    config_path: PathBuf,
    cmb_display: HWND,
    edt_path: HWND,
    chk_mute: HWND,
    chk_loop: HWND,
    lbl_volume: HWND,
    chk_fullscreen: HWND,
    chk_battery: HWND,
    lbl_status: HWND,
    last_error: Option<String>,
}

thread_local! {
    static GUI_STATE: RefCell<Option<GuiState>> = const { RefCell::new(None) };
}

/// Launch the native Windows Configuration UI and enter the Win32 message & playback loop.
pub fn run_windows_gui(initial_video: Option<PathBuf>) -> Result<(), String> {
    // If an Auralis Studio instance is already running in the background, bring its UI to front
    let class_wide = wide_string(WINDOW_CLASS_NAME);
    // SAFETY: FindWindowW checks for an existing Auralis configuration window on the desktop.
    let existing = unsafe { FindWindowW(class_wide.as_ptr(), std::ptr::null()) };
    if !existing.is_null() && initial_video.is_none() {
        unsafe {
            SendMessageW(existing, WM_AURALIS_SHOW_UI, 0, 0);
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
        }
        return Ok(());
    }

    let config_path = PathBuf::from("auralis_config.json");
    let config = AppConfig::load_from_file(&config_path).unwrap_or_default();
    let adapter = create_platform_adapter();
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

    GUI_STATE.with(|cell| {
        *cell.borrow_mut() = Some(GuiState {
            engine,
            config_path,
            cmb_display: std::ptr::null_mut(),
            edt_path: std::ptr::null_mut(),
            chk_mute: std::ptr::null_mut(),
            chk_loop: std::ptr::null_mut(),
            lbl_volume: std::ptr::null_mut(),
            chk_fullscreen: std::ptr::null_mut(),
            chk_battery: std::ptr::null_mut(),
            lbl_status: std::ptr::null_mut(),
            last_error: None,
        });
    });

    // SAFETY: GetModuleHandleW(null) retrieves the current process module handle.
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };

    let wnd_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_wide.as_ptr(),
    };

    // SAFETY: RegisterClassW registers the main window class for this process.
    unsafe { RegisterClassW(&wnd_class) };

    let title = wide_string(&format!("Auralis Live Wallpaper Studio v{VERSION}"));
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE;

    // SAFETY: CreateWindowExW creates the main configuration window and triggers WM_CREATE.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_wide.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            620,
            470,
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
        if let Some(mut state) = cell.borrow_mut().take() {
            let _ = state.engine.config.save_to_file(&state.config_path);
            state.engine.shutdown();
        }
    });

    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            create_child_controls(hwnd);
            sync_controls_from_state();
            update_status_readout();
            // SAFETY: SetTimer schedules periodic engine policy & seamless loop ticks.
            unsafe { SetTimer(hwnd, ID_TIMER_TICK, TIMER_INTERVAL_MS, None) };
            0
        }
        WM_AURALIS_SHOW_UI => {
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                SetForegroundWindow(hwnd);
            }
            update_status_readout();
            0
        }
        WM_TIMER => {
            if wparam == ID_TIMER_TICK {
                GUI_STATE.with(|cell| {
                    if let Some(state) = cell.borrow_mut().as_mut() {
                        let _ = state.engine.tick();
                    }
                });
                update_status_readout();
            }
            0
        }
        WM_COMMAND => {
            let control_id = wparam & 0xFFFF;
            handle_command(hwnd, control_id);
            0
        }
        WM_CLOSE => {
            // AGENTS.md Principle 9: Closing the configuration window must not unexpectedly
            // stop an active wallpaper. If a wallpaper is playing, hide the window to background;
            // otherwise close and exit cleanly.
            let has_active = GUI_STATE.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .is_some_and(|s| s.engine.has_active_wallpaper())
            });

            if has_active {
                unsafe { ShowWindow(hwnd, SW_HIDE) };
            } else {
                unsafe { DestroyWindow(hwnd) };
            }
            0
        }
        WM_DESTROY => {
            unsafe {
                KillTimer(hwnd, ID_TIMER_TICK);
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn create_child_controls(parent: HWND) {
    // SAFETY: GetModuleHandleW and GetStockObject retrieve process instance and default GUI font.
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let hfont = unsafe { GetStockObject(DEFAULT_GUI_FONT) };

    let mk_ctrl =
        |class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: usize| -> HWND {
            let cls = wide_string(class);
            let txt = wide_string(text);
            // SAFETY: Creates a child Win32 control inside `parent` and applies the system UI font.
            unsafe {
                let ctrl = CreateWindowExW(
                    0,
                    cls.as_ptr(),
                    txt.as_ptr(),
                    WS_CHILD | WS_VISIBLE | style,
                    x,
                    y,
                    w,
                    h,
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
        "Auralis Live Wallpaper Studio — Native Hardware-Decoded Video Engine",
        0,
        20,
        16,
        560,
        22,
        0,
    );

    mk_ctrl("STATIC", "Target Monitor:", 0, 20, 50, 110, 22, 0);
    let cmb_display = mk_ctrl(
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
        135,
        46,
        445,
        200,
        ID_CMB_DISPLAY,
    );

    mk_ctrl("STATIC", "Video File Path:", 0, 20, 88, 110, 22, 0);
    let edt_path = mk_ctrl(
        "EDIT",
        "",
        WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        135,
        85,
        325,
        26,
        ID_EDT_PATH,
    );
    mk_ctrl(
        "BUTTON",
        "Browse Video...",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        470,
        84,
        110,
        28,
        ID_BTN_BROWSE,
    );

    mk_ctrl(
        "BUTTON",
        "Apply & Play Wallpaper",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        20,
        128,
        200,
        34,
        ID_BTN_APPLY,
    );
    mk_ctrl(
        "BUTTON",
        "Pause / Resume",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        235,
        128,
        165,
        34,
        ID_BTN_PAUSE_RESUME,
    );
    mk_ctrl(
        "BUTTON",
        "Stop & Clear",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        415,
        128,
        165,
        34,
        ID_BTN_STOP,
    );

    let chk_mute = mk_ctrl(
        "BUTTON",
        "Mute Audio",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        20,
        178,
        130,
        24,
        ID_CHK_MUTE,
    );
    let chk_loop = mk_ctrl(
        "BUTTON",
        "Seamless Loop",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        160,
        178,
        140,
        24,
        ID_CHK_LOOP,
    );
    mk_ctrl(
        "BUTTON",
        "Vol -",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        320,
        176,
        65,
        26,
        ID_BTN_VOL_DOWN,
    );
    let lbl_volume = mk_ctrl("STATIC", "Volume: 0%", 0, 395, 181, 110, 22, 0);
    mk_ctrl(
        "BUTTON",
        "Vol +",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        515,
        176,
        65,
        26,
        ID_BTN_VOL_UP,
    );

    let chk_fullscreen = mk_ctrl(
        "BUTTON",
        "Pause wallpaper when a Full-Screen App or Game is active",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        20,
        214,
        540,
        24,
        ID_CHK_FULLSCREEN,
    );
    let chk_battery = mk_ctrl(
        "BUTTON",
        "Pause wallpaper on Battery Power / Battery Saver",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        20,
        242,
        540,
        24,
        ID_CHK_BATTERY,
    );

    let lbl_status = mk_ctrl(
        "STATIC",
        "Initializing engine telemetry...",
        WS_BORDER,
        20,
        278,
        560,
        92,
        0,
    );

    mk_ctrl(
        "BUTTON",
        "Hide Window (Keep Wallpaper Playing)",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        20,
        382,
        270,
        34,
        ID_BTN_HIDE,
    );
    mk_ctrl(
        "BUTTON",
        "Quit Auralis & Restore Desktop",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        310,
        382,
        270,
        34,
        ID_BTN_QUIT,
    );

    GUI_STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut() {
            state.cmb_display = cmb_display;
            state.edt_path = edt_path;
            state.chk_mute = chk_mute;
            state.chk_loop = chk_loop;
            state.lbl_volume = lbl_volume;
            state.chk_fullscreen = chk_fullscreen;
            state.chk_battery = chk_battery;
            state.lbl_status = lbl_status;

            for d in state.engine.displays() {
                let tag = if d.is_primary { " [Primary]" } else { "" };
                let label = format!("{} — {} ({}x{}){}", d.id, d.name, d.width, d.height, tag);
                let wide = wide_string(&label);
                unsafe {
                    SendMessageW(cmb_display, CB_ADDSTRING, 0, wide.as_ptr() as LPARAM);
                }
            }
            unsafe {
                SendMessageW(cmb_display, CB_SETCURSEL, 0, 0);
            }
        }
    });
}

fn selected_display_id(state: &GuiState) -> String {
    let idx = unsafe { SendMessageW(state.cmb_display, CB_GETCURSEL, 0, 0) };
    let index = if idx >= 0 { idx as usize } else { 0 };
    state
        .engine
        .displays()
        .get(index)
        .or_else(|| state.engine.displays().first())
        .map(|d| d.id.clone())
        .unwrap_or_else(|| "DISPLAY_1".to_string())
}

fn sync_controls_from_state() {
    GUI_STATE.with(|cell| {
        if let Some(state) = cell.borrow().as_ref() {
            let display_id = selected_display_id(state);

            if let Some(cfg) = state.engine.config.per_display.get(&display_id).or(state
                .engine
                .config
                .default_wallpaper
                .as_ref())
            {
                let wide_path = wide_string(&cfg.media_path.to_string_lossy());
                unsafe { SetWindowTextW(state.edt_path, wide_path.as_ptr()) };
            }

            let (muted, loop_playback, volume) = state
                .engine
                .controller(&display_id)
                .map(|c| (c.muted, c.loop_playback, c.volume))
                .unwrap_or((true, true, 0));

            unsafe {
                SendMessageW(
                    state.chk_mute,
                    BM_SETCHECK,
                    if muted { BST_CHECKED } else { BST_UNCHECKED },
                    0,
                );
                SendMessageW(
                    state.chk_loop,
                    BM_SETCHECK,
                    if loop_playback {
                        BST_CHECKED
                    } else {
                        BST_UNCHECKED
                    },
                    0,
                );
                SendMessageW(
                    state.chk_fullscreen,
                    BM_SETCHECK,
                    if state.engine.config.pause_on_fullscreen {
                        BST_CHECKED
                    } else {
                        BST_UNCHECKED
                    },
                    0,
                );
                SendMessageW(
                    state.chk_battery,
                    BM_SETCHECK,
                    if state.engine.config.pause_on_battery {
                        BST_CHECKED
                    } else {
                        BST_UNCHECKED
                    },
                    0,
                );

                let vol_txt = wide_string(&format!("Volume: {volume}%"));
                SetWindowTextW(state.lbl_volume, vol_txt.as_ptr());
            }
        }
    });
}

fn read_edit_text(hwnd_edit: HWND) -> String {
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

    let chosen = unsafe { GetOpenFileNameW(&mut ofn) };
    if chosen != 0 {
        let selected = string_from_wide(&file_buf);
        if !selected.is_empty() {
            return Some(PathBuf::from(selected));
        }
    }
    None
}

fn handle_command(hwnd: HWND, control_id: usize) {
    match control_id {
        ID_BTN_BROWSE => {
            if let Some(chosen) = browse_video_file(hwnd) {
                GUI_STATE.with(|cell| {
                    if let Some(state) = cell.borrow().as_ref() {
                        let wide = wide_string(&chosen.to_string_lossy());
                        unsafe { SetWindowTextW(state.edt_path, wide.as_ptr()) };
                    }
                });
            }
        }
        ID_BTN_APPLY => {
            GUI_STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
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
            });
            sync_controls_from_state();
            update_status_readout();
        }
        ID_BTN_PAUSE_RESUME => {
            GUI_STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
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
            });
            update_status_readout();
        }
        ID_BTN_STOP => {
            GUI_STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    let display_id = selected_display_id(state);
                    let _ = state.engine.stop_display(&display_id);
                    state.last_error = None;
                }
            });
            update_status_readout();
        }
        ID_CHK_MUTE | ID_BTN_VOL_DOWN | ID_BTN_VOL_UP => {
            GUI_STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    let display_id = selected_display_id(state);
                    let checked = unsafe { SendMessageW(state.chk_mute, BM_GETCHECK, 0, 0) }
                        == BST_CHECKED as isize;
                    let cur_vol = state.engine.controller(&display_id).map_or(0, |c| c.volume);
                    let new_vol = match control_id {
                        ID_BTN_VOL_DOWN => cur_vol.saturating_sub(10),
                        ID_BTN_VOL_UP => (cur_vol + 10).min(100),
                        _ => cur_vol,
                    };
                    let new_mute = if control_id == ID_BTN_VOL_UP && new_vol > 0 && checked {
                        unsafe {
                            SendMessageW(state.chk_mute, BM_SETCHECK, BST_UNCHECKED, 0);
                        }
                        false
                    } else {
                        checked
                    };
                    let _ = state
                        .engine
                        .set_display_audio(&display_id, new_mute, new_vol);
                    let _ = state.engine.config.save_to_file(&state.config_path);
                    let vol_txt = wide_string(&format!("Volume: {new_vol}%"));
                    unsafe { SetWindowTextW(state.lbl_volume, vol_txt.as_ptr()) };
                }
            });
            update_status_readout();
        }
        ID_CHK_LOOP => {
            GUI_STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    let display_id = selected_display_id(state);
                    let checked = unsafe { SendMessageW(state.chk_loop, BM_GETCHECK, 0, 0) }
                        == BST_CHECKED as isize;
                    state.engine.set_display_loop(&display_id, checked);
                    let _ = state.engine.config.save_to_file(&state.config_path);
                }
            });
            update_status_readout();
        }
        ID_CHK_FULLSCREEN | ID_CHK_BATTERY => {
            GUI_STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    let fs_checked =
                        unsafe { SendMessageW(state.chk_fullscreen, BM_GETCHECK, 0, 0) }
                            == BST_CHECKED as isize;
                    let bat_checked = unsafe { SendMessageW(state.chk_battery, BM_GETCHECK, 0, 0) }
                        == BST_CHECKED as isize;
                    state.engine.config.pause_on_fullscreen = fs_checked;
                    state.engine.config.pause_on_battery = bat_checked;
                    let _ = state.engine.config.save_to_file(&state.config_path);
                    let _ = state.engine.evaluate_policy();
                }
            });
            update_status_readout();
        }
        ID_BTN_HIDE => {
            unsafe { ShowWindow(hwnd, SW_HIDE) };
        }
        ID_BTN_QUIT => {
            unsafe { DestroyWindow(hwnd) };
        }
        _ => {}
    }
}

fn update_status_readout() {
    GUI_STATE.with(|cell| {
        if let Some(state) = cell.borrow().as_ref() {
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
                _ => "IDLE (Select a video file and click 'Apply & Play Wallpaper')".to_string(),
            };

            let hwnd_desc = state
                .engine
                .host_window(&display_id)
                .map_or_else(|| "Detached".to_string(), |h| format!("WorkerW HWND 0x{h:X}"));

            let power_desc = match state.engine.query_power_status() {
                Ok(PowerStatus::OnAcPower) => "AC Mains Power".to_string(),
                Ok(PowerStatus::OnBattery {
                    percent,
                    battery_saver,
                }) => {
                    let pct = percent.map_or_else(|| "?%".to_string(), |p| format!("{p}%"));
                    format!("Battery ({pct}, Saver: {battery_saver})")
                }
                _ => "Unknown".to_string(),
            };

            let status_line = if let Some(err) = &state.last_error {
                format!(
                    "Status: {state_desc}\r\nLayering: {hwnd_desc} | Power: {power_desc}\r\nNotice: {err}"
                )
            } else {
                format!(
                    "Status: {state_desc}\r\nLayering: {hwnd_desc} | Power: {power_desc}\r\nTip: Closing or hiding this window keeps your active wallpaper playing."
                )
            };

            let wide = wide_string(&status_line);
            unsafe { SetWindowTextW(state.lbl_status, wide.as_ptr()) };
        }
    });
}
