use crate::config::AppConfig;
use crate::win_apis::{
    distance_to_rect, get_primary_monitor_rect, to_wstring, ShellAndMediaDetector,
};
use std::ptr;
use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HDC, HWND, POINT, RECT, SIZE};
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::shellapi::{SHAppBarMessage, ABM_GETTASKBARPOS, APPBARDATA};
use winapi::um::wingdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
};
use winapi::um::winuser::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowW, GetCursorPos, GetDC,
    GetMonitorInfoW, GetWindow, GetWindowRect, MonitorFromWindow, RegisterClassW, ReleaseDC,
    SetWindowPos, ShowWindow, UpdateLayeredWindow, GW_HWNDPREV, HWND_TOPMOST, MONITORINFO,
    MONITOR_DEFAULTTOPRIMARY, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSENDCHANGING,
    SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

unsafe extern "system" fn taskbar_overlay_wndproc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

pub struct TaskbarDimmer {
    pub overlay_hwnd: HWND,
    current_dim_factor: f32, // 0.0 = full brightness (undimmed), 1.0 = max configured dim
    mem_dc: HDC,
    bmp: winapi::shared::windef::HBITMAP,
    bits: *mut u32,
    dib_w: i32,
    dib_h: i32,
    visible: bool,
    was_screensaver_visible: bool,
    last_alpha: u8,
    last_rect: RECT,
}

