use crate::config::AppConfig;
use crate::win_apis::{
    get_all_monitor_work_rects, get_visible_window_rect, resolve_eligible_active_window, to_wstring,
};
use std::ptr;
use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HDC, HBITMAP, HWND, POINT, RECT, SIZE};
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::wingdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
};
use winapi::um::winuser::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetDC, GetForegroundWindow,
    GetWindow, GetWindowLongW, RegisterClassW, ReleaseDC, SetWindowPos, ShowWindow,
    UpdateLayeredWindow, WindowFromPoint, GWL_EXSTYLE, GW_HWNDPREV, HWND_NOTOPMOST, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSENDCHANGING, SWP_NOSIZE, SW_HIDE,
    SW_SHOWNOACTIVATE, ULW_ALPHA, WM_ERASEBKGND, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

unsafe extern "system" fn dim_overlay_wndproc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_ERASEBKGND {
        return 1;
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Places `hwnd` immediately behind `target_hwnd` in the Win32 Z-order (`GetWindow(hwnd, GW_HWNDPREV) == target_hwnd`).
/// Makes zero syscalls if `hwnd` is already immediately behind `target_hwnd`.
unsafe fn place_window_behind(hwnd: HWND, target_hwnd: HWND) {
    if hwnd.is_null() || target_hwnd.is_null() {
        return;
    }
    if GetWindow(hwnd, GW_HWNDPREV) == target_hwnd {
        return;
    }

    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_NOOWNERZORDER;
    let target_topmost = (GetWindowLongW(target_hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST) != 0;

    if target_topmost {
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags);
        SetWindowPos(hwnd, target_hwnd, 0, 0, 0, 0, flags);
    } else {
        SetWindowPos(hwnd, target_hwnd, 0, 0, 0, 0, flags);
        if GetWindow(hwnd, GW_HWNDPREV) != target_hwnd {
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags);
            SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, flags);
            SetWindowPos(hwnd, target_hwnd, 0, 0, 0, 0, flags);
        }
    }
}

/// Full-monitor-work-area (`rcWork`, excluding the taskbar) pure black `UpdateLayeredWindow` surface.
/// Sits physically BEHIND the active window in the DWM Z-order so dragging the active window at 120Hz+
/// has ZERO lag and can NEVER flash white.
struct MonitorBackdrop {
    hwnd: HWND,
    work_rect: RECT,
    mem_dc: HDC,
    bmp: HBITMAP,
    visible: bool,
    last_alpha: u8,
}

impl MonitorBackdrop {
    unsafe fn new(work_rect: RECT) -> Self {
        let hinstance = GetModuleHandleW(ptr::null());
        let class_name = to_wstring("AntiBurnWindowDimOverlay");
        let w = (work_rect.right - work_rect.left).max(1);
        let h = (work_rect.bottom - work_rect.top).max(1);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            to_wstring("AntiBurn Monitor Backdrop").as_ptr(),
            WS_POPUP,
            work_rect.left,
            work_rect.top,
            w,
            h,
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
                biWidth: w,
                biHeight: -h,
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

        if !bits.is_null() {
            let slice = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
            slice.fill(0xFF000000);
        }

        // Upload the pure-black DIB texture ONCE at creation time with alpha = 0.
        let mut pt_src = POINT { x: 0, y: 0 };
        let mut pt_dst = POINT {
            x: work_rect.left,
            y: work_rect.top,
        };
        let mut size = SIZE { cx: w, cy: h };
        let mut blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER,
            BlendFlags: 0,
            SourceConstantAlpha: 0,
            AlphaFormat: AC_SRC_ALPHA,
        };
        UpdateLayeredWindow(
            hwnd,
            screen_dc,
            &mut pt_dst,
            &mut size,
            mem_dc,
            &mut pt_src,
            0,
            &mut blend,
            ULW_ALPHA,
        );
        ReleaseDC(ptr::null_mut(), screen_dc);

