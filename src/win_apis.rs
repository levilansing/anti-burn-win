use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
use winapi::shared::guiddef::GUID;
use winapi::shared::minwindef::{BOOL, DWORD, FALSE, HMODULE, LPVOID, TRUE, WORD};
use winapi::shared::windef::{HWND, POINT, RECT};
use winapi::shared::winerror::S_OK;
use winapi::shared::wtypesbase::CLSCTX_INPROC_SERVER;
use winapi::um::combaseapi::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL};
use winapi::um::dwmapi::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use winapi::um::libloaderapi::{GetProcAddress, LoadLibraryW};
use winapi::um::mmdeviceapi::{
    eCapture, eCommunications, IMMDevice, IMMDeviceEnumerator, CLSID_MMDeviceEnumerator,
};
use winapi::um::objbase::COINIT_APARTMENTTHREADED;
use winapi::um::sysinfoapi::GetTickCount64;
use winapi::um::unknwnbase::{IUnknown, IUnknownVtbl};
use winapi::um::winnt::HRESULT;
use winapi::um::winuser::{
    EnumDisplayMonitors, FindWindowW, GetAncestor, GetAsyncKeyState, GetClassNameW,
    GetForegroundWindow, GetLastInputInfo, GetMonitorInfoW, GetSystemMetrics, GetWindowLongW,
    GetWindowRect, GetWindowTextW, IsIconic, IsWindow, IsWindowVisible, MonitorFromWindow, GA_ROOT,
    GWL_EXSTYLE, LASTINPUTINFO, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, SM_CXSCREEN,
    SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    WS_EX_TOOLWINDOW,
};
use winapi::Interface;

#[repr(C)]
struct SYSTEMTIME {
    w_year: WORD,
    w_month: WORD,
    w_day_of_week: WORD,
    w_day: WORD,
    w_hour: WORD,
    w_minute: WORD,
    w_second: WORD,
    w_milliseconds: WORD,
}

extern "system" {
    fn GetLocalTime(lpSystemTime: *mut SYSTEMTIME);
}

#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(
        hKey: usize,
        lpSubKey: *const u16,
        Reserved: DWORD,
        lpClass: *mut u16,
        dwOptions: DWORD,
        samDesired: DWORD,
        lpSecurityAttributes: *mut winapi::ctypes::c_void,
        phkResult: *mut usize,
        lpdwDisposition: *mut DWORD,
    ) -> i32;
    fn RegSetValueExW(
        hKey: usize,
        lpValueName: *const u16,
        Reserved: DWORD,
        dwType: DWORD,
        lpData: *const u8,
        cbData: DWORD,
    ) -> i32;
    fn RegDeleteValueW(hKey: usize, lpValueName: *const u16) -> i32;
    fn RegCloseKey(hKey: usize) -> i32;
}

/// Registers or removes `AntiBurnOLEDGuard` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
/// so Anti-Burn OLED Guard launches automatically at Windows login (`--minimized` to tray).
pub fn sync_run_at_startup(enabled: bool) {
    let exe_path = match std::env::current_exe() {
        Ok(p) => {
            let s = p.to_string_lossy().to_string();
            // Skip registration when running inside cargo test runner binary
            if s.contains("deps\\anti_burn-") || s.contains("deps/anti_burn-") {
                return;
            }
            s.strip_prefix("\\\\?\\")
                .unwrap_or(&s)
                .replace(".old.exe", ".exe")
        }
        Err(_) => String::from("D:\\Projects\\anti-burn\\target\\release\\anti-burn.exe"),
    };

    let mut reg_succeeded = false;
    unsafe {
        const HKEY_CURRENT_USER: usize = 0x80000001;
        const KEY_SET_VALUE: DWORD = 0x0002;
        const REG_SZ: DWORD = 1;

        let subkey = to_wstring("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let val_name = to_wstring("AntiBurnOLEDGuard");
        let mut hkey: usize = 0;

        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            ptr::null_mut(),
            0,
            KEY_SET_VALUE,
            ptr::null_mut(),
            &mut hkey,
            ptr::null_mut(),
        ) == 0
            && hkey != 0
        {
            if enabled {
                let cmd = format!("\"{}\" --minimized", exe_path);
                let cmd_w = to_wstring(&cmd);
                if RegSetValueExW(
                    hkey,
                    val_name.as_ptr(),
                    0,
                    REG_SZ,
                    cmd_w.as_ptr() as *const u8,
                    (cmd_w.len() * 2) as DWORD,
                ) == 0
                {
                    reg_succeeded = true;
                }
            } else {
                let _ = RegDeleteValueW(hkey, val_name.as_ptr());
                reg_succeeded = true;
            }
            let _ = RegCloseKey(hkey);
        }
    }

    // Fallback to Windows Startup folder (`shell:startup`) if registry access was restricted
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let startup_cmd = PathBuf::from(appdata)
            .join("Microsoft\\Windows\\Start Menu\\Programs\\Startup\\AntiBurnOLEDGuard.cmd");
        if enabled && !reg_succeeded {
            let script = format!("@echo off\r\nstart \"\" \"{}\" --minimized\r\n", exe_path);
            let _ = fs::write(&startup_cmd, script);
        } else {
            let _ = fs::remove_file(&startup_cmd);
        }
    }
}

