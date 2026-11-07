use crate::config::AppConfig;
use crate::win_apis::{
    append_idle_log, detect_pressed_vk, format_local_time, get_idle_duration_ms,
    get_virtual_screen_rect, get_window_class, get_window_title, to_wstring, MagnificationApi,
    ShellAndMediaDetector,
};
use std::ptr;
use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HDC, HWND, POINT, SIZE};
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::wingdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
};
use winapi::um::winuser::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetDC, GetForegroundWindow,
    RegisterClassW, ReleaseDC, SetCursor, SetWindowPos, ShowCursor, ShowWindow,
    UpdateLayeredWindow, HWND_TOPMOST, SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA,
    WM_ERASEBKGND, WM_SETCURSOR, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

unsafe extern "system" fn idle_overlay_wndproc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_SETCURSOR => {
            SetCursor(ptr::null_mut());
            1
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub struct IdleController {
    mag_api: MagnificationApi,
    fallback_overlay_hwnd: HWND,
    mem_dc: HDC,
    bmp: winapi::shared::windef::HBITMAP,
    dib_w: i32,
    dib_h: i32,
    fallback_visible: bool,
    last_overlay_alpha: u8,
    cursor_hidden: bool,
    mouse_click_wake_pending: bool,
    pub current_idle_dim: f32,
    pub raw_idle_secs: f32,
    pub effective_idle_secs: f32,

    last_raw_idle_ms: u64,
    last_cursor: POINT,
    idle_anchor_cursor: POINT,
    synthetic_override_active: bool,
    was_idle_dimmed: bool,
    was_screensaver_active: bool,

    pub state_summary: String,
    pub last_wake_event: String,
    pub prev_wake_event: String,
}

impl IdleController {
    pub fn new() -> Self {
        unsafe {
            let hinstance = GetModuleHandleW(ptr::null());
            let class_name = to_wstring("AntiBurnIdleOverlay");

            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(idle_overlay_wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: ptr::null_mut(),
                hCursor: ptr::null_mut(),
                hbrBackground: ptr::null_mut(),
                lpszMenuName: ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&wc);

            let vrect = get_virtual_screen_rect();
            let vw = vrect.right - vrect.left;
            let vh = vrect.bottom - vrect.top;

            // Omit WS_EX_TRANSPARENT so that when visible (even at SourceConstantAlpha = 1),
            // Windows hit-tests this topmost window across the entire screen and routes
            // WM_SETCURSOR to our thread, hiding the bright hardware mouse cursor during Idle Dimming.
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_LAYERED
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                to_wstring("AntiBurn Global Idle Dimmer").as_ptr(),
                WS_POPUP,
                vrect.left,
                vrect.top,
                vw,
                vh,
                ptr::null_mut(),
                ptr::null_mut(),
                hinstance,
                ptr::null_mut(),
            );

            let screen_dc = GetDC(ptr::null_mut());
            let mem_dc = CreateCompatibleDC(screen_dc);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: vw,
                    biHeight: -vh,
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
            let bmp = CreateDIBSection(
                mem_dc,
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                ptr::null_mut(),
                0,
            );
            SelectObject(mem_dc, bmp as *mut _);
            ReleaseDC(ptr::null_mut(), screen_dc);

            if !bits.is_null() {
                let slice =
                    std::slice::from_raw_parts_mut(bits as *mut u32, (vw * vh) as usize);
                slice.fill(0xFF000000);
            }

            let mut cur = POINT { x: 0, y: 0 };
            GetCursorPos(&mut cur);

            append_idle_log("Anti-Burn OLED Guard started (Idle & Wake Diagnostics active)");

            Self {
                mag_api: MagnificationApi::new(),
                fallback_overlay_hwnd: hwnd,
                mem_dc,
                bmp,
                dib_w: vw,
                dib_h: vh,
                fallback_visible: false,
                last_overlay_alpha: 0,
                cursor_hidden: false,
                mouse_click_wake_pending: false,
                current_idle_dim: 0.0,
                raw_idle_secs: 0.0,
                effective_idle_secs: 0.0,
                last_raw_idle_ms: 0,
                last_cursor: cur,
                idle_anchor_cursor: cur,
                synthetic_override_active: false,
                was_idle_dimmed: false,
                was_screensaver_active: false,
                state_summary: String::from("Monitoring user activity"),
                last_wake_event: String::from("None yet (waiting for first idle cycle)"),
                prev_wake_event: String::from("Log file: idle_wake.log"),
            }
        }
    }

    pub fn fallback_hwnd(&self) -> HWND {
        self.fallback_overlay_hwnd
    }

    /// Invoked synchronously inside `WH_MOUSE_LL` before `win32k.sys` hit-tests a mouse click.
    /// If Idle Dimming is hiding the cursor via `fallback_overlay_hwnd`, immediately hides the
    /// overlay and restores the cursor so the click passes cleanly to the window underneath.
    pub fn on_pre_mouse_click(&mut self) {
        if self.was_idle_dimmed || self.cursor_hidden || self.fallback_visible {
            self.mouse_click_wake_pending = true;
            if self.cursor_hidden {
                unsafe {
                    ShowCursor(1);
                }
                self.cursor_hidden = false;
            }
            if self.fallback_visible {
                unsafe {
                    ShowWindow(self.fallback_overlay_hwnd, SW_HIDE);
                }
                self.fallback_visible = false;
            }
        }
    }

    unsafe fn show_and_update_overlay(&mut self, alpha: u8) {
        let vrect = get_virtual_screen_rect();
        if self.last_overlay_alpha != alpha {
            let mut pt_src = POINT { x: 0, y: 0 };
            let mut pt_dst = POINT {
                x: vrect.left,
                y: vrect.top,
            };
            let mut size = SIZE {
                cx: self.dib_w,
                cy: self.dib_h,
            };
            let mut blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER,
                BlendFlags: 0,
                SourceConstantAlpha: alpha,
                AlphaFormat: AC_SRC_ALPHA,
            };
            let screen_dc = GetDC(ptr::null_mut());
            UpdateLayeredWindow(
                self.fallback_overlay_hwnd,
                screen_dc,
                &mut pt_dst,
                &mut size,
                self.mem_dc,
                &mut pt_src,
                0,
                &mut blend,
                ULW_ALPHA,
            );
            ReleaseDC(ptr::null_mut(), screen_dc);
            self.last_overlay_alpha = alpha;
        }
        if !self.fallback_visible {
            ShowWindow(self.fallback_overlay_hwnd, SW_SHOWNOACTIVATE);
            self.fallback_visible = true;
        }
        SetWindowPos(
            self.fallback_overlay_hwnd,
            HWND_TOPMOST,
            vrect.left,
            vrect.top,
            self.dib_w,
            self.dib_h,
            SWP_NOACTIVATE,
        );
    }

    fn record_wake_event(&mut self, desc: String) {
        append_idle_log(&desc);
        let ts = format_local_time();
        let short_ts = if ts.len() >= 8 { &ts[..8] } else { &ts };
        self.prev_wake_event = std::mem::replace(
            &mut self.last_wake_event,
            format!("[{}] {}", short_ts, desc),
        );
    }

    fn describe_fg_window() -> String {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_null() {
                return String::from("none");
            }
            let title = get_window_title(fg);
            let cls = get_window_class(fg);
            if !title.is_empty() {
                let clean: String = title.chars().take(24).collect();
                format!("'{}'", clean)
            } else if !cls.is_empty() {
                cls
            } else {
                format!("{:?}", fg)
            }
        }
    }

    /// Updates idle tracking and returns `(is_idle_dimmed, should_run_screensaver, screensaver_elapsed_secs)`.
    pub fn update(
        &mut self,
        cfg: &AppConfig,
        detector: &mut ShellAndMediaDetector,
        dt_secs: f32,
        preview_idle_dim: bool,
        preview_screensaver: bool,
        preview_ss_elapsed_secs: f32,
    ) -> (bool, bool, f32) {
        let raw_idle_ms = get_idle_duration_ms();
        self.raw_idle_secs = (raw_idle_ms as f32) / 1000.0;

        let mut cur = POINT { x: 0, y: 0 };
        unsafe {
            GetCursorPos(&mut cur);
        }
        let dx = cur.x - self.last_cursor.x;
        let dy = cur.y - self.last_cursor.y;
        let anchor_dx = cur.x - self.idle_anchor_cursor.x;
        let anchor_dy = cur.y - self.idle_anchor_cursor.y;
        self.last_cursor = cur;

        let pressed_vk = detect_pressed_vk();
        let click_woke = std::mem::replace(&mut self.mouse_click_wake_pending, false);
        let real_mouse_moved = dx.abs() >= 2 || dy.abs() >= 2 || anchor_dx.abs() >= 3 || anchor_dy.abs() >= 3;
        let real_input = real_mouse_moved || pressed_vk.is_some() || click_woke;

        // Check if OS GetLastInputInfo suddenly reset
        let os_idle_reset = raw_idle_ms + 350 < self.last_raw_idle_ms;
        self.last_raw_idle_ms = raw_idle_ms;

        if real_input {
            if self.effective_idle_secs >= 10.0 || self.was_idle_dimmed || self.was_screensaver_active {
                let fg_desc = Self::describe_fg_window();
                let reason = if real_mouse_moved {
                    format!(
                        "Woke at {:.0}s: Mouse ({:+},{:+}px) fg={}",
                        self.effective_idle_secs, dx, dy, fg_desc
                    )
                } else if click_woke {
                    format!(
                        "Woke at {:.0}s: Mouse click fg={}",
                        self.effective_idle_secs, fg_desc
                    )
                } else if let Some(vk) = pressed_vk {
                    format!(
                        "Woke at {:.0}s: Key VK_0x{:02X} fg={}",
                        self.effective_idle_secs, vk, fg_desc
                    )
                } else {
                    format!("Woke at {:.0}s: User input fg={}", self.effective_idle_secs, fg_desc)
                };
                self.record_wake_event(reason);
            }
            self.synthetic_override_active = false;
            self.effective_idle_secs = self.raw_idle_secs;
            self.idle_anchor_cursor = cur;
        } else if os_idle_reset {
            // GetLastInputInfo reset WITHOUT >=2px mouse movement and WITHOUT any physical key detected!
            if self.effective_idle_secs >= 10.0 || self.was_idle_dimmed || self.was_screensaver_active {
                // Ignore synthetic 0px SendInput jiggle from background apps so idle dim & screensaver stay locked in!
                self.synthetic_override_active = true;
                self.effective_idle_secs += dt_secs;
                let fg_desc = Self::describe_fg_window();
                self.record_wake_event(format!(
                    "Blocked fake 0px wake at {:.0}s fg={}",
                    self.effective_idle_secs, fg_desc
                ));
            } else {
                self.effective_idle_secs = self.raw_idle_secs;
                self.idle_anchor_cursor = cur;
            }
        } else if self.synthetic_override_active {
            self.effective_idle_secs += dt_secs;
        } else {
            self.effective_idle_secs = self.raw_idle_secs;
            if self.effective_idle_secs < 2.0 {
                self.idle_anchor_cursor = cur;
            }
        }

        // Poll sustained audio/call activity (always poll so UI meters update live)
        let sustained_media = detector.is_watching_video_or_in_call(dt_secs);

        // CRITICAL: Only allow media/call audio to pause the idle timer BEFORE idle dim or screensaver engages!
        // Once the user has walked away and the screen is already dimmed or running the screensaver,
        // background notification sounds or mic noise can NEVER wake up the screen!
        let already_in_idle_mode = self.was_idle_dimmed || self.was_screensaver_active;
        let media_or_call_active = cfg.pause_on_media_or_call
            && !preview_screensaver
            && !preview_idle_dim
            && !already_in_idle_mode
            && sustained_media;

        if media_or_call_active {
            if self.effective_idle_secs >= 10.0 {
                self.record_wake_event(format!(
                    "Timer paused by active mic call (mic={:.2})",
                    detector.last_capture_peak
                ));
            }
            self.effective_idle_secs = 0.0;
            self.synthetic_override_active = false;
        }

        // Determine if screensaver should be active and how long it has been running
        let (screensaver_active, ss_elapsed_secs) = if preview_screensaver {
            (true, preview_ss_elapsed_secs)
        } else if cfg.screensaver_enabled
            && self.effective_idle_secs >= (cfg.screensaver_stage1_secs as f32)
        {
            (
                true,
                (self.effective_idle_secs - (cfg.screensaver_stage1_secs as f32)).max(0.0),
            )
        } else {
            (false, 0.0)
        };

        // Determine target global idle dim amount
        let should_idle_dim = preview_idle_dim
            || (cfg.idle_dim_enabled && self.effective_idle_secs >= (cfg.idle_timeout_secs as f32));
        let target_dim = if should_idle_dim {
            cfg.idle_dim_amount.clamp(0.0, 0.95)
        } else {
            0.0
        };

        // Log transitions into Idle Dim and Screensaver
        if should_idle_dim && !self.was_idle_dimmed {
            append_idle_log(&format!(
                "Entered Idle Dimming (idle={:.1}s, drop={:.0}%)",
                self.effective_idle_secs,
                cfg.idle_dim_amount * 100.0
            ));
        }
        if screensaver_active && !self.was_screensaver_active {
            append_idle_log(&format!(
                "Entered Screensaver Stage 1 (idle={:.1}s)",
                self.effective_idle_secs
            ));
        }
        self.was_idle_dimmed = should_idle_dim;
        self.was_screensaver_active = screensaver_active;

        // Update live human-readable status summary for the UI diagnostics bar
        self.state_summary = if screensaver_active {
            let s2_span = cfg.screensaver_stage2_secs.saturating_sub(cfg.screensaver_stage1_secs);
            if ss_elapsed_secs >= (s2_span as f32) {
                format!(
                    "SCREENSAVER STAGE 2 (Idle: {:.0}s | Raw OS: {:.0}s)",
                    self.effective_idle_secs, self.raw_idle_secs
                )
            } else {
                format!(
                    "SCREENSAVER STAGE 1 (Idle: {:.0}s / {}s | Raw OS: {:.0}s)",
                    self.effective_idle_secs, cfg.screensaver_stage2_secs, self.raw_idle_secs
                )
            }
        } else if should_idle_dim {
            format!(
                "IDLE DIMMED -{:.0}% (Idle: {:.0}s / SS at {}s | Raw OS: {:.0}s)",
                cfg.idle_dim_amount * 100.0,
                self.effective_idle_secs,
                cfg.screensaver_stage1_secs,
                self.raw_idle_secs
            )
        } else if media_or_call_active {
            format!(
                "PAUSED BY MIC CALL (Mic: {:.3})",
                detector.last_capture_peak
            )
        } else {
            format!(
                "ACTIVE — Idle: {:.0}s / {}s (SS at {}s) | Audio Out: Ignored | Mic: {:.3}",
                self.effective_idle_secs,
                cfg.idle_timeout_secs,
                cfg.screensaver_stage1_secs,
                detector.last_capture_peak
            )
        };

        let speed = if target_dim > self.current_idle_dim {
            1.4
        } else {
            12.0
        };
        let step = (speed * dt_secs).clamp(0.0, 1.0);
        self.current_idle_dim += (target_dim - self.current_idle_dim) * step;

        // Manage hardware mouse cursor visibility:
        // Hide the bright hardware cursor plane whenever Idle Dimming is active,
        // and restore it on the very first frame that the user wakes the screen.
        if should_idle_dim {
            unsafe {
                if !self.cursor_hidden {
                    ShowCursor(0);
                    self.cursor_hidden = true;
                }
                SetCursor(ptr::null_mut());
            }
        } else if self.cursor_hidden {
            unsafe {
                ShowCursor(1);
            }
            self.cursor_hidden = false;
        }

        if self.current_idle_dim < 0.004 {
            self.current_idle_dim = 0.0;
            self.mag_api.reset_global_brightness();
            if self.fallback_visible {
                unsafe {
                    ShowWindow(self.fallback_overlay_hwnd, SW_HIDE);
                }
                self.fallback_visible = false;
            }
        } else {
            let brightness_mult = (1.0 - self.current_idle_dim).clamp(0.05, 1.0);
            if self.mag_api.set_global_brightness(brightness_mult) {
                // Hardware Magnification API is dimming the desktop composition.
                // While `should_idle_dim` is true (and the screensaver isn't already covering the screen),
                // keep `fallback_overlay_hwnd` at `SourceConstantAlpha = 1` (0.39% opacity, visually
                // imperceptible) so Windows hit-tests our topmost window under the cursor everywhere
                // and hides the hardware cursor plane across all external applications.
                if should_idle_dim && !screensaver_active {
                    unsafe {
                        self.show_and_update_overlay(1);
                    }
                } else if self.fallback_visible {
                    unsafe {
                        ShowWindow(self.fallback_overlay_hwnd, SW_HIDE);
                    }
                    self.fallback_visible = false;
                }
            } else {
                unsafe {
                    let alpha = (self.current_idle_dim * 255.0).round().clamp(1.0, 255.0) as u8;
                    self.show_and_update_overlay(alpha);
                }
            }
        }

        (
            self.current_idle_dim > 0.01,
            screensaver_active,
            ss_elapsed_secs,
        )
    }
}

impl Drop for IdleController {
    fn drop(&mut self) {
        self.mag_api.reset_global_brightness();
        unsafe {
            if self.cursor_hidden {
                ShowCursor(1);
                self.cursor_hidden = false;
            }
            if !self.fallback_overlay_hwnd.is_null() {
                DestroyWindow(self.fallback_overlay_hwnd);
            }
            if !self.bmp.is_null() {
                DeleteObject(self.bmp as *mut _);
            }
            if !self.mem_dc.is_null() {
                DeleteDC(self.mem_dc);
            }
        }
    }
}