impl TaskbarDimmer {
    pub fn new() -> Self {
        unsafe {
            let hinstance = GetModuleHandleW(ptr::null());
            let class_name = to_wstring("AntiBurnTaskbarOverlay");

            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(taskbar_overlay_wndproc),
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

            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                to_wstring("AntiBurn Taskbar Filter").as_ptr(),
                WS_POPUP,
                0,
                0,
                10,
                10,
                ptr::null_mut(),
                ptr::null_mut(),
                hinstance,
                ptr::null_mut(),
            );

            Self {
                overlay_hwnd: hwnd,
                current_dim_factor: 1.0,
                mem_dc: ptr::null_mut(),
                bmp: ptr::null_mut(),
                bits: ptr::null_mut(),
                dib_w: 0,
                dib_h: 0,
                visible: false,
                was_screensaver_visible: false,
                last_alpha: 255,
                last_rect: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
            }
        }
    }

    /// Queries physical pixel coordinates of the primary taskbar.
    /// Prioritizes DPI-aware `Shell_TrayWnd` window rect and `GetMonitorInfoW` (`rcMonitor - rcWork`).
    pub unsafe fn get_taskbar_rect() -> Option<RECT> {
        let tray_name = to_wstring("Shell_TrayWnd");
        let tray_hwnd = FindWindowW(tray_name.as_ptr(), ptr::null());
        if !tray_hwnd.is_null() {
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(tray_hwnd, &mut r) != 0 && r.right > r.left && r.bottom > r.top {
                return Some(r);
            }
        }

        let hmon = MonitorFromWindow(ptr::null_mut(), MONITOR_DEFAULTTOPRIMARY);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut mi) != 0 && mi.rcWork.bottom < mi.rcMonitor.bottom {
            return Some(RECT {
                left: mi.rcMonitor.left,
                top: mi.rcWork.bottom,
                right: mi.rcMonitor.right,
                bottom: mi.rcMonitor.bottom,
            });
        }

        let mut abd: APPBARDATA = std::mem::zeroed();
        abd.cbSize = std::mem::size_of::<APPBARDATA>() as u32;
        if SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) != 0 {
            let r = abd.rc;
            if r.right > r.left && r.bottom > r.top {
                return Some(r);
            }
        }

        let m = get_primary_monitor_rect();
        Some(RECT {
            left: m.left,
            top: (m.bottom - 48).max(m.top),
            right: m.right,
            bottom: m.bottom,
        })
    }

    unsafe fn ensure_dib(&mut self, w: i32, h: i32) -> bool {
        if w == self.dib_w && h == self.dib_h && !self.mem_dc.is_null() {
            return false;
        }
        if !self.bmp.is_null() {
            DeleteObject(self.bmp as *mut _);
            self.bmp = ptr::null_mut();
        }
        if !self.mem_dc.is_null() {
            DeleteDC(self.mem_dc);
            self.mem_dc = ptr::null_mut();
        }

        let screen_dc = GetDC(ptr::null_mut());
        self.mem_dc = CreateCompatibleDC(screen_dc);
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
        let mut bits_ptr: *mut winapi::ctypes::c_void = ptr::null_mut();
        self.bmp = CreateDIBSection(
            self.mem_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            ptr::null_mut(),
            0,
        );
        SelectObject(self.mem_dc, self.bmp as *mut _);
        ReleaseDC(ptr::null_mut(), screen_dc);

        self.bits = bits_ptr as *mut u32;
        self.dib_w = w;
        self.dib_h = h;

        if !self.bits.is_null() {
            let slice = std::slice::from_raw_parts_mut(self.bits, (w * h) as usize);
            slice.fill(0xFF000000);
        }
        true
    }

    pub fn update(
        &mut self,
        cfg: &AppConfig,
        detector: &ShellAndMediaDetector,
        dt_secs: f32,
        screensaver_visible: bool,
    ) {
        unsafe {
            if !cfg.taskbar_dim_enabled {
                if self.visible {
                    ShowWindow(self.overlay_hwnd, SW_HIDE);
                    self.visible = false;
                }
                self.was_screensaver_visible = screensaver_visible;
                return;
            }

            let just_exited_screensaver = self.was_screensaver_visible && !screensaver_visible;
            self.was_screensaver_visible = screensaver_visible;

            let rect = match Self::get_taskbar_rect() {
                Some(r) => r,
                None => return,
            };

            // While the screensaver is running, ALWAYS target full taskbar dimming (1.0) even if the
            // cursor is resting in the bottom-left hot corner or on the taskbar when the screensaver starts!
            let target_dim_factor = if screensaver_visible {
                1.0
            } else if detector.is_start_or_flyout_open() {
                0.0
            } else {
                let mut cursor = POINT { x: 0, y: 0 };
                GetCursorPos(&mut cursor);
                let dist = distance_to_rect(cursor, rect);
                let prox = cfg.taskbar_proximity_px.max(1) as f32;

                if dist <= 0.0 {
                    0.0
                } else if dist < prox {
                    let t = (dist / prox).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                } else {
                    1.0
                }
            };

            let speed = if target_dim_factor == 0.0 { 18.0 } else { 10.0 };
            let step = (speed * dt_secs).clamp(0.0, 1.0);
            self.current_dim_factor += (target_dim_factor - self.current_dim_factor) * step;

            let effective_dim = (self.current_dim_factor * cfg.taskbar_dim_amount).clamp(0.0, 0.95);
            let alpha_byte = (effective_dim * 255.0).round() as u8;

            let w = (rect.right - rect.left).max(1);
            let h = (rect.bottom - rect.top).max(1);
            let dib_recreated = self.ensure_dib(w, h);

            let mut just_shown = false;
            if !self.visible {
                ShowWindow(self.overlay_hwnd, SW_SHOWNOACTIVATE);
                self.visible = true;
                just_shown = true;
            }

            let rect_changed = dib_recreated
                || rect.left != self.last_rect.left
                || rect.top != self.last_rect.top
                || rect.right != self.last_rect.right
                || rect.bottom != self.last_rect.bottom;

            if rect_changed || just_shown || just_exited_screensaver {
                self.last_rect = rect;
                self.last_alpha = alpha_byte;

                let mut pt_src = POINT { x: 0, y: 0 };
                let mut pt_dst = POINT {
                    x: rect.left,
                    y: rect.top,
                };
                let mut size = SIZE { cx: w, cy: h };
                let mut blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER,
                    BlendFlags: 0,
                    SourceConstantAlpha: alpha_byte,
                    AlphaFormat: AC_SRC_ALPHA,
                };

                let screen_dc = GetDC(ptr::null_mut());
                UpdateLayeredWindow(
                    self.overlay_hwnd,
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
            } else if alpha_byte != self.last_alpha {
                self.last_alpha = alpha_byte;
                let mut blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER,
                    BlendFlags: 0,
                    SourceConstantAlpha: alpha_byte,
                    AlphaFormat: AC_SRC_ALPHA,
                };
                // hdcSrc = NULL: hardware-only opacity update at 125 FPS!
                UpdateLayeredWindow(
                    self.overlay_hwnd,
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

            // Keep `overlay_hwnd` at the top of `HWND_TOPMOST` above `Shell_TrayWnd` whenever the screensaver
            // is not actively covering the screen. Windows 11 `explorer.exe` periodically promotes `Shell_TrayWnd`
            // in the `HWND_TOPMOST` band (especially after a fullscreen window / screensaver closes!), so checking
            // `GetWindow(overlay_hwnd, GW_HWNDPREV)` ensures `Shell_TrayWnd` can NEVER bury our taskbar filter!
            if !screensaver_visible && !GetWindow(self.overlay_hwnd, GW_HWNDPREV).is_null() {
                SetWindowPos(
                    self.overlay_hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_NOOWNERZORDER,
                );
            }
        }
    }
}

impl Drop for TaskbarDimmer {
    fn drop(&mut self) {
        unsafe {
            if !self.overlay_hwnd.is_null() {
                DestroyWindow(self.overlay_hwnd);
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