pub fn format_local_time() -> String {
    unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut st);
        format!(
            "{:02}:{:02}:{:02}.{:03}",
            st.w_hour, st.w_minute, st.w_second, st.w_milliseconds
        )
    }
}

pub fn append_idle_log(event: &str) {
    let timestamp = format_local_time();
    let line = format!("[{}] {}\n", timestamp, event);

    if let Some(appdata) = std::env::var_os("APPDATA") {
        let mut dir = PathBuf::from(appdata);
        dir.push("AntiBurnOLED");
        let _ = fs::create_dir_all(&dir);
        dir.push("idle_wake.log");
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&dir) {
            let _ = f.write_all(line.as_bytes());
        }
    }

    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("idle_wake.log")
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Requests 1ms timer resolution from the Windows kernel (`winmm.dll!timeBeginPeriod(1)`)
/// so our 8ms (125 FPS) animation loop never snaps to 15.6ms/31.2ms ticks.
pub fn enable_high_res_timer() {
    unsafe {
        let winmm = LoadLibraryW(to_wstring("winmm.dll").as_ptr());
        if !winmm.is_null() {
            type FnTimeBeginPeriod = unsafe extern "system" fn(u32) -> u32;
            let func: Option<FnTimeBeginPeriod> = std::mem::transmute(GetProcAddress(
                winmm,
                b"timeBeginPeriod\0".as_ptr() as *const i8,
            ));
            if let Some(f) = func {
                let _ = f(1);
            }
        }
    }
}

/// Enables Per-Monitor V2 DPI Awareness so `GetSystemMetrics`, `GetWindowRect`, and `GetCursorPos`
/// return true physical hardware pixels without DWM virtual scaling clipping or 1px right-edge gaps.
pub fn enable_dpi_awareness() {
    unsafe {
        let user32 = LoadLibraryW(to_wstring("user32.dll").as_ptr());
        if !user32.is_null() {
            type FnSetProcessDpiAwarenessContext = unsafe extern "system" fn(isize) -> BOOL;
            let func: Option<FnSetProcessDpiAwarenessContext> = std::mem::transmute(
                GetProcAddress(user32, b"SetProcessDpiAwarenessContext\0".as_ptr() as *const i8),
            );
            if let Some(f) = func {
                // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
                if f(-4) == TRUE {
                    return;
                }
            }
            type FnSetProcessDPIAware = unsafe extern "system" fn() -> BOOL;
            let fallback: Option<FnSetProcessDPIAware> = std::mem::transmute(GetProcAddress(
                user32,
                b"SetProcessDPIAware\0".as_ptr() as *const i8,
            ));
            if let Some(f) = fallback {
                let _ = f();
            }
        }
    }
}

/// Returns `(x, y, width, height)` of the primary monitor in physical pixels.
pub fn get_primary_monitor_rect() -> RECT {
    unsafe {
        let hmon = MonitorFromWindow(ptr::null_mut(), MONITOR_DEFAULTTOPRIMARY);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut mi) != 0 {
            mi.rcMonitor
        } else {
            RECT {
                left: 0,
                top: 0,
                right: GetSystemMetrics(SM_CXSCREEN).max(1920),
                bottom: GetSystemMetrics(SM_CYSCREEN).max(1080),
            }
        }
    }
}