        Self {
            hwnd,
            work_rect,
            mem_dc,
            bmp,
            visible: false,
            last_alpha: 0,
        }
    }

    unsafe fn show_and_set_alpha(&mut self, alpha: u8, behind_hwnd: HWND) {
        if !self.visible {
            place_window_behind(self.hwnd, behind_hwnd);
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            self.visible = true;
            self.last_alpha = 255;
        } else {
            place_window_behind(self.hwnd, behind_hwnd);
        }

        if alpha != self.last_alpha {
            self.last_alpha = alpha;
            let mut blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER,
                BlendFlags: 0,
                SourceConstantAlpha: alpha,
                AlphaFormat: AC_SRC_ALPHA,
            };
            // hdcSrc = NULL updates ONLY hardware SourceConstantAlpha in DWM with zero bitmap transfer!
            UpdateLayeredWindow(
                self.hwnd,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut blend,
                ULW_ALPHA,
            );
        }
    }

    /// Places this backdrop at the very top of the normal (`HWND_NOTOPMOST`) window band so that when
    /// NO application window is currently focused, ALL open windows and the desktop behind it are dimmed!
    /// Makes zero `SetWindowPos` calls once `self.hwnd` is already at the top of the normal band.
    unsafe fn show_at_top_of_normal_band(&mut self, alpha: u8) {
        let flags =
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_NOOWNERZORDER;
        let prev = GetWindow(self.hwnd, GW_HWNDPREV);
        let already_at_top = prev.is_null()
            || ((GetWindowLongW(prev, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST) != 0);

        if !self.visible {
            SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags);
            SetWindowPos(self.hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, flags);
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            self.visible = true;
            self.last_alpha = 255;
        } else if !already_at_top {
            SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags);
            SetWindowPos(self.hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, flags);
        }

        if alpha != self.last_alpha {
            self.last_alpha = alpha;
            let mut blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER,
                BlendFlags: 0,
                SourceConstantAlpha: alpha,
                AlphaFormat: AC_SRC_ALPHA,
            };
            UpdateLayeredWindow(
                self.hwnd,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut blend,
                ULW_ALPHA,
            );
        }
    }

    /// Sets alpha to 0 while keeping the DWM composition surface alive and ready (`visible == true`).
    unsafe fn set_alpha_zero_warm(&mut self) {
        if self.visible && self.last_alpha != 0 {
            self.last_alpha = 0;
            let mut blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER,
                BlendFlags: 0,
                SourceConstantAlpha: 0,
                AlphaFormat: AC_SRC_ALPHA,
            };
            UpdateLayeredWindow(
                self.hwnd,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut blend,
                ULW_ALPHA,
            );
        }
    }

    unsafe fn hide(&mut self) {
        if self.visible {
            ShowWindow(self.hwnd, SW_HIDE);
            self.visible = false;
            self.last_alpha = 0;
        }
    }
}

