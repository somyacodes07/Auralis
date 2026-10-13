//! Windows Media Foundation Hardware-Accelerated Video Player (`mfplay.dll`)
//!
//! Wraps the native Windows Media Foundation `IMFPMediaPlayer` COM interface to decode
//! and render hardware-accelerated video directly into a `WorkerW` wallpaper host `HWND`.

use super::PlatformError;
use std::ffi::c_void;
use std::path::Path;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

/// Media player state constants from `MFP_MEDIAPLAYER_STATE` in `mfplay.h`.
pub const MFP_STATE_EMPTY: u32 = 0;
pub const MFP_STATE_STOPPED: u32 = 1;
pub const MFP_STATE_PLAYING: u32 = 2;
pub const MFP_STATE_PAUSED: u32 = 3;
pub const MFP_STATE_SHUTDOWN: u32 = 4;

type HResult = i32;

type FnMfpCreateMediaPlayer = unsafe extern "system" fn(
    pwsz_url: *const u16,
    f_start_playback: i32,
    creation_options: u32,
    p_callback: *mut c_void,
    hwnd: HWND,
    pp_media_player: *mut *mut ImfpMediaPlayer,
) -> HResult;

#[repr(C)]
struct IUnknownVtbl {
    query_interface: unsafe extern "system" fn(
        this: *mut c_void,
        riid: *const c_void,
        ppv: *mut *mut c_void,
    ) -> HResult,
    add_ref: unsafe extern "system" fn(this: *mut c_void) -> u32,
    release: unsafe extern "system" fn(this: *mut c_void) -> u32,
}

#[repr(C)]
struct ImfpMediaItem {
    vtbl: *const IUnknownVtbl,
}

/// Complete COM virtual method table for `IMFPMediaPlayer` (`mfplay.h`).
#[repr(C)]
struct ImfpMediaPlayerVtbl {
    // IUnknown (0..=2)
    query_interface: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        riid: *const c_void,
        ppv: *mut *mut c_void,
    ) -> HResult,
    add_ref: unsafe extern "system" fn(this: *mut ImfpMediaPlayer) -> u32,
    release: unsafe extern "system" fn(this: *mut ImfpMediaPlayer) -> u32,

    // IMFPMediaPlayer (3..=38)
    play: unsafe extern "system" fn(this: *mut ImfpMediaPlayer) -> HResult,
    pause: unsafe extern "system" fn(this: *mut ImfpMediaPlayer) -> HResult,
    stop: unsafe extern "system" fn(this: *mut ImfpMediaPlayer) -> HResult,
    frame_step: unsafe extern "system" fn(this: *mut ImfpMediaPlayer) -> HResult,
    set_position: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        guid_position_type: *const c_void,
        pv_position_value: *const c_void,
    ) -> HResult,
    get_position: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        guid_position_type: *const c_void,
        pv_position_value: *mut c_void,
    ) -> HResult,
    get_duration: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        guid_position_type: *const c_void,
        pv_duration_value: *mut c_void,
    ) -> HResult,
    set_rate: unsafe extern "system" fn(this: *mut ImfpMediaPlayer, fl_rate: f32) -> HResult,
    get_rate: unsafe extern "system" fn(this: *mut ImfpMediaPlayer, pfl_rate: *mut f32) -> HResult,
    get_supported_rates: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        f_forward_direction: i32,
        pfl_slowest_rate: *mut f32,
        pfl_fastest_rate: *mut f32,
    ) -> HResult,
    get_state: unsafe extern "system" fn(this: *mut ImfpMediaPlayer, pe_state: *mut u32) -> HResult,
    create_media_item_from_url: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        pwsz_url: *const u16,
        f_sync: i32,
        dw_user_data: usize,
        pp_media_item: *mut *mut ImfpMediaItem,
    ) -> HResult,
    create_media_item_from_object: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        p_iunknown_obj: *mut c_void,
        f_sync: i32,
        dw_user_data: usize,
        pp_media_item: *mut *mut ImfpMediaItem,
    ) -> HResult,
    set_media_item: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        p_media_item: *mut ImfpMediaItem,
    ) -> HResult,
    clear_media_item: unsafe extern "system" fn(this: *mut ImfpMediaPlayer) -> HResult,
    get_media_item: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        pp_media_item: *mut *mut ImfpMediaItem,
    ) -> HResult,
    get_volume:
        unsafe extern "system" fn(this: *mut ImfpMediaPlayer, pfl_volume: *mut f32) -> HResult,
    set_volume: unsafe extern "system" fn(this: *mut ImfpMediaPlayer, fl_volume: f32) -> HResult,
    get_balance:
        unsafe extern "system" fn(this: *mut ImfpMediaPlayer, pfl_balance: *mut f32) -> HResult,
    set_balance: unsafe extern "system" fn(this: *mut ImfpMediaPlayer, fl_balance: f32) -> HResult,
    get_mute: unsafe extern "system" fn(this: *mut ImfpMediaPlayer, pf_mute: *mut i32) -> HResult,
    set_mute: unsafe extern "system" fn(this: *mut ImfpMediaPlayer, f_mute: i32) -> HResult,
    get_native_video_size: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        psz_video: *mut c_void,
        psz_ar_video: *mut c_void,
    ) -> HResult,
    get_ideal_video_size: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        psz_min: *mut c_void,
        psz_max: *mut c_void,
    ) -> HResult,
    set_video_source_rect: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        pnrc_source: *const c_void,
    ) -> HResult,
    get_video_source_rect:
        unsafe extern "system" fn(this: *mut ImfpMediaPlayer, pnrc_source: *mut c_void) -> HResult,
    set_aspect_ratio_mode:
        unsafe extern "system" fn(this: *mut ImfpMediaPlayer, dw_aspect_ratio_mode: u32) -> HResult,
    get_aspect_ratio_mode: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        pdw_aspect_ratio_mode: *mut u32,
    ) -> HResult,
    get_video_window:
        unsafe extern "system" fn(this: *mut ImfpMediaPlayer, phwnd_video: *mut HWND) -> HResult,
    update_video: unsafe extern "system" fn(this: *mut ImfpMediaPlayer) -> HResult,
    set_border_color: unsafe extern "system" fn(this: *mut ImfpMediaPlayer, clr: u32) -> HResult,
    get_border_color:
        unsafe extern "system" fn(this: *mut ImfpMediaPlayer, p_clr: *mut u32) -> HResult,
    insert_effect: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        p_effect: *mut c_void,
        f_optional: i32,
    ) -> HResult,
    remove_effect:
        unsafe extern "system" fn(this: *mut ImfpMediaPlayer, p_effect: *mut c_void) -> HResult,
    remove_all_effects: unsafe extern "system" fn(this: *mut ImfpMediaPlayer) -> HResult,
    shutdown: unsafe extern "system" fn(this: *mut ImfpMediaPlayer) -> HResult,
}

