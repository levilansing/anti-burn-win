#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod icon;
mod idle_controller;
mod neo_ui;
mod screensaver;
mod taskbar_dimmer;
mod win_apis;
mod window_dimmer;
#[cfg(test)]
mod tests;

use config::AppConfig;
use idle_controller::IdleController;
use neo_ui::{ControlId, NeoUi};
use screensaver::ScreensaverEngine;
use std::cell::RefCell;
use std::ptr;
use taskbar_dimmer::TaskbarDimmer;
use win_apis::{
    append_idle_log, create_app_hicon, detect_pressed_vk, enable_dpi_awareness,
    enable_high_res_timer, get_idle_duration_ms, get_primary_monitor_rect,
    notify_shell_icon_change, to_wstring, ShellAndMediaDetector,
};
use winapi::shared::minwindef::{DWORD, LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HICON, HWINEVENTHOOK, HWND, POINT, RECT};
use winapi::um::commctrl::InitCommonControls;
use winapi::um::dwmapi::DwmSetWindowAttribute;
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::shellapi::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use winapi::um::sysinfoapi::GetTickCount64;
use winapi::um::winuser::{
    AdjustWindowRectEx, AppendMenuW, BeginPaint, BringWindowToTop, CallNextHookEx, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, EndPaint,
    FindWindowW, GetClientRect, GetCursorPos, GetMessageW, InvalidateRect, IsIconic,
    IsWindowVisible, LoadCursorW, LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW,
    ReleaseCapture, SendMessageW, SetCapture, SetForegroundWindow, SetTimer, SetWinEventHook,
    SetWindowPos, SetWindowsHookExW, ShowWindow, TrackPopupMenu, TranslateMessage,
    UnhookWinEvent, UnhookWindowsHookEx, EVENT_SYSTEM_FOREGROUND, HWND_NOTOPMOST, HWND_TOPMOST,
    ICON_BIG, ICON_SMALL, IDC_ARROW, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG, MSLLHOOKSTRUCT,
    PAINTSTRUCT, SW_HIDE, SW_MINIMIZE, SW_RESTORE, SW_SHOW, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
    WH_MOUSE_LL, WINEVENT_OUTOFCONTEXT, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY,
    WM_ERASEBKGND, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE,
    WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETICON, WM_TIMER, WNDCLASSW, WS_CAPTION,
    WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
};
use window_dimmer::WindowDimmer;

const WM_TRAYICON: UINT = winapi::um::winuser::WM_USER + 101;
const TIMER_MAIN_LOOP: usize = 1;

const ID_TRAY_OPEN: usize = 901;
const ID_TRAY_TEST_IDLE: usize = 902;
const ID_TRAY_PREVIEW_SS: usize = 903;
const ID_TRAY_EXIT: usize = 904;

struct AppState {
    main_hwnd: HWND,
    cfg: AppConfig,
    ui: NeoUi,
    detector: ShellAndMediaDetector,
    taskbar_dimmer: TaskbarDimmer,
    window_dimmer: WindowDimmer,
    idle_controller: IdleController,
    screensaver: ScreensaverEngine,
    last_tick_ms: u64,
    ui_diag_refresh_counter: u32,
    tray_hicon: HICON,
    tray_available: bool,

    preview_idle_dim: bool,
    preview_idle_start_ms: u64,

    preview_screensaver: bool,
    preview_ss_full_black: bool,
    preview_ss_turn_off_monitor: bool,
    preview_ss_start_ms: u64,

    hot_corner_hover_secs: f32,
    hot_corner_latched: bool,
}

thread_local! {
    static STATE: RefCell<Option<AppState>> = RefCell::new(None);
}