impl Drop for MonitorBackdrop {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
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

/// Hardware-accelerated `UpdateLayeredWindow` rounded-corner transition mask for fading the newly
/// activated window (`Window B`) from `base_alpha -> 0` at 125 FPS.
///
/// Key Anti-Flicker Architecture:
/// 1. Pre-allocated maximum monitor-sized DIB (`max_w x max_h`) created ONCE at startup so switching
///    between windows NEVER calls `CreateDIBSection` or takes kernel page faults!
/// 2. Permanently kept warm (`SW_SHOWNOACTIVATE` with `SourceConstantAlpha = 0` when idle) so DWM
///    never has a 1-2 frame visual creation delay when a window switch starts!
/// 3. Pre-staged over the inactive window under the mouse cursor (`alpha = 0`) BEFORE the user clicks,
///    so clicking an inactive window flips `SourceConstantAlpha` from `0 -> base_alpha` via `hdcSrc = NULL`
///    in ~5 microseconds with zero pixel upload!
struct TransitionMask {
    hwnd: HWND,
    mem_dc: HDC,
    bmp: HBITMAP,
    bits: *mut u32,
    max_w: i32,
    max_h: i32,
    cur_w: i32,
    cur_h: i32,
    last_pos: POINT,
    last_alpha: u8,
    visible: bool,
}

impl TransitionMask {
    unsafe fn new(title: &str, max_w: i32, max_h: i32) -> Self {
        let hinstance = GetModuleHandleW(ptr::null());
        let class_name = to_wstring("AntiBurnWindowDimOverlay");
        let ex_style = WS_EX_LAYERED
            | WS_EX_TRANSPARENT
            | WS_EX_TOOLWINDOW
            | WS_EX_NOACTIVATE
            | WS_EX_TOPMOST;

        let mw = max_w.max(64);
        let mh = max_h.max(64);

        let hwnd = CreateWindowExW(
            ex_style,
            class_name.as_ptr(),
            to_wstring(title).as_ptr(),
            WS_POPUP,
            0,
            0,
            16,
            16,
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
                biWidth: mw,
                biHeight: -mh, // top-down
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
        let mut bits_ptr: *mut winapi::ctypes::c_void = ptr::null_mut();
        let bmp = CreateDIBSection(
            mem_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            ptr::null_mut(),
            0,
        );
        SelectObject(mem_dc, bmp as *mut _);
        ReleaseDC(ptr::null_mut(), screen_dc);

        let bits = bits_ptr as *mut u32;
        if !bits.is_null() {
            let slice = std::slice::from_raw_parts_mut(bits, (mw * mh) as usize);
            slice.fill(0xFF000000);
        }

        Self {
            hwnd,
            mem_dc,
            bmp,
            bits,
            max_w: mw,
            max_h: mh,
            cur_w: 0,
            cur_h: 0,
            last_pos: POINT { x: -99999, y: -99999 },
            last_alpha: 255,
            visible: false,
        }
    }

    /// Restores the 4 corners of the previous `(cur_w, cur_h)` sub-rect in the pre-allocated DIB back to
    /// `0xFF000000` (only 256 pixels total!) and carves the 8px anti-aliased SDF rounded corners for `(w, h)`.
    unsafe fn stamp_rounded_corners(&mut self, w: i32, h: i32) {
        if self.bits.is_null() {
            return;
        }
        let stride = self.max_w;
        let slice = std::slice::from_raw_parts_mut(self.bits, (self.max_w * self.max_h) as usize);

        // 1. Restore previous corners back to solid black 0xFF000000 (at most 4 * 8 * 8 = 256 pixels!)
        if self.cur_w > 0 && self.cur_h > 0 {
            let pw = self.cur_w.min(self.max_w);
            let ph = self.cur_h.min(self.max_h);
            let pr = 8i32.min(pw / 2).min(ph / 2);
            for cy in 0..pr {
                for cx in 0..pr {
                    slice[(cy * stride + cx) as usize] = 0xFF000000;
                    slice[(cy * stride + (pw - 1 - cx)) as usize] = 0xFF000000;
                    slice[((ph - 1 - cy) * stride + cx) as usize] = 0xFF000000;
                    slice[((ph - 1 - cy) * stride + (pw - 1 - cx)) as usize] = 0xFF000000;
                }
            }
        }

        // 2. Carve new 8px anti-aliased Windows 11 rounded corners for (w, h)
        let cw = w.min(self.max_w);
        let ch = h.min(self.max_h);
        let r = 8i32.min(cw / 2).min(ch / 2);
        let r_f = r as f32;
        for cy in 0..r {
            for cx in 0..r {
                let dx = (r_f - 0.5) - (cx as f32);
                let dy = (r_f - 0.5) - (cy as f32);
                let dist = (dx * dx + dy * dy).sqrt();
                let cov = (r_f + 0.5 - dist).clamp(0.0, 1.0);
                let a = (cov * 255.0).round() as u32;
                let px = a << 24;

                slice[(cy * stride + cx) as usize] = px;
                slice[(cy * stride + (cw - 1 - cx)) as usize] = px;
                slice[((ch - 1 - cy) * stride + cx) as usize] = px;
                slice[((ch - 1 - cy) * stride + (cw - 1 - cx)) as usize] = px;
            }
        }
    }

    /// Prepares the mask for `rect` at `initial_alpha`.
    /// If `(w, h)` already matches `(self.cur_w, self.cur_h)`, skips `hdcSrc` upload completely!
    unsafe fn prepare_shape(&mut self, rect: RECT, initial_alpha: u8) {
        let w = (rect.right - rect.left).clamp(1, self.max_w);
        let h = (rect.bottom - rect.top).clamp(1, self.max_h);

        if !self.visible {
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            self.visible = true;
        }

        // Ensure topmost without resizing via SetWindowPos (which would invalidate UpdateLayeredWindow!)
        SetWindowPos(
            self.hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_NOOWNERZORDER,
        );

        if w == self.cur_w && h == self.cur_h {
            // Fast path: dimensions already match! Update ONLY position & hardware SourceConstantAlpha!
            self.update_frame(rect, initial_alpha);
            return;
        }

        self.stamp_rounded_corners(w, h);
        self.cur_w = w;
        self.cur_h = h;

        let mut pt_src = POINT { x: 0, y: 0 };
        let mut pt_dst = POINT {
            x: rect.left,
            y: rect.top,
        };
        let mut size = SIZE { cx: w, cy: h };
        let mut blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER,
            BlendFlags: 0,
            SourceConstantAlpha: initial_alpha,
            AlphaFormat: AC_SRC_ALPHA,
        };
        let screen_dc = GetDC(ptr::null_mut());
        UpdateLayeredWindow(
            self.hwnd,
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

        self.last_pos = pt_dst;
        self.last_alpha = initial_alpha;
    }

    /// Updates ONLY hardware position/opacity via `hdcSrc = NULL` during the 125 FPS crossfade.
    unsafe fn update_frame(&mut self, rect: RECT, alpha: u8) {
        let w = (rect.right - rect.left).clamp(1, self.max_w);
        let h = (rect.bottom - rect.top).clamp(1, self.max_h);
        if w != self.cur_w || h != self.cur_h {
            self.prepare_shape(rect, alpha);
            return;
        }

        let pos_changed = rect.left != self.last_pos.x || rect.top != self.last_pos.y;
        let alpha_changed = alpha != self.last_alpha;
        if !pos_changed && !alpha_changed {
            return;
        }

        self.last_pos = POINT {
            x: rect.left,
            y: rect.top,
        };
        self.last_alpha = alpha;

        let mut pt_dst = self.last_pos;
        let mut blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA,
        };
        UpdateLayeredWindow(
            self.hwnd,
            ptr::null_mut(),
            if pos_changed {
                &mut pt_dst
            } else {
                ptr::null_mut()
            },
            ptr::null_mut(),
            ptr::null_mut(), // hdcSrc = NULL: zero pixel transfer!
            ptr::null_mut(),
            0,
            &mut blend,
            ULW_ALPHA,
        );
    }