#[repr(C)]
struct ImfpMediaPlayer {
    vtbl: *const ImfpMediaPlayerVtbl,
}

/// Native Windows Media Foundation video player bound to a specific host `HWND`.
pub struct MfVideoPlayer {
    /// Raw COM pointer stored as `usize` so the wrapper can be held inside the platform adapter.
    player_ptr: usize,
}

impl std::fmt::Debug for MfVideoPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MfVideoPlayer")
            .field("player_ptr", &format_args!("0x{:X}", self.player_ptr))
            .finish()
    }
}

impl MfVideoPlayer {
    /// Dynamically resolve `MFPCreateMediaPlayer` from `mfplay.dll`.
    fn load_create_fn() -> Result<FnMfpCreateMediaPlayer, PlatformError> {
        let dll_name: Vec<u16> = "mfplay.dll\0".encode_utf16().collect();
        // SAFETY: LoadLibraryW loads the system Media Foundation playback library.
        let h_mod = unsafe { LoadLibraryW(dll_name.as_ptr()) };
        if h_mod.is_null() {
            return Err(PlatformError::MediaPlaybackFailed(
                "Failed to load system library mfplay.dll".to_string(),
            ));
        }

        let proc_name = b"MFPCreateMediaPlayer\0";
        // SAFETY: GetProcAddress resolves the exported C function by null-terminated ASCII name.
        let proc_addr = unsafe { GetProcAddress(h_mod, proc_name.as_ptr()) };
        match proc_addr {
            Some(addr) => {
                // SAFETY: Signature matches `MFPCreateMediaPlayer` in `mfplay.h`.
                let func: FnMfpCreateMediaPlayer = unsafe { std::mem::transmute(addr) };
                Ok(func)
            }
            None => Err(PlatformError::MediaPlaybackFailed(
                "Could not resolve MFPCreateMediaPlayer in mfplay.dll".to_string(),
            )),
        }
    }