impl AppState {
    fn tick(&mut self) {
        let now = unsafe { GetTickCount64() };
        let dt_ms = now.saturating_sub(self.last_tick_ms).clamp(1, 100);
        self.last_tick_ms = now;
        let dt_secs = (dt_ms as f32) / 1000.0;

        let idle_ms = get_idle_duration_ms();

        // Check Bottom-Left Hot Corner: holding the cursor in the bottom-left 12x12 pixel corner
        // of the primary monitor for 1.0 full second either starts the screensaver or turns off the monitor!
        let mut cur = POINT { x: 0, y: 0 };
        unsafe {
            GetCursorPos(&mut cur);
        }
        let mrect = get_primary_monitor_rect();
        let in_bottom_left_corner = cur.x >= mrect.left
            && cur.x <= mrect.left + 12
            && cur.y >= mrect.bottom - 12
            && cur.y <= mrect.bottom;
        let outside_hot_corner_hysteresis =
            cur.x > mrect.left + 20 || cur.y < mrect.bottom - 20;

        if in_bottom_left_corner {
            if !self.hot_corner_latched {
                self.hot_corner_hover_secs += dt_secs;
                if self.hot_corner_hover_secs >= 1.0 {
                    self.hot_corner_latched = true;
                    self.preview_screensaver = true;
                    self.preview_ss_full_black = true;
                    self.preview_ss_turn_off_monitor = self.cfg.hot_corner_turn_off_monitor;
                    self.preview_ss_start_ms = now;
                    if self.cfg.hot_corner_turn_off_monitor {
                        append_idle_log("Turn Off Monitor triggered via Bottom-Left Hot Corner (1.0s hold)");
                    } else {
                        append_idle_log("Screensaver started via Bottom-Left Hot Corner (1.0s hold, 100% black backdrop)");
                    }
                }
            }
        } else if outside_hot_corner_hysteresis {
            self.hot_corner_hover_secs = 0.0;
            self.hot_corner_latched = false;
        } else if !self.hot_corner_latched {
            self.hot_corner_hover_secs = 0.0;
        }

        // Cancel manual Idle Dim test when user moves mouse or presses key after 1.5s grace period
        if self.preview_idle_dim
            && now.saturating_sub(self.preview_idle_start_ms) > 1500
            && idle_ms < 120
        {
            self.preview_idle_dim = false;
        }

        // Cancel manual/hot-corner Screensaver when user moves mouse OUT of the hot corner or presses a key
        if self.preview_screensaver && now.saturating_sub(self.preview_ss_start_ms) > 1500 {
            if in_bottom_left_corner {
                // While parked inside the bottom-left hot corner, ignore tiny mouse sensor jitter
                // and only wake if a physical keyboard key is pressed!
                if detect_pressed_vk().is_some() {
                    self.preview_screensaver = false;
                }
            } else if idle_ms < 120 {
                self.preview_screensaver = false;
            }
        }

        let preview_ss_elapsed_secs = if self.preview_screensaver {
            (now.saturating_sub(self.preview_ss_start_ms) as f32) / 1000.0
        } else {
            self.preview_ss_full_black = false;
            self.preview_ss_turn_off_monitor = false;
            0.0
        };

        // 1. Update Global Idle Dimmer & Screensaver Stage controller
        let (_is_idle_dimmed, ss_active, ss_elapsed_secs) = self.idle_controller.update(
            &self.cfg,
            &mut self.detector,
            dt_secs,
            self.preview_idle_dim,
            self.preview_screensaver,
            preview_ss_elapsed_secs,
        );

        let ss_visible = ss_active || self.screensaver.is_visible();

        if !self.screensaver.is_in_display_sleep() {
            // 2. Update Active/Inactive Window Dimmer (125 FPS hardware crossfade + true Z-order backdrop behind active window)
            let extra_hwnds = [
                self.taskbar_dimmer.overlay_hwnd,
                self.idle_controller.fallback_hwnd(),
                self.screensaver.hwnd,
            ];
            self.window_dimmer
                .update(&self.cfg, dt_secs, &extra_hwnds, ss_visible);

            // 3. Update Taskbar Dimmer (125 FPS hardware proximity fade)
            self.taskbar_dimmer
                .update(&self.cfg, &self.detector, dt_secs, ss_visible);
        }

        // 4. Update & Render Coupled Flow Field + Metaballs Screensaver OVER TOP of all active dimmers
        self.screensaver.update_and_render(
            &self.cfg,
            ss_active,
            ss_elapsed_secs,
            self.preview_ss_full_black,
            self.preview_ss_turn_off_monitor,
            dt_secs,
            self.main_hwnd,
        );

        // 5. Refresh Live Idle & Wakeup Diagnostics card in UI (~4 Hz when settings window is open)
        self.ui_diag_refresh_counter = self.ui_diag_refresh_counter.wrapping_add(1);
        if !self.tray_available && self.ui_diag_refresh_counter % 125 == 0 {
            self.tray_available = unsafe { add_tray_icon(self.main_hwnd, self.tray_hicon) };
        }
        if self.ui_diag_refresh_counter % 30 == 0 {
            unsafe {
                if IsWindowVisible(self.main_hwnd) != 0 && IsIconic(self.main_hwnd) == 0 {
                    let diag_rect = RECT {
                        left: 24,
                        top: 922,
                        right: 576,
                        bottom: 1012,
                    };
                    InvalidateRect(self.main_hwnd, &diag_rect, 0);
                }
            }
        }
    }

