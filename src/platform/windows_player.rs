//! Windows Media Foundation Hardware-Accelerated Video Player (`mfplay.dll`)
//!
//! Wraps the native Windows Media Foundation `IMFPMediaPlayer` COM interface to decode
//! and render hardware-accelerated video directly into a `WorkerW` wallpaper host `HWND`.

use super::PlatformError;
use std::ffi::c_void;
use std::mem;
use std::path::Path;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

/// Media player state constants from `MFP_MEDIAPLAYER_STATE` in `mfplay.h`.
pub const MFP_STATE_EMPTY: u32 = 0;
pub const MFP_STATE_STOPPED: u32 = 1;
pub const MFP_STATE_PLAYING: u32 = 2;
pub const MFP_STATE_PAUSED: u32 = 3;
pub const MFP_STATE_SHUTDOWN: u32 = 4;

/// Variant type tag `VT_I8` (signed 64-bit integer) used for 100-nanosecond media positions.
const VT_I8: u16 = 20;

/// Threshold before end-of-stream (150ms in 100-nanosecond units) at which seamless loop
/// seeks back to position `0` while keeping the presentation clock active.
const LOOP_SEEK_THRESHOLD_100NS: i64 = 1_500_000;

type HResult = i32;

type FnMfpCreateMediaPlayer = unsafe extern "system" fn(
    pwsz_url: *const u16,
    f_start_playback: i32,
    creation_options: u32,
    p_callback: *mut c_void,
    hwnd: HWND,
    pp_media_player: *mut *mut ImfpMediaPlayer,
) -> HResult;

/// 16-byte GUID structure (`MFP_POSITIONTYPE_100NS` is `GUID::zeroed()`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

/// 24-byte `PROPVARIANT` layout for `VT_I8` (`LARGE_INTEGER` 100ns position/duration values).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PropVariantI64 {
    vt: u16,
    w_reserved1: u16,
    w_reserved2: u16,
    w_reserved3: u16,
    h_val: i64,
    padding: u64,
}

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
        guid_position_type: *const Guid,
        pv_position_value: *const PropVariantI64,
    ) -> HResult,
    get_position: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        guid_position_type: *const Guid,
        pv_position_value: *mut PropVariantI64,
    ) -> HResult,
    get_duration: unsafe extern "system" fn(
        this: *mut ImfpMediaPlayer,
        guid_position_type: *const Guid,
        pv_duration_value: *mut PropVariantI64,
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
    /// Cached total media duration in 100-nanosecond units, if known.
    duration_100ns: Option<i64>,
    /// Desired mute state.
    muted: bool,
    /// Desired volume (`0..=100`).
    volume: u8,
    /// Whether one-time EVR surface/aspect initialization has been applied after entering `PLAYING`.
    evr_initialized: bool,
}