    /// Create a new hardware-accelerated Media Foundation player rendering into `window_hwnd`,
    /// synchronously load the video file at `path`, configure audio, and start playback.
    pub fn create_and_play(
        window_hwnd: isize,
        path: &Path,
        muted: bool,
        volume: u8,
    ) -> Result<Self, PlatformError> {
        if window_hwnd == 0 {
            return Err(PlatformError::MediaPlaybackFailed(
                "Cannot attach Media Foundation player to null HWND".to_string(),
            ));
        }

        // Ensure COM is initialized on the calling thread
        // SAFETY: CoInitializeEx is safe to call multiple times on a thread; S_OK (0) or S_FALSE (1)
        // or RPC_E_CHANGED_MODE are handled gracefully.
        unsafe {
            let _ = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        }

        let create_fn = Self::load_create_fn()?;

        let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let clean_path_str = abs_path
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string();
        let wide_url: Vec<u16> = clean_path_str
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut player: *mut ImfpMediaPlayer = std::ptr::null_mut();

        // SAFETY: Creates the IMFPMediaPlayer instance bound to `window_hwnd` without starting playback yet.
        let hr = unsafe {
            create_fn(
                std::ptr::null(),
                0, // FALSE: load item synchronously in next step
                0,
                std::ptr::null_mut(),
                window_hwnd as HWND,
                &mut player,
            )
        };

        if hr < 0 || player.is_null() {
            return Err(PlatformError::MediaPlaybackFailed(format!(
                "MFPCreateMediaPlayer failed with HRESULT 0x{:08X}",
                hr as u32
            )));
        }

        let instance = Self {
            player_ptr: player as usize,
        };

        // Load media item synchronously so invalid/corrupt files are caught immediately
        let mut item: *mut ImfpMediaItem = std::ptr::null_mut();
        // SAFETY: Calls CreateMediaItemFromURL with fSync = 1 (TRUE) on valid COM interface.
        let hr_item = unsafe {
            ((*(*player).vtbl).create_media_item_from_url)(
                player,
                wide_url.as_ptr(),
                1, // TRUE = synchronous creation
                0,
                &mut item,
            )
        };

        if hr_item < 0 || item.is_null() {
            return Err(PlatformError::MediaPlaybackFailed(format!(
                "Media Foundation rejected '{}': HRESULT 0x{:08X}",
                clean_path_str, hr_item as u32
            )));
        }

        // SAFETY: SetMediaItem attaches the loaded media item; we then release our local item ref.
        let hr_set = unsafe { ((*(*player).vtbl).set_media_item)(player, item) };
        unsafe {
            ((*(*item).vtbl).release)(item as *mut c_void);
        }

        if hr_set < 0 {
            return Err(PlatformError::MediaPlaybackFailed(format!(
                "SetMediaItem failed with HRESULT 0x{:08X}",
                hr_set as u32
            )));
        }

        // Configure borderless stretch-to-fill or aspect ratio and audio settings
        // SAFETY: Calls SetAspectRatioMode(0 = stretch to fill monitor) and SetBorderColor(black).
        unsafe {
            let _ = ((*(*player).vtbl).set_aspect_ratio_mode)(player, 0);
            let _ = ((*(*player).vtbl).set_border_color)(player, 0x0000_0000);
        }

        instance.set_audio(muted, volume)?;
        instance.play()?;

        Ok(instance)
    }

    #[inline]
    fn as_ptr(&self) -> *mut ImfpMediaPlayer {
        self.player_ptr as *mut ImfpMediaPlayer
    }

    /// Start or resume playback.
    pub fn play(&self) -> Result<(), PlatformError> {
        let player = self.as_ptr();
        if player.is_null() {
            return Err(PlatformError::MediaPlaybackFailed(
                "Media player is shut down".to_string(),
            ));
        }
        // SAFETY: Invokes IMFPMediaPlayer::Play on a valid COM interface pointer.
        let hr = unsafe { ((*(*player).vtbl).play)(player) };
        if hr < 0 {
            return Err(PlatformError::MediaPlaybackFailed(format!(
                "IMFPMediaPlayer::Play failed (0x{:08X})",
                hr as u32
            )));
        }
        Ok(())
    }

    /// Pause video playback.
    pub fn pause(&self) -> Result<(), PlatformError> {
        let player = self.as_ptr();
        if player.is_null() {
            return Ok(());
        }
        // SAFETY: Invokes IMFPMediaPlayer::Pause on a valid COM interface pointer.
        let hr = unsafe { ((*(*player).vtbl).pause)(player) };
        if hr < 0 {
            return Err(PlatformError::MediaPlaybackFailed(format!(
                "IMFPMediaPlayer::Pause failed (0x{:08X})",
                hr as u32
            )));
        }
        Ok(())
    }

    /// Stop video playback.
    pub fn stop(&self) -> Result<(), PlatformError> {
        let player = self.as_ptr();
        if player.is_null() {
            return Ok(());
        }
        // SAFETY: Invokes IMFPMediaPlayer::Stop on a valid COM interface pointer.
        let hr = unsafe { ((*(*player).vtbl).stop)(player) };
        if hr < 0 {
            return Err(PlatformError::MediaPlaybackFailed(format!(
                "IMFPMediaPlayer::Stop failed (0x{:08X})",
                hr as u32
            )));
        }
        Ok(())
    }