pub fn get_virtual_screen_rect() -> RECT {
    unsafe {
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1920);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1080);
        RECT {
            left: vx,
            top: vy,
            right: vx + vw,
            bottom: vy + vh,
        }
    }
}

unsafe extern "system" fn enum_monitor_work_proc(
    hmon: winapi::shared::windef::HMONITOR,
    _hdc: winapi::shared::windef::HDC,
    _lprect: *mut RECT,
    lparam: winapi::shared::minwindef::LPARAM,
) -> BOOL {
    let vec = &mut *(lparam as *mut Vec<RECT>);
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(hmon, &mut mi) != 0 {
        let mut work = mi.rcWork;
        // Also check Shell_TrayWnd in case taskbar is on this monitor and rcWork wasn't reduced
        let tray = FindWindowW(to_wstring("Shell_TrayWnd").as_ptr(), ptr::null());
        if !tray.is_null() {
            let mut tr: RECT = std::mem::zeroed();
            if GetWindowRect(tray, &mut tr) != 0 && tr.right > tr.left && tr.bottom > tr.top {
                // If taskbar is docked at the bottom of this monitor, clamp work.bottom to tr.top
                if tr.left >= mi.rcMonitor.left - 4
                    && tr.right <= mi.rcMonitor.right + 4
                    && tr.top > mi.rcMonitor.top + 200
                    && tr.bottom <= mi.rcMonitor.bottom + 4
                {
                    work.bottom = work.bottom.min(tr.top);
                }
            }
        }
        if work.right > work.left && work.bottom > work.top {
            vec.push(work);
        }
    }
    TRUE
}

/// Returns the physical work-area rectangles (`rcWork`, excluding the taskbar!) of all active monitors.
/// By sizing `WindowDimmer` backdrops strictly to `rcWork`, the window backdrop NEVER overlaps or double-dims the taskbar!
pub fn get_all_monitor_work_rects() -> Vec<RECT> {
    unsafe {
        let mut rects: Vec<RECT> = Vec::new();
        let _ = EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(enum_monitor_work_proc),
            &mut rects as *mut Vec<RECT> as isize,
        );
        if rects.is_empty() {
            let m = get_primary_monitor_rect();
            rects.push(RECT {
                left: m.left,
                top: m.top,
                right: m.right,
                bottom: (m.bottom - 48).max(m.top + 100),
            });
        }
        rects
    }
}