    /// Sets `SourceConstantAlpha = 0` while keeping the window `WS_VISIBLE` so DWM never destroys its visual!
    unsafe fn set_alpha_zero_warm(&mut self) {
        if self.visible && self.last_alpha != 0 {
            self.last_alpha = 0;
            let mut blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER,
                BlendFlags: 0,
                SourceConstantAlpha: 0,
                AlphaFormat: AC_SRC_ALPHA,
            };
            UpdateLayeredWindow(
                self.hwnd,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut blend,
                ULW_ALPHA,
            );
        }
    }

    unsafe fn hide(&mut self) {
        if self.visible {
            ShowWindow(self.hwnd, SW_HIDE);
            self.visible = false;
            self.last_alpha = 0;
        }
    }
}

impl Drop for TransitionMask {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
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

pub struct WindowDimmer {
    backdrops_primary: Vec<MonitorBackdrop>,
    backdrops_secondary: Vec<MonitorBackdrop>,
    new_mask: TransitionMask,

    current_active: HWND,
    previous_active: HWND,
    prestaged_hover_hwnd: HWND,
    pre_click_grace_secs: f32,
    fade_progress: f32,
}

impl WindowDimmer {
    pub fn new() -> Self {
        unsafe {
            let hinstance = GetModuleHandleW(ptr::null());
            let class_name = to_wstring("AntiBurnWindowDimOverlay");

            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(dim_overlay_wndproc),
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

            let work_rects = get_all_monitor_work_rects();
            let mut backdrops_primary = Vec::with_capacity(work_rects.len());
            let mut backdrops_secondary = Vec::with_capacity(work_rects.len());
            let mut max_w = 1920;
            let mut max_h = 1080;
            for r in work_rects {
                max_w = max_w.max(r.right - r.left);
                max_h = max_h.max(r.bottom - r.top);
                backdrops_primary.push(MonitorBackdrop::new(r));
                backdrops_secondary.push(MonitorBackdrop::new(r));
            }

            let new_mask = TransitionMask::new("AntiBurn New Active Mask", max_w, max_h);

            Self {
                backdrops_primary,
                backdrops_secondary,
                new_mask,
                current_active: ptr::null_mut(),
                previous_active: ptr::null_mut(),
                prestaged_hover_hwnd: ptr::null_mut(),
                pre_click_grace_secs: 0.0,
                fade_progress: 1.0,
            }
        }
    }