    fn on_pre_mouse_click(&mut self, pt: POINT) {
        let now = unsafe { GetTickCount64() };
        if self.preview_idle_dim && now.saturating_sub(self.preview_idle_start_ms) > 500 {
            self.preview_idle_dim = false;
        }
        self.idle_controller.on_pre_mouse_click();

        let ss_visible = self.screensaver.is_visible();
        let extra_hwnds = [
            self.taskbar_dimmer.overlay_hwnd,
            self.idle_controller.fallback_hwnd(),
            self.screensaver.hwnd,
        ];
        self.window_dimmer
            .on_pre_mouse_click(pt, &self.cfg, &extra_hwnds, ss_visible);
    }
}

/// Low-level mouse hook (`WH_MOUSE_LL`) invoked synchronously by `win32k.sys` BEFORE a mouse click
/// is delivered to an inactive window or raises it in the Z-order.
unsafe extern "system" fn low_level_mouse_proc(
    n_code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if n_code >= 0 {
        let msg = wparam as UINT;
        if msg == WM_LBUTTONDOWN || msg == WM_RBUTTONDOWN || msg == WM_MBUTTONDOWN {
            let info = &*(lparam as *const MSLLHOOKSTRUCT);
            STATE.with(|cell| {
                if let Ok(mut borrow) = cell.try_borrow_mut() {
                    if let Some(ref mut st) = *borrow {
                        st.on_pre_mouse_click(info.pt);
                    }
                }
            });
        }
    }
    CallNextHookEx(ptr::null_mut(), n_code, wparam, lparam)
}

/// WinEvent hook (`EVENT_SYSTEM_FOREGROUND`) that runs `tick()` immediately when any window changes focus.
unsafe extern "system" fn win_event_foreground_proc(
    _hook: HWINEVENTHOOK,
    _event: DWORD,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: DWORD,
    _dwms_event_time: DWORD,
) {
    STATE.with(|cell| {
        if let Ok(mut borrow) = cell.try_borrow_mut() {
            if let Some(ref mut st) = *borrow {
                st.tick();
            }
        }
    });
}

unsafe fn add_tray_icon(hwnd: HWND, hicon: HICON) -> bool {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAYICON;
    nid.hIcon = if !hicon.is_null() {
        hicon
    } else {
        LoadIconW(ptr::null_mut(), IDI_APPLICATION)
    };
    let tip = to_wstring("Anti-Burn OLED Guard (Right-click for options)");
    for (i, &ch) in tip.iter().take(nid.szTip.len() - 1).enumerate() {
        nid.szTip[i] = ch;
    }
    Shell_NotifyIconW(NIM_ADD, &mut nid) != 0
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    Shell_NotifyIconW(NIM_DELETE, &mut nid);
}