    /// Configure mute state and volume (`0..=100`).
    pub fn set_audio(&self, muted: bool, volume: u8) -> Result<(), PlatformError> {
        let player = self.as_ptr();
        if player.is_null() {
            return Ok(());
        }
        let clamped_vol = (f32::from(volume.min(100))) / 100.0;
        // SAFETY: Invokes SetMute and SetVolume on valid COM interface pointer.
        unsafe {
            let _ = ((*(*player).vtbl).set_mute)(player, i32::from(muted));
            let _ = ((*(*player).vtbl).set_volume)(player, clamped_vol);
        }
        Ok(())
    }

    /// Query the current `MFP_MEDIAPLAYER_STATE` (`0..=4`).
    pub fn state(&self) -> Result<u32, PlatformError> {
        let player = self.as_ptr();
        if player.is_null() {
            return Ok(MFP_STATE_SHUTDOWN);
        }
        let mut state: u32 = MFP_STATE_EMPTY;
        // SAFETY: Invokes GetState on valid COM interface pointer.
        let hr = unsafe { ((*(*player).vtbl).get_state)(player, &mut state) };
        if hr < 0 {
            return Err(PlatformError::MediaPlaybackFailed(format!(
                "IMFPMediaPlayer::GetState failed (0x{:08X})",
                hr as u32
            )));
        }
        Ok(state)
    }

    /// Periodic tick called by the runtime loop to enforce seamless looping and repaint video.
    pub fn tick(&self, loop_playback: bool) -> Result<(), PlatformError> {
        let player = self.as_ptr();
        if player.is_null() {
            return Ok(());
        }

        let current_state = self.state()?;
        if loop_playback && current_state == MFP_STATE_STOPPED {
            // Reached end of stream; Play() restarts from position 0
            self.play()?;
        }

        // SAFETY: UpdateVideo notifies the Enhanced Video Renderer to repaint if needed.
        unsafe {
            let _ = ((*(*player).vtbl).update_video)(player);
        }
        Ok(())
    }

    /// Explicitly shut down the Media Foundation session and release the COM object.
    pub fn shutdown(&mut self) {
        let player = self.as_ptr();
        if !player.is_null() {
            // SAFETY: Stops playback, shuts down the MFPlay session, and releases the COM reference.
            unsafe {
                let _ = ((*(*player).vtbl).stop)(player);
                let _ = ((*(*player).vtbl).shutdown)(player);
                ((*(*player).vtbl).release)(player);
            }
            self.player_ptr = 0;
        }
    }
}

impl Drop for MfVideoPlayer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::windows::WindowsPlatformAdapter;
    use crate::platform::PlatformAdapter;
    use std::io::Write;
    use std::path::PathBuf;

    #[test]
    fn test_mf_player_rejects_null_hwnd() {
        let res = MfVideoPlayer::create_and_play(0, Path::new("test.mp4"), true, 0);
        assert!(matches!(res, Err(PlatformError::MediaPlaybackFailed(_))));
    }

    #[test]
    fn test_mf_player_rejects_corrupt_mp4() {
        let mut adapter = WindowsPlatformAdapter::new();
        let displays = adapter.enumerate_displays().unwrap();
        if let Ok(hwnd) = adapter.create_wallpaper_host_window(&displays[0]) {
            let corrupt_path = std::env::temp_dir().join("auralis_corrupt_test.mp4");
            {
                let mut f = std::fs::File::create(&corrupt_path).unwrap();
                f.write_all(b"not a real mp4 container").unwrap();
            }

            let res = MfVideoPlayer::create_and_play(hwnd, &corrupt_path, true, 0);
            assert!(res.is_err());

            // Also verify system sample MP4 if present on this Windows machine
            let sample = PathBuf::from(
                r"C:\Windows\SystemApps\Microsoft.Windows.CloudExperienceHost_cw5n1h2txyewy\media\oobe-intro.mp4",
            );
            if sample.exists() {
                if let Ok(mut player) = MfVideoPlayer::create_and_play(hwnd, &sample, true, 50) {
                    assert!(player.pause().is_ok());
                    assert!(player.play().is_ok());
                    assert!(player.set_audio(false, 25).is_ok());
                    assert!(player.tick(true).is_ok());
                    player.shutdown();
                }
            }

            let _ = adapter.destroy_wallpaper_host_window(hwnd);
            let _ = std::fs::remove_file(corrupt_path);
        }
    }
}