    /// Clamps a window rectangle to the work area (`rcWork`, excluding taskbar) of the monitor it is on.
    fn clamp_to_work_area(&self, rect: RECT) -> RECT {
        let cx = (rect.left + rect.right) / 2;
        let cy = (rect.top + rect.bottom) / 2;
        for bd in &self.backdrops_primary {
            let w = bd.work_rect;
            if cx >= w.left && cx <= w.right && cy >= w.top && cy <= w.bottom + 120 {
                return RECT {
                    left: rect.left.max(w.left),
                    top: rect.top.max(w.top),
                    right: rect.right.min(w.right),
                    bottom: rect.bottom.min(w.bottom),
                };
            }
        }
        if let Some(first) = self.backdrops_primary.first() {
            let w = first.work_rect;
            return RECT {
                left: rect.left.max(w.left),
                top: rect.top.max(w.top),
                right: rect.right.min(w.right),
                bottom: rect.bottom.min(w.bottom),
            };
        }
        rect
    }

    fn collect_own_hwnds(&self, extra_own_hwnds: &[HWND]) -> Vec<HWND> {
        let mut all_own = Vec::with_capacity(
            self.backdrops_primary.len()
                + self.backdrops_secondary.len()
                + 1
                + extra_own_hwnds.len(),
        );
        for bd in &self.backdrops_primary {
            all_own.push(bd.hwnd);
        }
        for bd in &self.backdrops_secondary {
            all_own.push(bd.hwnd);
        }
        all_own.push(self.new_mask.hwnd);
        all_own.extend_from_slice(extra_own_hwnds);
        all_own
    }