unsafe extern "system" fn settings_wndproc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let cfg = AppConfig::load();

            // Enable Windows 11 dark titlebar attribute (DWMWA_USE_IMMERSIVE_DARK_MODE = 20)
            let dark_mode: u32 = 1;
            let _ = DwmSetWindowAttribute(
                hwnd,
                20,
                &dark_mode as *const u32 as *const _,
                std::mem::size_of::<u32>() as u32,
            );

            // Apply custom 32-bit ARGB Anti-Burn OLED Guard icons to window titlebar, taskbar, Alt+Tab, and system tray
            let icon_big = create_app_hicon(256);
            let icon_small = create_app_hicon(32);
            if !icon_big.is_null() {
                SendMessageW(hwnd, WM_SETICON, ICON_BIG as WPARAM, icon_big as LPARAM);
            }
            if !icon_small.is_null() {
                SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, icon_small as LPARAM);
            }

            let tray_available = add_tray_icon(hwnd, icon_small);

            let state = AppState {
                main_hwnd: hwnd,
                cfg,
                ui: NeoUi::new(),
                detector: ShellAndMediaDetector::new(),
                taskbar_dimmer: TaskbarDimmer::new(),
                window_dimmer: WindowDimmer::new(),
                idle_controller: IdleController::new(),
                screensaver: ScreensaverEngine::new(),
                last_tick_ms: GetTickCount64(),
                ui_diag_refresh_counter: 0,
                tray_hicon: icon_small,
                tray_available,
                preview_idle_dim: false,
                preview_idle_start_ms: 0,
                preview_screensaver: false,
                preview_ss_full_black: false,
                preview_ss_turn_off_monitor: false,
                preview_ss_start_ms: 0,
                hot_corner_hover_secs: 0.0,
                hot_corner_latched: false,
            };

            STATE.with(|cell| {
                *cell.borrow_mut() = Some(state);
            });

            // 8ms interval with 1ms high-res timer = 125 FPS smooth animations!
            SetTimer(hwnd, TIMER_MAIN_LOOP, 8, None);
            0
        }

        // Prevent GDI background erase so our offscreen double-buffered WM_PAINT has zero flicker!
        WM_ERASEBKGND => 1,

        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut client_rect: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut client_rect);

            STATE.with(|cell| {
                if let Some(ref st) = *cell.borrow() {
                    st.ui.paint(
                        hwnd,
                        hdc,
                        client_rect,
                        &st.cfg,
                        &st.idle_controller.state_summary,
                        &st.idle_controller.last_wake_event,
                        &st.idle_controller.prev_wake_event,
                    );
                }
            });

            EndPaint(hwnd, &ps);
            0
        }

        WM_LBUTTONDOWN => {
            let pt = POINT {
                x: (lparam & 0xFFFF) as i16 as i32,
                y: ((lparam >> 16) & 0xFFFF) as i16 as i32,
            };
            SetCapture(hwnd);

            let mut clicked = None;
            let mut tray_avail = false;
            STATE.with(|cell| {
                if let Some(ref mut st) = *cell.borrow_mut() {
                    clicked = st.ui.on_mouse_down(pt, &mut st.cfg);
                    tray_avail = st.tray_available;
                }
            });

            if let Some(id) = clicked {
                match id {
                    ControlId::BtnPreviewIdleDim => {
                        ReleaseCapture();
                        STATE.with(|cell| {
                            if let Some(ref mut st) = *cell.borrow_mut() {
                                st.ui.on_mouse_up();
                                st.preview_idle_dim = true;
                                st.preview_idle_start_ms = GetTickCount64();
                            }
                        });
                    }
                    ControlId::BtnPreviewScreensaver => {
                        ReleaseCapture();
                        STATE.with(|cell| {
                            if let Some(ref mut st) = *cell.borrow_mut() {
                                st.ui.on_mouse_up();
                                st.preview_screensaver = true;
                                st.preview_ss_full_black = false;
                                st.preview_ss_turn_off_monitor = false;
                                st.preview_ss_start_ms = GetTickCount64();
                            }
                        });
                    }
                    ControlId::BtnMinimizeToTray => {
                        ReleaseCapture();
                        if tray_avail {
                            ShowWindow(hwnd, SW_HIDE);
                        } else {
                            ShowWindow(hwnd, SW_MINIMIZE);
                        }
                    }
                    _ => {}
                }
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }

        WM_MOUSEMOVE => {
            let pt = POINT {
                x: (lparam & 0xFFFF) as i16 as i32,
                y: ((lparam >> 16) & 0xFFFF) as i16 as i32,
            };
            let mut needs_repaint = false;
            STATE.with(|cell| {
                if let Some(ref mut st) = *cell.borrow_mut() {
                    needs_repaint = st.ui.on_mouse_move(pt, &mut st.cfg);
                }
            });
            if needs_repaint {
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }

        WM_LBUTTONUP => {
            ReleaseCapture();
            let mut needs_repaint = false;
            STATE.with(|cell| {
                if let Some(ref mut st) = *cell.borrow_mut() {
                    needs_repaint = st.ui.on_mouse_up();
                }
            });
            if needs_repaint {
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }

        WM_TIMER => {
            if wparam == TIMER_MAIN_LOOP {
                STATE.with(|cell| {
                    if let Some(ref mut st) = *cell.borrow_mut() {
                        st.tick();
                    }
                });
            }
            0
        }

        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as usize;
            match id {
                ID_TRAY_OPEN => {
                    ShowWindow(hwnd, SW_SHOW);
                    ShowWindow(hwnd, SW_RESTORE);
                    SetForegroundWindow(hwnd);
                }
                ID_TRAY_TEST_IDLE => {
                    STATE.with(|cell| {
                        if let Some(ref mut st) = *cell.borrow_mut() {
                            st.preview_idle_dim = true;
                            st.preview_idle_start_ms = GetTickCount64();
                        }
                    });
                }
                ID_TRAY_PREVIEW_SS => {
                    STATE.with(|cell| {
                        if let Some(ref mut st) = *cell.borrow_mut() {
                            st.preview_screensaver = true;
                            st.preview_ss_full_black = false;
                            st.preview_ss_turn_off_monitor = false;
                            st.preview_ss_start_ms = GetTickCount64();
                        }
                    });
                }
                ID_TRAY_EXIT => {
                    remove_tray_icon(hwnd);
                    DestroyWindow(hwnd);
                    PostQuitMessage(0);
                }
                _ => {}
            }
            0
        }

        WM_TRAYICON => {
            let event = (lparam & 0xFFFF) as UINT;
            if event == WM_LBUTTONDBLCLK {
                ShowWindow(hwnd, SW_SHOW);
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            } else if event == WM_RBUTTONUP {
                let menu = CreatePopupMenu();
                let s_open = to_wstring("Open Anti-Burn Settings");
                let s_idle = to_wstring("Test Idle Dim Now");
                let s_prev = to_wstring("Test Screensaver Now");
                let s_exit = to_wstring("Exit Anti-Burn");
                AppendMenuW(menu, MF_STRING, ID_TRAY_OPEN, s_open.as_ptr());
                AppendMenuW(menu, MF_STRING, ID_TRAY_TEST_IDLE, s_idle.as_ptr());
                AppendMenuW(menu, MF_STRING, ID_TRAY_PREVIEW_SS, s_prev.as_ptr());
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
                AppendMenuW(menu, MF_STRING, ID_TRAY_EXIT, s_exit.as_ptr());

                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                SetForegroundWindow(hwnd);
                TrackPopupMenu(
                    menu,
                    TPM_LEFTALIGN | TPM_BOTTOMALIGN,
                    pt.x,
                    pt.y,
                    0,
                    hwnd,
                    ptr::null(),
                );
                DestroyMenu(menu);
            }
            0
        }

        WM_CLOSE => {
            let mut tray_avail = false;
            STATE.with(|cell| {
                if let Some(ref st) = *cell.borrow() {
                    tray_avail = st.tray_available;
                }
            });
            if tray_avail {
                ShowWindow(hwnd, SW_HIDE);
            } else {
                DestroyWindow(hwnd);
            }
            0
        }

        WM_DESTROY => {
            remove_tray_icon(hwnd);
            STATE.with(|cell| {
                *cell.borrow_mut() = None;
            });
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    unsafe {
        // Must be called first so all coordinates and windows use physical monitor pixels and 1ms timer resolution!
        enable_dpi_awareness();
        enable_high_res_timer();
        InitCommonControls();

        let class_name = to_wstring("AntiBurnMainSettingsClass");

        // Single-instance guard: if an earlier instance of anti-burn.exe is running, tell it to exit cleanly
        // so two instances NEVER double-dim the taskbar or fight over MagSetFullscreenColorEffect!
        let existing = FindWindowW(class_name.as_ptr(), ptr::null());
        if !existing.is_null() {
            PostMessageW(existing, WM_COMMAND, ID_TRAY_EXIT, 0);
            PostMessageW(existing, WM_DESTROY, 0, 0);
            std::thread::sleep(std::time::Duration::from_millis(80));
        }

        let hinstance = GetModuleHandleW(ptr::null());
        let app_hicon = create_app_hicon(256);
        notify_shell_icon_change();

        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(settings_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: if !app_hicon.is_null() {
                app_hicon
            } else {
                LoadIconW(ptr::null_mut(), IDI_APPLICATION)
            },
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        RegisterClassW(&wc);

        // Install low-level mouse hook (WH_MOUSE_LL) and foreground WinEvent hook so clicking an inactive window
        // covers it with `new_mask` BEFORE win32k.sys raises it in the Z-order (zero white flash on window switch!)
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), ptr::null_mut(), 0);
        let fg_hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            ptr::null_mut(),
            Some(win_event_foreground_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );

        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut win_rect = RECT {
            left: 0,
            top: 0,
            right: 600,
            bottom: 1080,
        };
        AdjustWindowRectEx(&mut win_rect, style, 0, 0);

        let title = to_wstring("Anti-Burn OLED Guard");
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            style,
            120,
            80,
            win_rect.right - win_rect.left,
            win_rect.bottom - win_rect.top,
            ptr::null_mut(),
            ptr::null_mut(),
            hinstance,
            ptr::null_mut(),
        );

        if hwnd.is_null() {
            return;
        }

        let start_minimized = std::env::args().any(|a| a == "--minimized" || a == "--startup");
        if start_minimized {
            // When launched automatically at Windows login (--minimized), always start hidden in the system tray!
            ShowWindow(hwnd, SW_HIDE);
        } else {
            // Force window to the front of the Z-order above the IDE even if Windows Foreground Lock is active
            ShowWindow(hwnd, SW_SHOW);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                120,
                80,
                win_rect.right - win_rect.left,
                win_rect.bottom - win_rect.top,
                0,
            );
            SetWindowPos(
                hwnd,
                HWND_NOTOPMOST,
                120,
                80,
                win_rect.right - win_rect.left,
                win_rect.bottom - win_rect.top,
                0,
            );
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if !mouse_hook.is_null() {
            UnhookWindowsHookEx(mouse_hook);
        }
        if !fg_hook.is_null() {
            UnhookWinEvent(fg_hook);
        }
    }
}