impl std::fmt::Debug for MfVideoPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MfVideoPlayer")
            .field("player_ptr", &format_args!("0x{:X}", self.player_ptr))
            .field("duration_100ns", &self.duration_100ns)
            .field("muted", &self.muted)
            .field("volume", &self.volume)
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
                let func: FnMfpCreateMediaPlayer = unsafe { mem::transmute(addr) };
                Ok(func)
            }
            None => Err(PlatformError::MediaPlaybackFailed(
                "Could not resolve MFPCreateMediaPlayer in mfplay.dll".to_string(),
            )),
        }
    }

    /// Pump pending thread messages briefly so MFPlay can process asynchronous `SetMediaItem`
    /// topology binding on the apartment thread.
    fn pump_pending_messages() {
        // SAFETY: Standard non-blocking PeekMessageW loop for the current thread.
        unsafe {
            let mut msg: MSG = mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
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
        // SAFETY: CoInitializeEx is safe to call multiple times on a thread.
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

        let mut instance = Self {
            player_ptr: player as usize,
            duration_100ns: None,
            muted,
            volume: volume.min(100),
            evr_initialized: false,
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

        // SAFETY: SetMediaItem attaches the loaded media item to the MFPlay session.
        let hr_set = unsafe { ((*(*player).vtbl).set_media_item)(player, item) };
        // SAFETY: Release our local reference to `item` now that the session holds its own reference.
        unsafe {
            ((*(*item).vtbl).release)(item as *mut c_void);
        }

        if hr_set < 0 {
            return Err(PlatformError::MediaPlaybackFailed(format!(
                "SetMediaItem failed with HRESULT 0x{:08X}",
                hr_set as u32
            )));
        }

        // Allow MFPlay's internal worker message to finish binding the topology to EVR
        let wait_start = Instant::now();
        while wait_start.elapsed() < Duration::from_millis(350) {
            Self::pump_pending_messages();
            if instance.state().unwrap_or(MFP_STATE_EMPTY) != MFP_STATE_EMPTY {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        instance.apply_evr_and_audio_settings();
        instance.play()?;
        Self::pump_pending_messages();
        instance.apply_evr_and_audio_settings();
        instance.duration_100ns = instance.query_duration_100ns();

        Ok(instance)
    }

    #[inline]
    fn as_ptr(&self) -> *mut ImfpMediaPlayer {
        self.player_ptr as *mut ImfpMediaPlayer
    }

    /// Apply EVR full-window aspect mode (`0`), black border color, audio mute/volume,
    /// and a single `UpdateVideo` geometry sync.
    fn apply_evr_and_audio_settings(&self) {
        let player = self.as_ptr();
        if player.is_null() {
            return;
        }
        let clamped_vol = (f32::from(self.volume.min(100))) / 100.0;
        // SAFETY: Invokes EVR and audio configuration methods on valid IMFPMediaPlayer pointer.
        unsafe {
            let _ = ((*(*player).vtbl).set_aspect_ratio_mode)(player, 0);
            let _ = ((*(*player).vtbl).set_border_color)(player, 0x0000_0000);
            let _ = ((*(*player).vtbl).set_mute)(player, i32::from(self.muted));
            let _ = ((*(*player).vtbl).set_volume)(player, clamped_vol);
            let _ = ((*(*player).vtbl).update_video)(player);
        }
    }

    /// Query the total media duration in 100-nanosecond units (`MFP_POSITIONTYPE_100NS`).
    fn query_duration_100ns(&self) -> Option<i64> {
        let player = self.as_ptr();
        if player.is_null() {
            return None;
        }
        let pos_type = Guid::default();
        let mut prop = PropVariantI64::default();
        // SAFETY: GetDuration populates `prop` when the media item has a known duration.
        let hr = unsafe { ((*(*player).vtbl).get_duration)(player, &pos_type, &mut prop) };
        if hr >= 0 && prop.vt == VT_I8 && prop.h_val > 0 {
            Some(prop.h_val)
        } else {
            None
        }
    }

    /// Query the current playback position in 100-nanosecond units (`MFP_POSITIONTYPE_100NS`).
    fn query_position_100ns(&self) -> Option<i64> {
        let player = self.as_ptr();
        if player.is_null() {
            return None;
        }
        let pos_type = Guid::default();
        let mut prop = PropVariantI64::default();
        // SAFETY: GetPosition populates `prop` with the current presentation clock time.
        let hr = unsafe { ((*(*player).vtbl).get_position)(player, &pos_type, &mut prop) };
        if hr >= 0 && prop.vt == VT_I8 {
            Some(prop.h_val)
        } else {
            None
        }
    }

    /// Seek directly to a 100-nanosecond timestamp without stopping the presentation clock.
    fn seek_100ns(&self, position_100ns: i64) -> Result<(), PlatformError> {
        let player = self.as_ptr();
        if player.is_null() {
            return Ok(());
        }
        let pos_type = Guid::default();
        let prop = PropVariantI64 {
            vt: VT_I8,
            w_reserved1: 0,
            w_reserved2: 0,
            w_reserved3: 0,
            h_val: position_100ns.max(0),
            padding: 0,
        };
        // SAFETY: SetPosition seeks to `position_100ns` using `MFP_POSITIONTYPE_100NS`.
        let hr = unsafe { ((*(*player).vtbl).set_position)(player, &pos_type, &prop) };
        if hr < 0 {
            return Err(PlatformError::MediaPlaybackFailed(format!(
                "IMFPMediaPlayer::SetPosition failed (0x{:08X})",
                hr as u32
            )));
        }
        Ok(())
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
    pub fn set_audio(&mut self, muted: bool, volume: u8) -> Result<(), PlatformError> {
        self.muted = muted;
        self.volume = volume.min(100);
        let player = self.as_ptr();
        if player.is_null() {
            return Ok(());
        }
        let clamped_vol = (f32::from(self.volume)) / 100.0;
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

    /// Periodic tick called by the runtime loop to enforce seamless looping without calling
    /// `UpdateVideo` repeatedly during active hardware presentation (which would cause flicker).
    pub fn tick(&mut self, loop_playback: bool) -> Result<(), PlatformError> {
        let player = self.as_ptr();
        if player.is_null() {
            return Ok(());
        }

        let current_state = self.state()?;
        if current_state == MFP_STATE_PLAYING {
            if !self.evr_initialized {
                self.apply_evr_and_audio_settings();
                self.evr_initialized = true;
            }

            if !loop_playback {
                return Ok(());
            }

            let duration = self.duration_100ns.or_else(|| {
                let d = self.query_duration_100ns();
                self.duration_100ns = d;
                d
            });

            if let (Some(dur), Some(pos)) = (duration, self.query_position_100ns()) {
                if dur > LOOP_SEEK_THRESHOLD_100NS
                    && pos >= dur.saturating_sub(LOOP_SEEK_THRESHOLD_100NS)
                {
                    // Seamless seek back to 0 while clock is still in PLAYING state
                    let _ = self.seek_100ns(0);
                }
            }
        } else if loop_playback && current_state == MFP_STATE_STOPPED {
            // Fallback if a very short clip reached STOPPED state between ticks
            let _ = self.seek_100ns(0);
            self.play()?;
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