    unsafe fn start_transition(&mut self, old_win: HWND, new_win: HWND, cfg: &AppConfig) {
        let base_alpha = (cfg.inactive_dim_amount.clamp(0.0, 0.95) * 255.0).round() as u8;
        if cfg.active_window_fade_enabled {
            // Case 1: Switching from Window A (`old_win`) to Window B (`new_win`)
            if !old_win.is_null() && !new_win.is_null() {
                if let (Some(raw_prev), Some(raw_cur)) = (
                    get_visible_window_rect(old_win),
                    get_visible_window_rect(new_win),
                ) {
                    let prev_r = self.clamp_to_work_area(raw_prev);
                    let cur_r = self.clamp_to_work_area(raw_cur);
                    if prev_r.right > prev_r.left
                        && prev_r.bottom > prev_r.top
                        && cur_r.right > cur_r.left
                        && cur_r.bottom > cur_r.top
                    {
                        self.previous_active = old_win;
                        self.current_active = new_win;
                        self.fade_progress = 0.0;

                        self.new_mask.prepare_shape(cur_r, base_alpha);

                        let mut anchor_b = self.current_active;
                        for bd in &mut self.backdrops_primary {
                            bd.show_and_set_alpha(0, anchor_b);
                            anchor_b = bd.hwnd;
                        }
                        let mut anchor_a = self.previous_active;
                        for bd in &mut self.backdrops_secondary {
                            bd.show_and_set_alpha(base_alpha, anchor_a);
                            anchor_a = bd.hwnd;
                        }
                        return;
                    }
                }
            }
            // Case 2: No window was active (`old_win` is null) -> Focusing Window B (`new_win`)
            else if old_win.is_null() && !new_win.is_null() {
                if let Some(raw_cur) = get_visible_window_rect(new_win) {
                    let cur_r = self.clamp_to_work_area(raw_cur);
                    if cur_r.right > cur_r.left && cur_r.bottom > cur_r.top {
                        self.previous_active = ptr::null_mut();
                        self.current_active = new_win;
                        self.fade_progress = 0.0;

                        self.new_mask.prepare_shape(cur_r, base_alpha);

                        let mut anchor_b = self.current_active;
                        for bd in &mut self.backdrops_primary {
                            bd.show_and_set_alpha(base_alpha, anchor_b);
                            anchor_b = bd.hwnd;
                        }
                        for bd in &mut self.backdrops_secondary {
                            bd.set_alpha_zero_warm();
                        }
                        return;
                    }
                }
            }
            // Case 3: Window A (`old_win`) lost focus and NO window is now active (`new_win` is null)
            else if !old_win.is_null() && new_win.is_null() {
                if get_visible_window_rect(old_win).is_some() {
                    self.previous_active = old_win;
                    self.current_active = ptr::null_mut();
                    self.fade_progress = 0.0;
                    self.new_mask.set_alpha_zero_warm();

                    for bd in &mut self.backdrops_primary {
                        bd.show_at_top_of_normal_band(0);
                    }
                    let mut anchor_a = self.previous_active;
                    for bd in &mut self.backdrops_secondary {
                        bd.show_and_set_alpha(base_alpha, anchor_a);
                        anchor_a = bd.hwnd;
                    }
                    return;
                }
            }
        }
        self.current_active = new_win;
        self.previous_active = ptr::null_mut();
        self.fade_progress = 1.0;
    }

    /// Called synchronously from `WH_MOUSE_LL` on `WM_LBUTTONDOWN` / `WM_RBUTTONDOWN` / `WM_MBUTTONDOWN`
    /// BEFORE `win32k.sys` delivers the click or raises the clicked inactive window in the Z-order!
    /// Immediately activates `new_mask` at `base_alpha` over the clicked inactive window BEFORE `CallNextHookEx`
    /// returns to `win32k.sys`, so when `win32k.sys` raises Window B to the front of the normal Z-order band,
    /// `new_mask` (`HWND_TOPMOST`) is ALREADY covering Window B at `base_alpha` with zero white flash!
    pub fn on_pre_mouse_click(
        &mut self,
        pt: POINT,
        cfg: &AppConfig,
        extra_own_hwnds: &[HWND],
        screensaver_visible: bool,
    ) {
        unsafe {
            if !cfg.inactive_dim_enabled
                || cfg.inactive_dim_amount <= 0.01
                || !cfg.active_window_fade_enabled
                || screensaver_visible
            {
                return;
            }

            let all_own = self.collect_own_hwnds(extra_own_hwnds);
            let raw_hit = WindowFromPoint(pt);
            if let Some(clicked_win) = resolve_eligible_active_window(raw_hit, &all_own) {
                if clicked_win != self.current_active {
                    let old_win = self.current_active;
                    self.pre_click_grace_secs = 0.14;
                    self.start_transition(old_win, clicked_win, cfg);
                    if !old_win.is_null() {
                        let base_alpha =
                            (cfg.inactive_dim_amount.clamp(0.0, 0.95) * 255.0).round() as u8;
                        let mut anchor = clicked_win;
                        for bd in &mut self.backdrops_secondary {
                            bd.show_and_set_alpha(base_alpha, anchor);
                            anchor = bd.hwnd;
                        }
                    }
                }
            }
        }
    }