pub fn to_wstring(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

// --- Magnification.dll dynamic FFI ---

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MAGCOLOREFFECT {
    pub transform: [[f32; 5]; 5],
}

impl MAGCOLOREFFECT {
    pub fn identity() -> Self {
        Self {
            transform: [
                [1.0, 0.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Creates a perceptual (sRGB gamma-aware) brightness + contrast color matrix.
    pub fn brightness_and_contrast(brightness_mult: f32, contrast: f32) -> Self {
        let b_linear = brightness_mult.clamp(0.01, 1.0).powf(1.85);
        let c = contrast.clamp(0.1, 1.0);
        let scale = b_linear * c;
        let offset = b_linear * 0.5 * (1.0 - c);
        Self {
            transform: [
                [scale, 0.0, 0.0, 0.0, 0.0],
                [0.0, scale, 0.0, 0.0, 0.0],
                [0.0, 0.0, scale, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
                [offset, offset, offset, 0.0, 1.0],
            ],
        }
    }
}

type FnMagInitialize = unsafe extern "system" fn() -> BOOL;
type FnMagUninitialize = unsafe extern "system" fn() -> BOOL;
type FnMagSetFullscreenColorEffect = unsafe extern "system" fn(*const MAGCOLOREFFECT) -> BOOL;

pub struct MagnificationApi {
    _module: HMODULE,
    pub mag_init: Option<FnMagInitialize>,
    pub mag_uninit: Option<FnMagUninitialize>,
    pub set_fullscreen_color: Option<FnMagSetFullscreenColorEffect>,
    pub initialized: bool,
    last_applied_mult: f32,
}

impl MagnificationApi {
    pub fn new() -> Self {
        unsafe {
            let name = to_wstring("Magnification.dll");
            let module = LoadLibraryW(name.as_ptr());
            if module.is_null() {
                return Self {
                    _module: ptr::null_mut(),
                    mag_init: None,
                    mag_uninit: None,
                    set_fullscreen_color: None,
                    initialized: false,
                    last_applied_mult: 1.0,
                };
            }
            let mag_init: Option<FnMagInitialize> = std::mem::transmute(GetProcAddress(
                module,
                b"MagInitialize\0".as_ptr() as *const i8,
            ));
            let mag_uninit: Option<FnMagUninitialize> = std::mem::transmute(GetProcAddress(
                module,
                b"MagUninitialize\0".as_ptr() as *const i8,
            ));
            let set_fullscreen_color: Option<FnMagSetFullscreenColorEffect> = std::mem::transmute(
                GetProcAddress(module, b"MagSetFullscreenColorEffect\0".as_ptr() as *const i8),
            );

            let initialized = if let Some(init_fn) = mag_init {
                init_fn() == TRUE
            } else {
                false
            };

            Self {
                _module: module,
                mag_init,
                mag_uninit,
                set_fullscreen_color,
                initialized,
                last_applied_mult: 1.0,
            }
        }
    }

    pub fn set_global_brightness(&mut self, brightness_mult: f32) -> bool {
        if !self.initialized {
            return false;
        }
        if (brightness_mult - self.last_applied_mult).abs() < 0.0005 {
            return true;
        }
        if let Some(func) = self.set_fullscreen_color {
            let effect = MAGCOLOREFFECT::brightness_and_contrast(brightness_mult, 1.0);
            if unsafe { func(&effect) == TRUE } {
                self.last_applied_mult = brightness_mult;
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn reset_global_brightness(&mut self) {
        if !self.initialized || (self.last_applied_mult - 1.0).abs() < 0.0005 {
            return;
        }
        if let Some(func) = self.set_fullscreen_color {
            let id = MAGCOLOREFFECT::identity();
            unsafe {
                let _ = func(&id);
            }
            self.last_applied_mult = 1.0;
        }
    }
}

impl Drop for MagnificationApi {
    fn drop(&mut self) {
        self.last_applied_mult = 0.0; // Force reset on exit
        self.reset_global_brightness();
        if self.initialized {
            if let Some(uninit) = self.mag_uninit {
                unsafe {
                    uninit();
                }
            }
        }
    }
}

// --- IAppVisibility COM Interface for Start Menu Detection ---

const CLSID_APP_VISIBILITY: GUID = GUID {
    Data1: 0x7E5FE3D9,
    Data2: 0x985F,
    Data3: 0x4908,
    Data4: [0x91, 0xF9, 0xEE, 0x19, 0xF9, 0xFD, 0x15, 0x14],
};

const IID_IAPP_VISIBILITY: GUID = GUID {
    Data1: 0x2246EA2D,
    Data2: 0xCAEA,
    Data3: 0x4444,
    Data4: [0xA3, 0xC4, 0x6D, 0xE8, 0x27, 0xE4, 0x43, 0x13],
};

#[repr(C)]
#[allow(non_snake_case)]
struct IAppVisibilityVtbl {
    parent: IUnknownVtbl,
    GetAppVisibilityOnMonitor: unsafe extern "system" fn(
        *mut IAppVisibility,
        *mut winapi::shared::windef::HMONITOR,
        *mut u32,
    ) -> HRESULT,
    IsLauncherVisible: unsafe extern "system" fn(*mut IAppVisibility, *mut BOOL) -> HRESULT,
    Advise: unsafe extern "system" fn(*mut IAppVisibility, *mut IUnknown, *mut DWORD) -> HRESULT,
    Unadvise: unsafe extern "system" fn(*mut IAppVisibility, DWORD) -> HRESULT,
}

#[repr(C)]
#[allow(non_snake_case)]
struct IAppVisibility {
    lpVtbl: *const IAppVisibilityVtbl,
}

// --- IAudioMeterInformation COM Interface for Video / Conference Call Detection ---

const IID_IAUDIO_METER_INFORMATION: GUID = GUID {
    Data1: 0xC02216F6,
    Data2: 0x8C67,
    Data3: 0x4B5B,
    Data4: [0x9D, 0x00, 0xD0, 0x08, 0xE7, 0x3E, 0x00, 0x64],
};

#[repr(C)]
#[allow(non_snake_case)]
struct IAudioMeterInformationVtbl {
    parent: IUnknownVtbl,
    GetPeakValue: unsafe extern "system" fn(*mut IAudioMeterInformation, *mut f32) -> HRESULT,
}

#[repr(C)]
#[allow(non_snake_case)]
struct IAudioMeterInformation {
    lpVtbl: *const IAudioMeterInformationVtbl,
}

pub struct ShellAndMediaDetector {
    app_visibility: *mut IAppVisibility,
    device_enumerator: *mut IMMDeviceEnumerator,
    last_media_active_tick: u64,
    last_audio_poll_tick: u64,
    pub last_render_peak: f32,
    pub last_capture_peak: f32,
    pub sustained_audio_secs: f32,
}

impl ShellAndMediaDetector {
    pub fn new() -> Self {
        unsafe {
            let _ = CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED);

            let mut app_visibility: *mut IAppVisibility = ptr::null_mut();
            let _ = CoCreateInstance(
                &CLSID_APP_VISIBILITY,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_IAPP_VISIBILITY,
                &mut app_visibility as *mut *mut _ as *mut LPVOID,
            );

            let mut device_enumerator: *mut IMMDeviceEnumerator = ptr::null_mut();
            let _ = CoCreateInstance(
                &CLSID_MMDeviceEnumerator,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IMMDeviceEnumerator::uuidof(),
                &mut device_enumerator as *mut *mut _ as *mut LPVOID,
            );

            Self {
                app_visibility,
                device_enumerator,
                last_media_active_tick: 0,
                last_audio_poll_tick: 0,
                last_render_peak: 0.0,
                last_capture_peak: 0.0,
                sustained_audio_secs: 0.0,
            }
        }
    }

    /// Returns true if the Start Menu, Search flyout, or System Tray overflow flyout is currently open.
    /// Does NOT treat `Shell_TrayWnd` (the taskbar itself) as a flyout so taskbar dimming continues
    /// working even when the taskbar holds foreground focus or when no application window is active!
    pub fn is_start_or_flyout_open(&self) -> bool {
        unsafe {
            if !self.app_visibility.is_null() {
                let mut visible: BOOL = FALSE;
                let hr = ((*(*self.app_visibility).lpVtbl).IsLauncherVisible)(
                    self.app_visibility,
                    &mut visible,
                );
                if hr == S_OK && visible == TRUE {
                    return true;
                }
            }

            let fg = GetForegroundWindow();
            if !fg.is_null() {
                let class_name = get_window_class(fg);
                if class_name == "XamlExplorerHostIslandWindow"
                    || class_name == "TopLevelWindowForOverflowXamlIsland"
                {
                    return true;
                }
                if class_name == "Windows.UI.Core.CoreWindow" {
                    let title = get_window_title(fg);
                    if title == "Start"
                        || title == "Search"
                        || title.contains("Shell Experience")
                        || title.contains("Action Center")
                    {
                        return true;
                    }
                }
            }
            false
        }
    }

    /// Polls CoreAudio microphone capture meter (throttled to 10Hz).
    /// Audio output (`eRender`) is intentionally ignored so background music, videos, or game audio
    /// NEVER pause or wake the idle timer. Only sustained active microphone speech (`capture > 0.045`
    /// for >= 2.0s) pauses the pre-idle timer when `pause_on_media_or_call` is enabled.
    pub fn is_watching_video_or_in_call(&mut self, dt_secs: f32) -> bool {
        unsafe {
            let now = GetTickCount64();
            if now.saturating_sub(self.last_audio_poll_tick) >= 100 {
                self.last_audio_poll_tick = now;
                self.last_render_peak = 0.0;
                self.last_capture_peak = self.get_endpoint_peak(eCapture, eCommunications);
            }

            if self.last_capture_peak > 0.045 {
                self.sustained_audio_secs = (self.sustained_audio_secs + dt_secs).min(8.0);
                if self.sustained_audio_secs >= 2.0 {
                    self.last_media_active_tick = now;
                    return true;
                }
            } else {
                self.sustained_audio_secs = (self.sustained_audio_secs - dt_secs * 1.5).max(0.0);
            }

            // Keep call state true for an 8-second grace window after sustained microphone speech
            if self.last_media_active_tick > 0
                && now.saturating_sub(self.last_media_active_tick) < 8_000
            {
                return true;
            }

            false
        }
    }

    unsafe fn get_endpoint_peak(&self, data_flow: u32, role: u32) -> f32 {
        if self.device_enumerator.is_null() {
            return 0.0;
        }
        let mut device: *mut IMMDevice = ptr::null_mut();
        let hr = (*self.device_enumerator).GetDefaultAudioEndpoint(data_flow, role, &mut device);
        if hr != S_OK || device.is_null() {
            return 0.0;
        }

        let mut meter: *mut IAudioMeterInformation = ptr::null_mut();
        let hr = (*device).Activate(
            &IID_IAUDIO_METER_INFORMATION,
            CLSCTX_ALL,
            ptr::null_mut(),
            &mut meter as *mut *mut _ as *mut LPVOID,
        );
        (*device).Release();

        if hr != S_OK || meter.is_null() {
            return 0.0;
        }

        let mut peak: f32 = 0.0;
        let _ = ((*(*meter).lpVtbl).GetPeakValue)(meter, &mut peak);
        ((*(*meter).lpVtbl).parent.Release)(meter as *mut IUnknown);
        peak
    }
}

impl Drop for ShellAndMediaDetector {
    fn drop(&mut self) {
        unsafe {
            if !self.app_visibility.is_null() {
                ((*(*self.app_visibility).lpVtbl).parent.Release)(
                    self.app_visibility as *mut IUnknown,
                );
            }
            if !self.device_enumerator.is_null() {
                (*self.device_enumerator).Release();
            }
        }
    }
}

// --- Helper functions for Input Idle Time & Window Frame Geometry ---

pub fn get_idle_duration_ms() -> u64 {
    unsafe {
        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut lii) != 0 {
            let now = GetTickCount64() as u32;
            now.wrapping_sub(lii.dwTime) as u64
        } else {
            0
        }
    }
}

/// Checks if any physical mouse button or keyboard key (`0x01..=0xFE`) is currently pressed
/// or was pressed since the last check. Returns the virtual key code (`VK_*`) if found.
pub fn detect_pressed_vk() -> Option<u32> {
    unsafe {
        for vk in 0x01i32..=0xFEi32 {
            // Skip toggle lock state bit, check high bit (0x8000 = currently down) or low bit (0x0001 = pressed)
            let state = GetAsyncKeyState(vk) as u16;
            if (state & 0x8001) != 0 {
                return Some(vk as u32);
            }
        }
        None
    }
}

pub fn get_window_class(hwnd: HWND) -> String {
    unsafe {
        if hwnd.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 256];
        let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len > 0 {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            String::new()
        }
    }
}

pub fn get_window_title(hwnd: HWND) -> String {
    unsafe {
        if hwnd.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 128];
        let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len > 0 {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            String::new()
        }
    }
}

/// Gets the exact visible DWM frame bounds (without the invisible 7px resize shadow border).
pub fn get_visible_window_rect(hwnd: HWND) -> Option<RECT> {
    unsafe {
        if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return None;
        }
        let mut cloaked: u32 = 0;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            std::mem::size_of::<u32>() as u32,
        );
        if cloaked != 0 {
            return None;
        }

        let mut rect: RECT = std::mem::zeroed();
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        );
        if hr != S_OK {
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return None;
            }
        }
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return None;
        }
        Some(rect)
    }
}

