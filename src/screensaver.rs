use crate::config::AppConfig;
use crate::win_apis::{append_idle_log, get_primary_monitor_rect, to_wstring};
use std::f32::consts::{PI, TAU};
use std::ptr;
use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::wingdi::{
    CreateSolidBrush, SetStretchBltMode, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    COLORONCOLOR, DIB_RGB_COLORS, SRCCOPY,
};
use winapi::um::winuser::{
    BeginPaint, CreateWindowExW, DefWindowProcW, DestroyWindow, EndPaint, GetDC, GetWindow,
    RegisterClassW, ReleaseDC, SendNotifyMessageW, SetCursor, SetLayeredWindowAttributes,
    SetWindowPos, ShowCursor, ShowWindow, GW_HWNDPREV, HWND_BROADCAST, HWND_TOPMOST, LWA_ALPHA,
    PAINTSTRUCT, SC_MONITORPOWER, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER,
    SWP_NOSENDCHANGING, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, WM_ERASEBKGND, WM_PAINT,
    WM_SETCURSOR, WM_SYSCOMMAND, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

unsafe extern "system" fn screensaver_wndproc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Suppress background erase so DWM never flashes during fade-in
        WM_ERASEBKGND => 1,
        // Always hide the mouse cursor whenever the screensaver window is visible
        WM_SETCURSOR => {
            SetCursor(ptr::null_mut());
            1
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[derive(Clone, Copy)]
struct Metaball {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    anchor_x: f32,
    anchor_y: f32,
    phase: f32,
    radius: f32,
    charge: f32,
    hue: f32,
    hue_speed: f32,
}

#[derive(Clone, Copy)]
struct FlowParticle {
    x: f32,
    y: f32,
    prev_x: f32,
    prev_y: f32,
    vx: f32,
    vy: f32,
    hue: f32,
    life: f32,
    max_life: f32,
}

pub struct ScreensaverEngine {
    pub hwnd: HWND,
    visible: bool,
    cursor_hidden: bool,
    fade_in: f32,
    blackout_fade: f32,
    blackout_frame_presented: bool,
    monitor_sleep_timer_secs: f32,
    monitor_sleep_logged: bool,
    sim_time: f32,
    frame_accum: f32,
    frame_counter: u32,
    last_alpha_byte: u8,

    metaballs: Vec<Metaball>,
    particles: Vec<FlowParticle>,
    rng_state: u64,

    // Low-res simulation buffer (exact 1/10th monitor resolution, e.g. 512x216 on 5120x2160 = only 442 KB!)
    buf_w: usize,
    buf_h: usize,
    accum_rgb: Vec<[f32; 3]>,
    pixel_buf: Vec<u32>,
    bmi: BITMAPINFO,

    mon_x: i32,
    mon_y: i32,
    screen_w: i32,
    screen_h: i32,
}

impl ScreensaverEngine {
    pub fn new() -> Self {
        unsafe {
            let hinstance = GetModuleHandleW(ptr::null());
            let class_name = to_wstring("AntiBurnScreensaverWindow");

            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(screensaver_wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: ptr::null_mut(),
                hCursor: ptr::null_mut(),
                hbrBackground: CreateSolidBrush(0x00000000),
                lpszMenuName: ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&wc);

            let mrect = get_primary_monitor_rect();
            let mw = (mrect.right - mrect.left).max(1920);
            let mh = (mrect.bottom - mrect.top).max(1080);

            // Created WITHOUT WS_EX_TRANSPARENT so that while the screensaver is visible,
            // it owns mouse hit-testing across the entire monitor and hides the mouse cursor!
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                to_wstring("AntiBurn OLED Screensaver").as_ptr(),
                WS_POPUP,
                mrect.left,
                mrect.top,
                mw,
                mh,
                ptr::null_mut(),
                ptr::null_mut(),
                hinstance,
                ptr::null_mut(),
            );

            // Initialize layered window in hardware LWA_ALPHA mode (NOT 44MB UpdateLayeredWindow mode!)
            SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);

            // Match the exact aspect ratio of the monitor at ~240p vertical resolution
            // (e.g., 512 x 216 on a 5120 x 2160 monitor, or 426 x 240 on 16:9)
            let buf_h = 216usize;
            let buf_w = (((mw as usize) * buf_h) / (mh as usize)).clamp(320, 640);

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: buf_w as i32,
                    biHeight: -(buf_h as i32), // top-down DIB
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

            let mut engine = Self {
                hwnd,
                visible: false,
                cursor_hidden: false,
                fade_in: 0.0,
                blackout_fade: 0.0,
                blackout_frame_presented: false,
                monitor_sleep_timer_secs: 0.0,
                monitor_sleep_logged: false,
                sim_time: 0.0,
                frame_accum: 0.0,
                frame_counter: 0,
                last_alpha_byte: 0,
                metaballs: Vec::new(),
                particles: Vec::new(),
                rng_state: 0x853C49E6748FEA9B,
                buf_w,
                buf_h,
                accum_rgb: vec![[0.0; 3]; buf_w * buf_h],
                pixel_buf: vec![0x00000000; buf_w * buf_h],
                bmi,
                mon_x: mrect.left,
                mon_y: mrect.top,
                screen_w: mw,
                screen_h: mh,
            };

            engine.init_simulation(7, 1800);
            engine
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn is_in_display_sleep(&self) -> bool {
        self.visible && self.blackout_frame_presented
    }

    #[inline(always)]
    fn next_f32(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng_state = x;
        let val = x.wrapping_mul(0x2545F4914F6CDD1D);
        ((val >> 40) as u32 as f32) / ((1u32 << 24) as f32)
    }

    fn init_simulation(&mut self, metaball_count: usize, particle_count: usize) {
        let w = self.buf_w as f32;
        let h = self.buf_h as f32;

        self.metaballs.clear();
        for i in 0..metaball_count {
            // Evenly distribute phases around 0..TAU so half start on the LEFT (cos < 0) and half on the RIGHT (cos > 0)!
            let phase = (i as f32 + 0.5) * (TAU / (metaball_count.max(1) as f32));
            let anchor_x = (w * 0.5) + phase.cos() * (w * 0.41);
            let anchor_y = (h * 0.5) + (phase * 1.618).sin() * (h * 0.38);

            let angle = phase + PI * 0.5;
            let speed = 12.0 + self.next_f32() * 10.0;
            let radius = 28.0 + self.next_f32() * 16.0;
            let charge = if i % 2 == 0 { 1.0 } else { -1.0 };
            let hue = (i as f32 / metaball_count.max(1) as f32 + self.next_f32() * 0.12) % 1.0;
            let rh = self.next_f32();

            // Start each metaball directly at its own balanced anchor so left & right of monitor are equally populated!
            self.metaballs.push(Metaball {
                x: anchor_x,
                y: anchor_y,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed,
                anchor_x,
                anchor_y,
                phase,
                radius,
                charge,
                hue,
                hue_speed: 0.015 + rh * 0.02,
            });
        }

        self.particles.clear();
        for _ in 0..particle_count {
            let x = self.next_f32() * (w - 1.001);
            let y = self.next_f32() * (h - 1.001);
            let hue = self.next_f32();
            let max_life = 5.0 + self.next_f32() * 8.0;
            let life = self.next_f32() * max_life;
            self.particles.push(FlowParticle {
                x,
                y,
                prev_x: x,
                prev_y: y,
                vx: 0.0,
                vy: 0.0,
                hue,
                life,
                max_life,
            });
        }
    }

    pub fn update_and_render(
        &mut self,
        cfg: &AppConfig,
        active: bool,
        ss_elapsed_secs: f32,
        force_opaque_bg: bool,
        force_turn_off_monitor: bool,
        dt_secs: f32,
        main_hwnd: HWND,
    ) {
        unsafe {
            if !active {
                self.blackout_fade = 0.0;
                self.blackout_frame_presented = false;
                self.monitor_sleep_timer_secs = 0.0;
                self.monitor_sleep_logged = false;
                if !self.visible {
                    if self.cursor_hidden {
                        ShowCursor(1);
                        self.cursor_hidden = false;
                    }
                    return;
                }
                self.fade_in = (self.fade_in - dt_secs * 3.5).max(0.0);
                if self.fade_in <= 0.005 {
                    ShowWindow(self.hwnd, SW_HIDE);
                    self.visible = false;
                    self.last_alpha_byte = 0;
                    if self.cursor_hidden {
                        ShowCursor(1);
                        self.cursor_hidden = false;
                    }
                    return;
                }
            } else {
                // Smooth fade-in (faster ~1.2s fade when directly turning off monitor via hot corner)
                let fade_speed = if force_turn_off_monitor { 0.85 } else { 0.28 };
                self.fade_in = (self.fade_in + dt_secs * fade_speed).min(1.0);

                // Check if "Turn Off Monitor After" timeout has been reached or forced via Hot Corner
                let turn_off_active = force_turn_off_monitor
                    || (cfg.screensaver_turn_off_mins > 0
                        && ss_elapsed_secs >= (cfg.screensaver_turn_off_mins as f32) * 60.0);
                if turn_off_active {
                    if force_turn_off_monitor {
                        self.blackout_fade = 1.0;
                    } else {
                        self.blackout_fade = (self.blackout_fade + dt_secs * 0.4).min(1.0);
                    }
                    if self.blackout_frame_presented {
                        // Once the pure #000000 black frame has been presented, completely cease all
                        // 60 FPS GDI StretchDIBits/SetWindowPos/SetCursor calls so DWM is 100% idle,
                        // then send SC_MONITORPOWER = 2 to put the physical monitor into hardware sleep!
                        self.monitor_sleep_timer_secs += dt_secs;
                        if self.monitor_sleep_timer_secs >= 0.8 {
                            self.monitor_sleep_timer_secs = -24.0; // Re-assert every 25s if a fake 0px wake turns the panel on
                            if !self.monitor_sleep_logged {
                                self.monitor_sleep_logged = true;
                                if force_turn_off_monitor {
                                    append_idle_log(
                                        "Turn Off Monitor engaged via Bottom-Left Hot Corner — sending SC_MONITORPOWER=2",
                                    );
                                } else {
                                    append_idle_log(&format!(
                                        "Turn Off Monitor engaged (after {} min) — sending SC_MONITORPOWER=2",
                                        cfg.screensaver_turn_off_mins
                                    ));
                                }
                            }
                            if !main_hwnd.is_null() {
                                DefWindowProcW(main_hwnd, WM_SYSCOMMAND, SC_MONITORPOWER, 2);
                            }
                            SendNotifyMessageW(HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER, 2);
                        }
                        return;
                    }
                } else {
                    self.blackout_fade = (self.blackout_fade - dt_secs * 1.5).max(0.0);
                    self.blackout_frame_presented = false;
                    self.monitor_sleep_timer_secs = 0.0;
                    self.monitor_sleep_logged = false;
                }
            }

            // Throttle screensaver physics & StretchDIBits to 60 FPS (16ms) even when main loop runs at 125Hz (8ms)
            self.frame_accum += dt_secs;
            if self.frame_accum < 0.0155 && self.visible {
                return;
            }
            let step_dt = self.frame_accum.clamp(0.008, 0.04);
            self.frame_accum = 0.0;

            let target_mb = cfg.screensaver_metaball_count.clamp(2, 16) as usize;
            let target_pt = cfg.screensaver_particle_count.clamp(200, 5000) as usize;
            if self.metaballs.len() != target_mb || self.particles.len() != target_pt {
                self.init_simulation(target_mb, target_pt);
            }

            self.sim_time += step_dt;
            self.frame_counter = self.frame_counter.wrapping_add(1);

            // Only step particle/metaball physics when not in 100% pure blackout mode!
            if self.blackout_fade < 0.999 {
                let substeps = 3;
                let sub_dt = step_dt / (substeps as f32);
                let vibrancy = cfg.screensaver_vibrancy.clamp(0.1, 1.0) * (1.0 - self.blackout_fade);

                let decay = 0.91f32 * (1.0 - self.blackout_fade * 0.15);
                for px in &mut self.accum_rgb {
                    px[0] *= decay;
                    px[1] *= decay;
                    px[2] *= decay;
                }

                for _ in 0..substeps {
                    self.step_physics_and_splat(sub_dt, vibrancy);
                }
            } else {
                for px in &mut self.accum_rgb {
                    *px = [0.0, 0.0, 0.0];
                }
            }

            // Render & present via hardware StretchDIBits + LWA_ALPHA
            self.render_frame(cfg, ss_elapsed_secs, force_opaque_bg);
        }
    }

    fn step_physics_and_splat(&mut self, dt: f32, vibrancy: f32) {
        let w = self.buf_w as f32;
        let h = self.buf_h as f32;
        let bw = self.buf_w;
        let bh = self.buf_h;
        let n_mb = self.metaballs.len();

        // 1. Update roaming Lissajous anchors spanning from 8% to 92% of the full monitor width
        //    so gravity wells spend equal time on the left third, middle third, and right third!
        for mb in &mut self.metaballs {
            mb.anchor_x = (w * 0.5) + (self.sim_time * 0.065 + mb.phase).cos() * (w * 0.42);
            mb.anchor_y = (h * 0.5) + (self.sim_time * 0.049 + mb.phase * 1.618).sin() * (h * 0.39);
        }

        for i in 0..n_mb {
            let (xi, yi, ri, ax, ay) = (
                self.metaballs[i].x,
                self.metaballs[i].y,
                self.metaballs[i].radius,
                self.metaballs[i].anchor_x,
                self.metaballs[i].anchor_y,
            );

            // Strong enough anchor spring so metaballs never abandon the left side to clump on the right
            let mut fx = (ax - xi) * 0.32;
            let mut fy = (ay - yi) * 0.32;

            for j in 0..n_mb {
                if i == j {
                    continue;
                }
                let dx = self.metaballs[j].x - xi;
                let dy = self.metaballs[j].y - yi;
                let dist_sq = dx * dx + dy * dy + 48.0;
                let dist = dist_sq.sqrt();
                let dir_x = dx / dist;
                let dir_y = dy / dist;

                let combined_r = ri + self.metaballs[j].radius;
                if dist < combined_r * 0.85 {
                    let push = (combined_r * 0.85 - dist) * 2.2;
                    fx -= dir_x * push;
                    fy -= dir_y * push;
                } else if dist < combined_r * 1.65 {
                    // Local bridge attraction only when two wandering metaballs pass close to each other
                    let pull = 8.0 / (dist * 0.06 + 1.0);
                    fx += dir_x * pull;
                    fy += dir_y * pull;
                }
            }

            let mb = &mut self.metaballs[i];
            mb.vx = (mb.vx + fx * dt) * 0.985;
            mb.vy = (mb.vy + fy * dt) * 0.985;

            let max_speed = 26.0;
            let speed = (mb.vx * mb.vx + mb.vy * mb.vy).sqrt();
            if speed > max_speed {
                mb.vx = (mb.vx / speed) * max_speed;
                mb.vy = (mb.vy / speed) * max_speed;
            }

            mb.x += mb.vx * dt;
            mb.y += mb.vy * dt;

            let pad = 8.0;
            if mb.x < pad {
                mb.x = pad;
                mb.vx = mb.vx.abs() * 0.9;
            } else if mb.x > w - pad {
                mb.x = w - pad;
                mb.vx = -mb.vx.abs() * 0.9;
            }
            if mb.y < pad {
                mb.y = pad;
                mb.vy = mb.vy.abs() * 0.9;
            } else if mb.y > h - pad {
                mb.y = h - pad;
                mb.vy = -mb.vy.abs() * 0.9;
            }

            mb.hue = (mb.hue + mb.hue_speed * dt) % 1.0;
        }

        let mut mb_cache = [([0.0f32; 2], [0.0f32; 2], 0.0f32, 0.0f32, 0.0f32, 0.0f32); 16];
        for (i, mb) in self.metaballs.iter().take(16).enumerate() {
            mb_cache[i] = (
                [mb.x, mb.y],
                [mb.vx, mb.vy],
                mb.radius * mb.radius,
                mb.charge,
                (mb.hue * TAU).sin(),
                (mb.hue * TAU).cos(),
            );
        }

        // 2. Update Flow Field Particles with seamless edge flow (no turnaround pile-up 50-100px from edges!)
        let t = self.sim_time;
        let max_x = w - 1.001;
        let max_y = h - 1.001;

        for idx in 0..self.particles.len() {
            let mut p = self.particles[idx];
            p.prev_x = p.x;
            p.prev_y = p.y;
            p.life += dt;

            if p.life >= p.max_life {
                let r1 = self.next_f32();
                let r2 = self.next_f32();
                let r3 = self.next_f32();
                if r1 < 0.60 && n_mb > 0 {
                    let mb_idx = ((r2 * (n_mb as f32)) as usize).min(n_mb - 1);
                    let mb = self.metaballs[mb_idx];
                    let ang = r3 * TAU;
                    let rad = mb.radius * (0.55 + 0.60 * self.next_f32());
                    p.x = mb.x + ang.cos() * rad;
                    p.y = mb.y + ang.sin() * rad;
                    p.hue = mb.hue;
                } else {
                    p.x = r2 * max_x;
                    p.y = r3 * max_y;
                }
                p.prev_x = p.x;
                p.prev_y = p.y;
                p.vx = 0.0;
                p.vy = 0.0;
                p.life = 0.0;
                p.max_life = 5.0 + self.next_f32() * 8.0;
            }

            // 2D spatially varying vortex lattice (depends on BOTH nx and ny so no entire row pushes into a wall)
            let nx = p.x * 0.022;
            let ny = p.y * 0.022;
            let vx_curl = (nx + t * 0.16).sin() * (ny - t * 0.12).cos()
                + (ny * 1.3 - nx * 0.8 + t * 0.14).sin() * 0.65;
            let vy_curl = -(nx + t * 0.16).cos() * (ny - t * 0.12).sin()
                + (nx * 1.3 + ny * 0.8 - t * 0.11).cos() * 0.65;
            let mut target_vx = vx_curl * 22.0;
            let mut target_vy = vy_curl * 22.0;

            let mut dominant_weight = 0.10f32;
            let mut blended_hue_sin = (p.hue * TAU).sin() * dominant_weight;
            let mut blended_hue_cos = (p.hue * TAU).cos() * dominant_weight;

            for &(pos, vel, r2, charge, h_sin, h_cos) in mb_cache.iter().take(n_mb) {
                let dx = p.x - pos[0];
                let dy = p.y - pos[1];
                let d2 = dx * dx + dy * dy + 28.0;
                let influence = r2 / d2;

                if influence > 0.04 {
                    let inv_d = 1.0 / d2.sqrt();
                    let rx = dx * inv_d;
                    let ry = dy * inv_d;

                    let tx = -ry * charge;
                    let ty = rx * charge;

                    let swirl = (influence * 42.0).min(56.0);
                    target_vx += tx * swirl;
                    target_vy += ty * swirl;

                    let radial = if influence > 1.15 {
                        14.0
                    } else {
                        -8.0 * influence
                    };
                    target_vx += rx * radial;
                    target_vy += ry * radial;

                    target_vx += vel[0] * (influence * 0.55).min(0.8);
                    target_vy += vel[1] * (influence * 0.55).min(0.8);

                    let w_hue = influence.min(2.0);
                    blended_hue_sin += h_sin * w_hue;
                    blended_hue_cos += h_cos * w_hue;
                    dominant_weight += w_hue;
                }
            }

            if dominant_weight > 0.11 {
                let mut new_hue = blended_hue_sin.atan2(blended_hue_cos) / TAU;
                if new_hue < 0.0 {
                    new_hue += 1.0;
                }
                p.hue = new_hue;
            }

            // No screen-edge wall containment: particles freely leak off the edges of the monitor
            // into off-screen space (`p.x < 0`, `p.x > w`, `p.y < 0`, `p.y > h`), and experience a
            // smooth, wide-arc orbital return pull while off-screen so they curve naturally back on!
            if p.x < 0.0 {
                target_vx += (-p.x) * 0.55;
            } else if p.x > max_x {
                target_vx -= (p.x - max_x) * 0.55;
            }
            if p.y < 0.0 {
                target_vy += (-p.y) * 0.55;
            } else if p.y > max_y {
                target_vy -= (p.y - max_y) * 0.55;
            }

            p.vx += (target_vx - p.vx) * (dt * 4.5).min(1.0);
            p.vy += (target_vy - p.vy) * (dt * 4.5).min(1.0);
            p.x += p.vx * dt;
            p.y += p.vy * dt;

            self.particles[idx] = p;

            // Sub-pixel Bilinear Anti-Aliased Streak Splatting inside the low-res grid
            // (Uses signed `isize` bounds check so off-screen particles at `sx < 0` or `sy < 0`
            // never clamp to pixel 0 or pile up on the screen borders!)
            let life_norm = (p.life / p.max_life).clamp(0.0, 1.0);
            let env = (life_norm * PI).sin();
            let intensity = 0.14 * env * vibrancy;
            let (r, g, b) = hsv_to_rgb(p.hue, 0.92, intensity);

            for s in 0..2 {
                let frac = (s as f32 + 0.5) * 0.5;
                let sx = p.prev_x + (p.x - p.prev_x) * frac;
                let sy = p.prev_y + (p.y - p.prev_y) * frac;

                let ix_s = sx.floor() as isize;
                let iy_s = sy.floor() as isize;
                if ix_s >= 0 && iy_s >= 0 {
                    let ix = ix_s as usize;
                    let iy = iy_s as usize;
                    if ix + 1 < bw && iy + 1 < bh {
                        let fx = sx - (ix as f32);
                        let fy = sy - (iy as f32);
                        let w00 = (1.0 - fx) * (1.0 - fy);
                        let w10 = fx * (1.0 - fy);
                        let w01 = (1.0 - fx) * fy;
                        let w11 = fx * fy;

                        let i00 = iy * bw + ix;
                        let i10 = i00 + 1;
                        let i01 = i00 + bw;
                        let i11 = i01 + 1;

                        self.accum_rgb[i00][0] = (self.accum_rgb[i00][0] + r * w00).min(1.0);
                        self.accum_rgb[i00][1] = (self.accum_rgb[i00][1] + g * w00).min(1.0);
                        self.accum_rgb[i00][2] = (self.accum_rgb[i00][2] + b * w00).min(1.0);

                        self.accum_rgb[i10][0] = (self.accum_rgb[i10][0] + r * w10).min(1.0);
                        self.accum_rgb[i10][1] = (self.accum_rgb[i10][1] + g * w10).min(1.0);
                        self.accum_rgb[i10][2] = (self.accum_rgb[i10][2] + b * w10).min(1.0);

                        self.accum_rgb[i01][0] = (self.accum_rgb[i01][0] + r * w01).min(1.0);
                        self.accum_rgb[i01][1] = (self.accum_rgb[i01][1] + g * w01).min(1.0);
                        self.accum_rgb[i01][2] = (self.accum_rgb[i01][2] + b * w01).min(1.0);

                        self.accum_rgb[i11][0] = (self.accum_rgb[i11][0] + r * w11).min(1.0);
                        self.accum_rgb[i11][1] = (self.accum_rgb[i11][1] + g * w11).min(1.0);
                        self.accum_rgb[i11][2] = (self.accum_rgb[i11][2] + b * w11).min(1.0);
                    }
                }
            }
        }
    }

    unsafe fn render_frame(
        &mut self,
        cfg: &AppConfig,
        ss_elapsed_secs: f32,
        force_opaque_bg: bool,
    ) {
        let bw = self.buf_w;
        let bh = self.buf_h;
        let alive_mult = (1.0 - self.blackout_fade).clamp(0.0, 1.0);
        let vibrancy = cfg.screensaver_vibrancy.clamp(0.1, 1.0) * alive_mult;

        let total_span = (cfg
            .screensaver_stage2_secs
            .saturating_sub(cfg.screensaver_stage1_secs))
        .max(60) as f32;
        let progressive_darkening = if force_opaque_bg || self.blackout_fade > 0.001 {
            1.0
        } else {
            (ss_elapsed_secs / total_span).clamp(0.0, 1.0)
        };
        let window_alpha_f = (cfg.screensaver_stage1_opacity
            + (1.0 - cfg.screensaver_stage1_opacity) * progressive_darkening)
            * self.fade_in;
        let alpha_byte = (window_alpha_f.clamp(0.0, 1.0) * 255.0).round() as u8;
        if alpha_byte == 0 {
            return;
        }

        if self.blackout_fade >= 0.999 {
            // Pure #000000 blackout: zero pixel emission across the entire screen
            self.pixel_buf.fill(0xFF000000);
        } else {
            let show_metaballs = cfg.screensaver_show_metaballs && alive_mult > 0.001;
            let solid_metaballs = cfg.screensaver_solid_metaballs;

            let mut mb_eval = [([0.0f32; 2], 0.0f32, [0.0f32; 3]); 16];
            let n_mb = self.metaballs.len().min(16);
            if show_metaballs {
                for (i, mb) in self.metaballs.iter().take(n_mb).enumerate() {
                    let (cr, cg, cb) = hsv_to_rgb(mb.hue, 0.95, 1.0);
                    mb_eval[i] = ([mb.x, mb.y], mb.radius * mb.radius, [cr, cg, cb]);
                }
            }

            let frame_seed = (self.frame_counter & 63) as f32;

            // Evaluate only the tiny 512x216 (`110,592` pixels) buffer on the CPU!
            for y in 0..bh {
                let fy = y as f32;
                let row_offset = y * bw;
                for x in 0..bw {
                    let fx = x as f32;

                    let mut pr = self.accum_rgb[row_offset + x][0] * alive_mult;
                    let mut pg = self.accum_rgb[row_offset + x][1] * alive_mult;
                    let mut pb = self.accum_rgb[row_offset + x][2] * alive_mult;

                    if show_metaballs {
                        let mut field = 0.0f32;
                        let mut mr = 0.0f32;
                        let mut mg = 0.0f32;
                        let mut mb_col = 0.0f32;

                        for &(pos, r2, col) in mb_eval.iter().take(n_mb) {
                            let dx = fx - pos[0];
                            let dy = fy - pos[1];
                            let contrib = r2 / (dx * dx + dy * dy + 20.0);
                            field += contrib;
                            mr += col[0] * contrib;
                            mg += col[1] * contrib;
                            mb_col += col[2] * contrib;
                        }

                        let inv_f = 1.0 / (field + 1e-5);
                        let norm_r = mr * inv_f;
                        let norm_g = mg * inv_f;
                        let norm_b = mb_col * inv_f;

                        // If `solid_metaballs` is enabled, fill the interior (`field >= 1.02`) solidly with
                        // the bright rim color instead of letting `rim` decay to a dark center!
                        let rim = if solid_metaballs && field >= 1.02 {
                            1.0
                        } else {
                            let diff = (field - 1.02) * 2.1;
                            (-diff * diff).exp() * (field * 1.5).min(1.0)
                        };
                        let aura = (field * 0.24).min(0.32) * (1.0 - (-field * 2.5).exp());
                        let meta_lum = (rim * 0.62 + aura) * vibrancy;

                        pr = (pr + norm_r * meta_lum).min(1.0);
                        pg = (pg + norm_g * meta_lum).min(1.0);
                        pb = (pb + norm_b * meta_lum).min(1.0);
                    }

                    // Interleaved Gradient Noise dithering so dark color transitions have zero banding
                    let ign = ((52.9829189
                        * ((0.06711056 * fx + 0.00583715 * fy + frame_seed * 0.17).fract()))
                    .fract())
                        - 0.5;
                    let dither = ign * 1.5;

                    let r_u8 = if pr > 0.0015 {
                        (pr * 255.0 + dither).round().clamp(0.0, 255.0) as u32
                    } else {
                        0
                    };
                    let g_u8 = if pg > 0.0015 {
                        (pg * 255.0 + dither).round().clamp(0.0, 255.0) as u32
                    } else {
                        0
                    };
                    let b_u8 = if pb > 0.0015 {
                        (pb * 255.0 + dither).round().clamp(0.0, 255.0) as u32
                    } else {
                        0
                    };

                    self.pixel_buf[row_offset + x] = 0xFF000000 | (r_u8 << 16) | (g_u8 << 8) | b_u8;
                }
            }
        }

        // Only call SetLayeredWindowAttributes when `alpha_byte` actually changes!
        if alpha_byte != self.last_alpha_byte {
            self.last_alpha_byte = alpha_byte;
            SetLayeredWindowAttributes(self.hwnd, 0, alpha_byte, LWA_ALPHA);
        }

        // Blit the 442 KB low-res buffer directly to the window DC using hardware nearest-neighbor StretchDIBits!
        let hdc = GetDC(self.hwnd);
        if !hdc.is_null() {
            SetStretchBltMode(hdc, COLORONCOLOR);
            StretchDIBits(
                hdc,
                0,
                0,
                self.screen_w,
                self.screen_h,
                0,
                0,
                bw as i32,
                bh as i32,
                self.pixel_buf.as_ptr() as *const _,
                &self.bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            ReleaseDC(self.hwnd, hdc);
        }

        // Ensure the screensaver window is shown, at the very top of `HWND_TOPMOST`, and the mouse cursor is hidden!
        if !self.visible {
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                self.mon_x,
                self.mon_y,
                self.screen_w,
                self.screen_h,
                SWP_NOACTIVATE,
            );
            self.visible = true;
        } else if !GetWindow(self.hwnd, GW_HWNDPREV).is_null() {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_NOOWNERZORDER,
            );
        }

        if !self.cursor_hidden {
            ShowCursor(0);
            SetCursor(ptr::null_mut());
            self.cursor_hidden = true;
        }

        if self.blackout_fade >= 0.999 && alpha_byte == 255 {
            self.blackout_frame_presented = true;
        }
    }
}

impl Drop for ScreensaverEngine {
    fn drop(&mut self) {
        unsafe {
            if self.cursor_hidden {
                ShowCursor(1);
                self.cursor_hidden = false;
            }
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
        }
    }
}

#[inline(always)]
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h6 = (h.fract() + 1.0).fract() * 6.0;
    let i = h6.floor() as i32;
    let f = h6 - (i as f32);
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}