    pub fn update(
        &mut self,
        cfg: &AppConfig,
        dt_secs: f32,
        extra_own_hwnds: &[HWND],
        screensaver_visible: bool,
    ) {
        unsafe {
            if !cfg.inactive_dim_enabled || cfg.inactive_dim_amount <= 0.01 {
                self.hide();
                return;
            }

            if screensaver_visible {
                return;
            }

            self.pre_click_grace_secs = (self.pre_click_grace_secs - dt_secs).max(0.0);

            let all_own = self.collect_own_hwnds(extra_own_hwnds);
            let base_alpha_f = cfg.inactive_dim_amount.clamp(0.0, 0.95);
            let base_alpha = (base_alpha_f * 255.0).round() as u8;

            let raw_fg = GetForegroundWindow();
            match resolve_eligible_active_window(raw_fg, &all_own) {
                Some(fg) => {
                    if fg != self.current_active {
                        let is_stale_prev =
                            self.pre_click_grace_secs > 0.0 && fg == self.previous_active;
                        if !is_stale_prev {
                            let old_win = self.current_active;
                            self.pre_click_grace_secs = 0.0;
                            self.start_transition(old_win, fg, cfg);
                        }
                    } else {
                        self.pre_click_grace_secs = 0.0;
                    }
                }
                None => {
                    // No eligible application window is currently selected (e.g., user clicked Desktop,
                    // clicked empty taskbar, or minimized/closed the active window).
                    // Transition to `current_active = null` so ALL open windows + desktop are dimmed!
                    if self.pre_click_grace_secs <= 0.0 && !self.current_active.is_null() {
                        let old_win = self.current_active;
                        self.start_transition(old_win, ptr::null_mut(), cfg);
                    }
                }
            }

            let cur_rect_opt = if !self.current_active.is_null() {
                get_visible_window_rect(self.current_active).map(|r| self.clamp_to_work_area(r))
            } else {
                None
            };

            // If `current_active` was closed or minimized mid-frame, treat as no active window (dim everything!)
            if !self.current_active.is_null() && cur_rect_opt.is_none() {
                self.current_active = ptr::null_mut();
                self.previous_active = ptr::null_mut();
                self.fade_progress = 1.0;
            }

            if self.fade_progress < 1.0 {
                let duration_secs = (cfg.active_window_fade_ms.max(60) as f32) / 1000.0;
                self.fade_progress = (self.fade_progress + dt_secs / duration_secs).min(1.0);

                if self.fade_progress < 1.0 {
                    let t = self.fade_progress;
                    let ease = t * t * (3.0 - 2.0 * t);

                    // Case 1: Crossfading between Window A (`previous_active`) and Window B (`current_active`)
                    if let (Some(cur_rect), true) = (
                        cur_rect_opt,
                        !self.previous_active.is_null()
                            && get_visible_window_rect(self.previous_active).is_some(),
                    ) {
                        let new_alpha =
                            (((1.0 - ease) * (base_alpha as f32)).round() as u8).min(base_alpha);
                        self.new_mask.update_frame(cur_rect, new_alpha);

                        let a_mid = (ease * base_alpha_f).clamp(0.0, base_alpha_f);
                        let alpha_mid_byte = (a_mid * 255.0).round() as u8;
                        let a_bot = ((base_alpha_f - a_mid) / (1.0 - a_mid).max(0.01))
                            .clamp(0.0, base_alpha_f);
                        let alpha_bot_byte = (a_bot * 255.0).round() as u8;

                        let mut anchor_b = self.current_active;
                        for bd in &mut self.backdrops_primary {
                            bd.show_and_set_alpha(alpha_mid_byte, anchor_b);
                            anchor_b = bd.hwnd;
                        }
                        let mut anchor_a = self.previous_active;
                        for bd in &mut self.backdrops_secondary {
                            bd.show_and_set_alpha(alpha_bot_byte, anchor_a);
                            anchor_a = bd.hwnd;
                        }
                        return;
                    }
                    // Case 2: Fading in Window B (`current_active`) from all-dimmed state (`previous_active` is null)
                    else if let Some(cur_rect) = cur_rect_opt {
                        let new_alpha =
                            (((1.0 - ease) * (base_alpha as f32)).round() as u8).min(base_alpha);
                        self.new_mask.update_frame(cur_rect, new_alpha);

                        let mut anchor_b = self.current_active;
                        for bd in &mut self.backdrops_primary {
                            bd.show_and_set_alpha(base_alpha, anchor_b);
                            anchor_b = bd.hwnd;
                        }
                        for bd in &mut self.backdrops_secondary {
                            bd.set_alpha_zero_warm();
                        }
                        return;
                    }
                    // Case 3: Fading out Window A (`previous_active`) to all-dimmed state (`current_active` is null)
                    else if !self.previous_active.is_null()
                        && get_visible_window_rect(self.previous_active).is_some()
                    {
                        self.new_mask.set_alpha_zero_warm();

                        let a_mid = (ease * base_alpha_f).clamp(0.0, base_alpha_f);
                        let alpha_mid_byte = (a_mid * 255.0).round() as u8;
                        let a_bot = ((base_alpha_f - a_mid) / (1.0 - a_mid).max(0.01))
                            .clamp(0.0, base_alpha_f);
                        let alpha_bot_byte = (a_bot * 255.0).round() as u8;

                        for bd in &mut self.backdrops_primary {
                            bd.show_at_top_of_normal_band(alpha_mid_byte);
                        }
                        let mut anchor_a = self.previous_active;
                        for bd in &mut self.backdrops_secondary {
                            bd.show_and_set_alpha(alpha_bot_byte, anchor_a);
                            anchor_a = bd.hwnd;
                        }
                        return;
                    } else {
                        self.fade_progress = 1.0;
                    }
                }
            }

            // Steady state:
            if !self.current_active.is_null() {
                // Active window is selected: `backdrops_primary` sits immediately behind `current_active`
                let mut anchor = self.current_active;
                for bd in &mut self.backdrops_primary {
                    bd.show_and_set_alpha(base_alpha, anchor);
                    anchor = bd.hwnd;
                }
            } else {
                // NO active window is selected: `backdrops_primary` sits at the top of the normal window band
                // so ALL open windows and the desktop are dimmed!
                for bd in &mut self.backdrops_primary {
                    bd.show_at_top_of_normal_band(base_alpha);
                }
            }
            for bd in &mut self.backdrops_secondary {
                bd.set_alpha_zero_warm();
            }
            self.new_mask.set_alpha_zero_warm();
            self.previous_active = ptr::null_mut();

            // Hover Pre-Staging: while idle between transitions, check which window is under the mouse cursor.
            // If the user is hovering over an inactive window `hover_win != current_active`, pre-stage `new_mask`
            // (`alpha = 0`) to `hover_win`'s exact rectangle so clicking `hover_win` requires ZERO bitmap upload!
            if cfg.active_window_fade_enabled {
                let mut pt = POINT { x: 0, y: 0 };
                if GetCursorPos(&mut pt) != 0 {
                    let raw_hover = WindowFromPoint(pt);
                    if let Some(hover_win) = resolve_eligible_active_window(raw_hover, &all_own) {
                        if hover_win != self.current_active {
                            if let Some(raw_hr) = get_visible_window_rect(hover_win) {
                                let hr = self.clamp_to_work_area(raw_hr);
                                if hr.right > hr.left && hr.bottom > hr.top {
                                    self.new_mask.prepare_shape(hr, 0);
                                    self.prestaged_hover_hwnd = hover_win;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn hide(&mut self) {
        unsafe {
            for bd in &mut self.backdrops_primary {
                bd.hide();
            }
            for bd in &mut self.backdrops_secondary {
                bd.hide();
            }
            self.new_mask.hide();
        }
    }
}