/// Normalizes an HWND to its top-level root window (`GA_ROOT`) and checks if it is a normal user
/// application window (not desktop, taskbar, shell flyout, or our own overlays).
pub fn resolve_eligible_active_window(raw_hwnd: HWND, own_overlay_hwnds: &[HWND]) -> Option<HWND> {
    unsafe {
        if raw_hwnd.is_null() {
            return None;
        }
        let root = GetAncestor(raw_hwnd, GA_ROOT);
        let mut hwnd = if !root.is_null() { root } else { raw_hwnd };

        if hwnd.is_null() || own_overlay_hwnds.contains(&hwnd) || own_overlay_hwnds.contains(&raw_hwnd) {
            return None;
        }
        if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return None;
        }
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if (ex_style & WS_EX_TOOLWINDOW) != 0 {
            // If an active popup/palette is a toolwindow owned by a main application window,
            // resolve to its root owner (`GA_ROOTOWNER` = 3) so the parent app stays the active window!
            let root_owner = GetAncestor(raw_hwnd, 3);
            if !root_owner.is_null()
                && root_owner != hwnd
                && IsWindowVisible(root_owner) != 0
                && IsIconic(root_owner) == 0
                && ((GetWindowLongW(root_owner, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW) == 0
            {
                hwnd = root_owner;
            } else {
                return None;
            }
        }
        let class = get_window_class(hwnd);
        if class == "Progman"
            || class == "WorkerW"
            || class == "Shell_TrayWnd"
            || class == "Shell_SecondaryTrayWnd"
            || class == "Windows.UI.Core.CoreWindow"
            || class == "XamlExplorerHostIslandWindow"
            || class == "TopLevelWindowForOverflowXamlIsland"
            || class.ends_with("Overlay")
        {
            return None;
        }
        // If running inside a sandboxed Job Object where external desktop windows cannot be queried,
        // skip AntiBurnMainSettingsClass so the backdrop stays hidden instead of freezing around our UI.
        if class == "AntiBurnMainSettingsClass" {
            let tray = FindWindowW(to_wstring("Shell_TrayWnd").as_ptr(), ptr::null());
            if tray.is_null() || IsWindow(tray) == 0 {
                return None;
            }
        }
        if get_visible_window_rect(hwnd).is_some() {
            Some(hwnd)
        } else {
            None
        }
    }
}

/// Shortest distance in pixels from `pt` to a `RECT` (0 if inside the rect).
pub fn distance_to_rect(pt: POINT, rect: RECT) -> f32 {
    let dx = if pt.x < rect.left {
        (rect.left - pt.x) as f32
    } else if pt.x > rect.right {
        (pt.x - rect.right) as f32
    } else {
        0.0
    };
    let dy = if pt.y < rect.top {
        (rect.top - pt.y) as f32
    } else if pt.y > rect.bottom {
        (pt.y - rect.bottom) as f32
    } else {
        0.0
    };
    (dx * dx + dy * dy).sqrt()
}

/// Creates a native 32-bit ARGB `HICON` at the specified pixel size (`16`, `32`, `48`, `256`)
/// from our procedural Anti-Burn OLED Guard icon renderer.
pub fn create_app_hicon(size: usize) -> winapi::shared::windef::HICON {
    use winapi::shared::windef::HICON;
    use winapi::um::wingdi::{
        CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use winapi::um::winuser::{CreateIconIndirect, GetDC, ReleaseDC, ICONINFO};

    unsafe {
        let pixels = crate::icon::render_icon_bgra(size);
        let screen_dc = GetDC(ptr::null_mut());
        let mem_dc = CreateCompatibleDC(screen_dc);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [std::mem::zeroed()],
        };
        let mut bits: *mut winapi::ctypes::c_void = ptr::null_mut();
        let hbm_color = CreateDIBSection(
            mem_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            ptr::null_mut(),
            0,
        );
        DeleteDC(mem_dc);
        ReleaseDC(ptr::null_mut(), screen_dc);

        if hbm_color.is_null() || bits.is_null() {
            return ptr::null_mut() as HICON;
        }

        let dst = std::slice::from_raw_parts_mut(bits as *mut u32, size * size);
        dst.copy_from_slice(&pixels);

        let hbm_mask = CreateBitmap(size as i32, size as i32, 1, 1, ptr::null());
        let mut ii: ICONINFO = std::mem::zeroed();
        ii.fIcon = 1;
        ii.hbmColor = hbm_color;
        ii.hbmMask = hbm_mask;

        let hicon = CreateIconIndirect(&mut ii);
        DeleteObject(hbm_color as *mut _);
        if !hbm_mask.is_null() {
            DeleteObject(hbm_mask as *mut _);
        }
        hicon
    }
}

/// Asks the Windows Shell to refresh its file association & icon cache so updated `.exe` icons show immediately.
pub fn notify_shell_icon_change() {
    unsafe {
        let shell32 = LoadLibraryW(to_wstring("shell32.dll").as_ptr());
        if !shell32.is_null() {
            type FnSHChangeNotify =
                unsafe extern "system" fn(i32, u32, *const winapi::ctypes::c_void, *const winapi::ctypes::c_void);
            let func: Option<FnSHChangeNotify> = std::mem::transmute(GetProcAddress(
                shell32,
                b"SHChangeNotify\0".as_ptr() as *const i8,
            ));
            if let Some(sh_change_notify) = func {
                const SHCNE_ASSOCCHANGED: i32 = 0x08000000;
                const SHCNF_IDLIST: u32 = 0x0000;
                sh_change_notify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, ptr::null(), ptr::null());
            }
        }
    }
}
