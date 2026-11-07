use crate::config::AppConfig;
use crate::win_apis::to_wstring;
use std::ptr;
use winapi::shared::windef::{HDC, HWND, POINT, RECT};
use winapi::um::wingdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    SRCCOPY, TRANSPARENT,
};
use winapi::um::winuser::{DrawTextW, DT_CENTER, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    Toggle,
    Slider { min: i32, max: i32, step: i32 },
    Button,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ControlId {
    RunAtStartup,

    TaskbarEnabled,
    TaskbarDimPct,
    TaskbarProximityPx,

    InactiveEnabled,
    InactiveDimPct,
    ActiveFadeEnabled,
    ActiveFadeMs,

    IdleEnabled,
    IdleTimeoutMins,
    IdleDimPct,
    PauseOnMedia,

    ScreensaverEnabled,
    SsStage1Mins,
    SsStage1BgBrightnessPct,
    SsStage2Mins,
    SsVibrancyPct,
    SsShowMetaballs,
    SsSolidMetaballs,

    BtnPreviewIdleDim,
    BtnPreviewScreensaver,
    BtnMinimizeToTray,
}

pub struct NeoControl {
    pub id: ControlId,
    pub kind: ControlKind,
    pub label: &'static str,
    pub rect: RECT,
}

pub struct NeoUi {
    pub controls: Vec<NeoControl>,
    pub active_drag: Option<ControlId>,
    pub hovered: Option<ControlId>,
}

/// RGB color struct `[R, G, B]` in `0.0..=255.0`
#[derive(Clone, Copy)]
struct Rgb(f32, f32, f32);

impl Rgb {
    const fn from_hex(hex: u32) -> Self {
        Self(
            ((hex >> 16) & 0xFF) as f32,
            ((hex >> 8) & 0xFF) as f32,
            (hex & 0xFF) as f32,
        )
    }

    fn to_colorref(self) -> u32 {
        let r = self.0.round().clamp(0.0, 255.0) as u32;
        let g = self.1.round().clamp(0.0, 255.0) as u32;
        let b = self.2.round().clamp(0.0, 255.0) as u32;
        (b << 16) | (g << 8) | r
    }
}

impl NeoUi {
    pub fn new() -> Self {
        let mut controls = Vec::new();

        // Top Header Toggle: Launch at Windows Startup
        controls.push(NeoControl {
            id: ControlId::RunAtStartup,
            kind: ControlKind::Toggle,
            label: "Launch at Startup",
            rect: RECT { left: 392, top: 16, right: 572, bottom: 44 },
        });

        // Card 1: Taskbar & Start Menu Protection (y: 58..202)
        controls.push(NeoControl {
            id: ControlId::TaskbarEnabled,
            kind: ControlKind::Toggle,
            label: "Dim Taskbar (fades in on mouse intent or Start Menu)",
            rect: RECT { left: 40, top: 96, right: 560, bottom: 124 },
        });
        controls.push(NeoControl {
            id: ControlId::TaskbarDimPct,
            kind: ControlKind::Slider { min: 10, max: 90, step: 5 },
            label: "Brightness Reduction",
            rect: RECT { left: 40, top: 130, right: 560, bottom: 158 },
        });
        controls.push(NeoControl {
            id: ControlId::TaskbarProximityPx,
            kind: ControlKind::Slider { min: 20, max: 250, step: 10 },
            label: "Mouse Intent Zone",
            rect: RECT { left: 40, top: 164, right: 560, bottom: 192 },
        });

        // Card 2: Inactive Window Backdrop & Crossfade (y: 214..382)
        controls.push(NeoControl {
            id: ControlId::InactiveEnabled,
            kind: ControlKind::Toggle,
            label: "Dim Inactive Windows & Desktop Backdrop (Strategy A)",
            rect: RECT { left: 40, top: 244, right: 560, bottom: 272 },
        });
        controls.push(NeoControl {
            id: ControlId::InactiveDimPct,
            kind: ControlKind::Slider { min: 10, max: 85, step: 5 },
            label: "Backdrop Dimming",
            rect: RECT { left: 40, top: 278, right: 560, bottom: 306 },
        });
        controls.push(NeoControl {
            id: ControlId::ActiveFadeEnabled,
            kind: ControlKind::Toggle,
            label: "Precision Rounded Crossfade Between Active Windows",
            rect: RECT { left: 40, top: 312, right: 560, bottom: 340 },
        });
        controls.push(NeoControl {
            id: ControlId::ActiveFadeMs,
            kind: ControlKind::Slider { min: 80, max: 600, step: 20 },
            label: "Crossfade Duration",
            rect: RECT { left: 40, top: 346, right: 560, bottom: 374 },
        });

        // Card 3: Global Idle Dimming (y: 394..562)
        controls.push(NeoControl {
            id: ControlId::IdleEnabled,
            kind: ControlKind::Toggle,
            label: "Dim Entire Screen When Idle (HDR & SDR Compositor)",
            rect: RECT { left: 40, top: 424, right: 560, bottom: 452 },
        });
        controls.push(NeoControl {
            id: ControlId::IdleTimeoutMins,
            kind: ControlKind::Slider { min: 1, max: 20, step: 1 },
            label: "Idle Timeout",
            rect: RECT { left: 40, top: 458, right: 560, bottom: 486 },
        });
        controls.push(NeoControl {
            id: ControlId::IdleDimPct,
            kind: ControlKind::Slider { min: 15, max: 90, step: 5 },
            label: "Idle Brightness Drop",
            rect: RECT { left: 40, top: 492, right: 560, bottom: 520 },
        });
        controls.push(NeoControl {
            id: ControlId::PauseOnMedia,
            kind: ControlKind::Toggle,
            label: "Pause Idle Timer During Active Microphone Calls",
            rect: RECT { left: 40, top: 526, right: 560, bottom: 554 },
        });

        // Card 4: Coupled Metaball & Flow Field Screensaver (y: 574..774)
        controls.push(NeoControl {
            id: ControlId::ScreensaverEnabled,
            kind: ControlKind::Toggle,
            label: "Two-Stage Coupled Flow Field & Metaball Screensaver",
            rect: RECT { left: 40, top: 604, right: 560, bottom: 632 },
        });
        controls.push(NeoControl {
            id: ControlId::SsStage1Mins,
            kind: ControlKind::Slider { min: 1, max: 30, step: 1 },
            label: "Stage 1 Start Time",
            rect: RECT { left: 40, top: 638, right: 560, bottom: 666 },
        });
        controls.push(NeoControl {
            id: ControlId::SsStage1BgBrightnessPct,
            kind: ControlKind::Slider { min: 10, max: 60, step: 5 },
            label: "Initial Desktop Brightness",
            rect: RECT { left: 40, top: 672, right: 560, bottom: 700 },
        });
        controls.push(NeoControl {
            id: ControlId::SsStage2Mins,
            kind: ControlKind::Slider { min: 5, max: 60, step: 5 },
            label: "Full Blackout (#000000) At",
            rect: RECT { left: 40, top: 706, right: 560, bottom: 734 },
        });
        controls.push(NeoControl {
            id: ControlId::SsVibrancyPct,
            kind: ControlKind::Slider { min: 20, max: 100, step: 5 },
            label: "Pattern Color Vibrancy",
            rect: RECT { left: 40, top: 740, right: 560, bottom: 768 },
        });
        controls.push(NeoControl {
            id: ControlId::SsShowMetaballs,
            kind: ControlKind::Toggle,
            label: "Show Metaball Glow Background (Off = Particles Only)",
            rect: RECT { left: 40, top: 774, right: 560, bottom: 802 },
        });
        controls.push(NeoControl {
            id: ControlId::SsSolidMetaballs,
            kind: ControlKind::Toggle,
            label: "Solid Colored Metaballs (Instead of Dark Centers)",
            rect: RECT { left: 40, top: 808, right: 560, bottom: 836 },
        });

        // Bottom Neomorphic Action Buttons (y: 956..996)
        controls.push(NeoControl {
            id: ControlId::BtnPreviewIdleDim,
            kind: ControlKind::Button,
            label: "Test Idle Dim",
            rect: RECT { left: 24, top: 956, right: 204, bottom: 996 },
        });
        controls.push(NeoControl {
            id: ControlId::BtnPreviewScreensaver,
            kind: ControlKind::Button,
            label: "Test Screensaver",
            rect: RECT { left: 218, top: 956, right: 402, bottom: 996 },
        });
        controls.push(NeoControl {
            id: ControlId::BtnMinimizeToTray,
            kind: ControlKind::Button,
            label: "Hide to Tray",
            rect: RECT { left: 416, top: 956, right: 576, bottom: 996 },
        });

        Self {
            controls,
            active_drag: None,
            hovered: None,
        }
    }

    pub fn slider_track_rect(ctrl_rect: RECT) -> RECT {
        RECT {
            left: ctrl_rect.left + 215,
            top: ctrl_rect.top + 10,
            right: ctrl_rect.right - 78,
            bottom: ctrl_rect.top + 18,
        }
    }

    pub fn get_value(cfg: &AppConfig, id: ControlId) -> i32 {
        match id {
            ControlId::RunAtStartup => i32::from(cfg.run_at_startup),
            ControlId::TaskbarEnabled => i32::from(cfg.taskbar_dim_enabled),
            ControlId::TaskbarDimPct => (cfg.taskbar_dim_amount * 100.0).round() as i32,
            ControlId::TaskbarProximityPx => cfg.taskbar_proximity_px,
            ControlId::InactiveEnabled => i32::from(cfg.inactive_dim_enabled),
            ControlId::InactiveDimPct => (cfg.inactive_dim_amount * 100.0).round() as i32,
            ControlId::ActiveFadeEnabled => i32::from(cfg.active_window_fade_enabled),
            ControlId::ActiveFadeMs => cfg.active_window_fade_ms as i32,
            ControlId::IdleEnabled => i32::from(cfg.idle_dim_enabled),
            ControlId::IdleTimeoutMins => {
                ((cfg.idle_timeout_secs as f32) / 60.0).round().max(1.0) as i32
            }
            ControlId::IdleDimPct => (cfg.idle_dim_amount * 100.0).round() as i32,
            ControlId::PauseOnMedia => i32::from(cfg.pause_on_media_or_call),
            ControlId::ScreensaverEnabled => i32::from(cfg.screensaver_enabled),
            ControlId::SsStage1Mins => {
                ((cfg.screensaver_stage1_secs as f32) / 60.0).round().max(1.0) as i32
            }
            ControlId::SsStage1BgBrightnessPct => {
                ((1.0 - cfg.screensaver_stage1_opacity) * 100.0).round() as i32
            }
            ControlId::SsStage2Mins => {
                ((cfg.screensaver_stage2_secs as f32) / 60.0).round().max(5.0) as i32
            }
            ControlId::SsVibrancyPct => (cfg.screensaver_vibrancy * 100.0).round() as i32,
            ControlId::SsShowMetaballs => i32::from(cfg.screensaver_show_metaballs),
            ControlId::SsSolidMetaballs => i32::from(cfg.screensaver_solid_metaballs),
            _ => 0,
        }
    }

    pub fn format_value(id: ControlId, val: i32) -> String {
        match id {
            ControlId::TaskbarDimPct
            | ControlId::InactiveDimPct
            | ControlId::IdleDimPct
            | ControlId::SsStage1BgBrightnessPct
            | ControlId::SsVibrancyPct => format!("{}%", val),
            ControlId::TaskbarProximityPx => format!("{} px", val),
            ControlId::ActiveFadeMs => format!("{} ms", val),
            ControlId::IdleTimeoutMins | ControlId::SsStage1Mins | ControlId::SsStage2Mins => {
                format!("{} min", val)
            }
            _ => String::new(),
        }
    }

    pub fn set_value(cfg: &mut AppConfig, id: ControlId, val: i32) {
        match id {
            ControlId::RunAtStartup => cfg.run_at_startup = val != 0,
            ControlId::TaskbarEnabled => cfg.taskbar_dim_enabled = val != 0,
            ControlId::TaskbarDimPct => cfg.taskbar_dim_amount = (val as f32) / 100.0,
            ControlId::TaskbarProximityPx => cfg.taskbar_proximity_px = val,
            ControlId::InactiveEnabled => cfg.inactive_dim_enabled = val != 0,
            ControlId::InactiveDimPct => cfg.inactive_dim_amount = (val as f32) / 100.0,
            ControlId::ActiveFadeEnabled => cfg.active_window_fade_enabled = val != 0,
            ControlId::ActiveFadeMs => cfg.active_window_fade_ms = val as u32,
            ControlId::IdleEnabled => cfg.idle_dim_enabled = val != 0,
            ControlId::IdleTimeoutMins => cfg.idle_timeout_secs = (val.max(1) as u32) * 60,
            ControlId::IdleDimPct => cfg.idle_dim_amount = (val as f32) / 100.0,
            ControlId::PauseOnMedia => cfg.pause_on_media_or_call = val != 0,
            ControlId::ScreensaverEnabled => cfg.screensaver_enabled = val != 0,
            ControlId::SsStage1Mins => cfg.screensaver_stage1_secs = (val.max(1) as u32) * 60,
            ControlId::SsStage1BgBrightnessPct => {
                cfg.screensaver_stage1_opacity = (1.0 - (val as f32) / 100.0).clamp(0.10, 0.95)
            }
            ControlId::SsStage2Mins => cfg.screensaver_stage2_secs = (val.max(5) as u32) * 60,
            ControlId::SsVibrancyPct => cfg.screensaver_vibrancy = (val as f32) / 100.0,
            ControlId::SsShowMetaballs => cfg.screensaver_show_metaballs = val != 0,
            ControlId::SsSolidMetaballs => cfg.screensaver_solid_metaballs = val != 0,
            _ => {}
        }
    }

    pub fn on_mouse_down(&mut self, pt: POINT, cfg: &mut AppConfig) -> Option<ControlId> {
        for ctrl in &self.controls {
            if pt_in_rect(pt, ctrl.rect) {
                match ctrl.kind {
                    ControlKind::Toggle => {
                        let cur = Self::get_value(cfg, ctrl.id);
                        Self::set_value(cfg, ctrl.id, if cur == 0 { 1 } else { 0 });
                        cfg.save();
                        return Some(ctrl.id);
                    }
                    ControlKind::Slider { min, max, step } => {
                        self.active_drag = Some(ctrl.id);
                        Self::update_slider_from_x(ctrl, pt.x, min, max, step, cfg);
                        cfg.save();
                        return Some(ctrl.id);
                    }
                    ControlKind::Button => {
                        return Some(ctrl.id);
                    }
                }
            }
        }
        None
    }

    pub fn on_mouse_move(&mut self, pt: POINT, cfg: &mut AppConfig) -> bool {
        let mut changed = false;
        if let Some(drag_id) = self.active_drag {
            for ctrl in &self.controls {
                if ctrl.id == drag_id {
                    if let ControlKind::Slider { min, max, step } = ctrl.kind {
                        let old = Self::get_value(cfg, drag_id);
                        Self::update_slider_from_x(ctrl, pt.x, min, max, step, cfg);
                        if Self::get_value(cfg, drag_id) != old {
                            cfg.save();
                            changed = true;
                        }
                    }
                    break;
                }
            }
        }

        let mut new_hover = None;
        for ctrl in &self.controls {
            if pt_in_rect(pt, ctrl.rect) {
                new_hover = Some(ctrl.id);
                break;
            }
        }
        if new_hover != self.hovered {
            self.hovered = new_hover;
            changed = true;
        }
        changed
    }

    pub fn on_mouse_up(&mut self) -> bool {
        if self.active_drag.is_some() {
            self.active_drag = None;
            true
        } else {
            false
        }
    }

    fn update_slider_from_x(
        ctrl: &NeoControl,
        x: i32,
        min: i32,
        max: i32,
        step: i32,
        cfg: &mut AppConfig,
    ) {
        let track = Self::slider_track_rect(ctrl.rect);
        let w = (track.right - track.left).max(1) as f32;
        let t = (((x - track.left) as f32) / w).clamp(0.0, 1.0);
        let raw = (min as f32) + t * ((max - min) as f32);
        let snapped = (((raw - (min as f32)) / (step as f32)).round() as i32) * step + min;
        Self::set_value(cfg, ctrl.id, snapped.clamp(min, max));
    }

    /// True SDF-Anti-Aliased + ClearType-Rendered Neomorphic UI!
    /// Uses a 32-bit DIB section so every rounded card, soft shadow, pill toggle, and solid slider thumb
    /// is mathematically anti-aliased at sub-pixel precision, followed by native ClearType font rendering.
    pub unsafe fn paint(
        &self,
        hwnd: HWND,
        hdc: HDC,
        client_rect: RECT,
        cfg: &AppConfig,
        idle_state_summary: &str,
        last_wake_event: &str,
        prev_wake_event: &str,
    ) {
        let w = (client_rect.right - client_rect.left).max(1);
        let h = (client_rect.bottom - client_rect.top).max(1);

        let mem_dc = CreateCompatibleDC(hdc);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
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
        let mem_bmp = CreateDIBSection(
            mem_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            ptr::null_mut(),
            0,
        );
        let old_bmp = SelectObject(mem_dc, mem_bmp as *mut _);

        let col_bg = Rgb::from_hex(0x1B1F27);
        let col_card = Rgb::from_hex(0x232832);
        let col_card_hover = Rgb::from_hex(0x29303C);
        let col_shadow_dark = Rgb::from_hex(0x0E1015);
        let col_highlight = Rgb::from_hex(0x363E4D);
        let col_trough = Rgb::from_hex(0x14171D);
        let col_accent = Rgb::from_hex(0x38BDF8);
        let col_accent_bright = Rgb::from_hex(0x7DD3FC);
        let col_accent_sec = Rgb::from_hex(0xA78BFA);
        let col_text_main = Rgb::from_hex(0xF1F5F9);
        let col_text_muted = Rgb::from_hex(0x94A3B8);

        if !bits_ptr.is_null() {
            let pixels = std::slice::from_raw_parts_mut(bits_ptr as *mut u32, (w * h) as usize);

            // 1. Fill base neomorphic background
            let bg_u32 = pack_rgb(col_bg);
            pixels.fill(bg_u32);

            // 2. Draw 5 Anti-Aliased Neomorphic Cards with soft Gaussian-like ambient drop shadows + top-left bevel
            let cards = [
                RECT { left: 24, top: 58, right: 576, bottom: 202 },
                RECT { left: 24, top: 214, right: 576, bottom: 382 },
                RECT { left: 24, top: 394, right: 576, bottom: 562 },
                RECT { left: 24, top: 574, right: 576, bottom: 842 },
                RECT { left: 24, top: 854, right: 576, bottom: 944 },
            ];

            for crect in &cards {
                sdf_draw_neo_card(
                    pixels,
                    w,
                    h,
                    *crect,
                    14.0,
                    col_card,
                    col_highlight,
                    col_shadow_dark,
                );
            }

            // 3. Draw Anti-Aliased Toggle Switches, Solid-Cap Sliders, and Neomorphic Buttons
            for ctrl in &self.controls {
                let is_hovered =
                    self.hovered == Some(ctrl.id) || self.active_drag == Some(ctrl.id);
                match ctrl.kind {
                    ControlKind::Toggle => {
                        let on = Self::get_value(cfg, ctrl.id) != 0;
                        let pill = RECT {
                            left: ctrl.rect.right - 50,
                            top: ctrl.rect.top + 3,
                            right: ctrl.rect.right - 4,
                            bottom: ctrl.rect.bottom - 3,
                        };
                        sdf_draw_pill_toggle(
                            pixels,
                            w,
                            h,
                            pill,
                            on,
                            is_hovered,
                            col_trough,
                            col_accent,
                            col_accent_bright,
                            col_highlight,
                        );
                    }
                    ControlKind::Slider { min, max, .. } => {
                        let val = Self::get_value(cfg, ctrl.id);
                        let track = Self::slider_track_rect(ctrl.rect);
                        let frac =
                            (((val - min) as f32) / ((max - min).max(1) as f32)).clamp(0.0, 1.0);
                        sdf_draw_slider(
                            pixels,
                            w,
                            h,
                            track,
                            frac,
                            is_hovered,
                            col_trough,
                            col_accent,
                            col_accent_bright,
                        );
                    }
                    ControlKind::Button => {
                        let surf = if is_hovered { col_card_hover } else { col_card };
                        let border = if is_hovered { col_accent } else { col_highlight };
                        sdf_draw_neo_card(
                            pixels,
                            w,
                            h,
                            ctrl.rect,
                            10.0,
                            surf,
                            border,
                            col_shadow_dark,
                        );
                    }
                }
            }

            // 3b. Draw 28x28 custom Anti-Burn OLED Eclipse emblem in the top-left header (x: 24..52, y: 16..44)
            let icon_sz = 28usize;
            let icon_px = crate::icon::render_icon_bgra(icon_sz);
            for iy in 0..icon_sz {
                let dy = 16 + iy as i32;
                if dy >= 0 && dy < h {
                    for ix in 0..icon_sz {
                        let dx = 24 + ix as i32;
                        if dx >= 0 && dx < w {
                            let src = icon_px[iy * icon_sz + ix];
                            let a = ((src >> 24) & 0xFF) as f32 / 255.0;
                            if a > 0.001 {
                                let r = ((src >> 16) & 0xFF) as f32;
                                let g = ((src >> 8) & 0xFF) as f32;
                                let b = (src & 0xFF) as f32;
                                blend_pixel(pixels, (dy * w + dx) as usize, Rgb(r, g, b), a);
                            }
                        }
                    }
                }
            }
        }

        // 4. Render crisp ClearType text on top of the anti-aliased DIB buffer!
        SetBkMode(mem_dc, TRANSPARENT as i32);

        let font_title = create_cleartype_font(-19, 700);
        let font_sec = create_cleartype_font(-13, 700);
        let font_body = create_cleartype_font(-14, 500);
        let font_val = create_cleartype_font(-14, 700);

        SelectObject(mem_dc, font_title as *mut _);
        SetTextColor(mem_dc, col_text_main.to_colorref());
        draw_text(
            mem_dc,
            "ANTI-BURN OLED GUARD",
            RECT { left: 60, top: 16, right: 388, bottom: 44 },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        let card_titles = [
            (RECT { left: 24, top: 58, right: 576, bottom: 202 }, "TASKBAR & START MENU FILTER"),
            (RECT { left: 24, top: 214, right: 576, bottom: 382 }, "INACTIVE WINDOW BACKDROP & CROSSFADE"),
            (RECT { left: 24, top: 394, right: 576, bottom: 562 }, "GLOBAL IDLE DIMMING (HDR & SDR)"),
            (RECT { left: 24, top: 574, right: 576, bottom: 842 }, "COUPLED FLOW FIELD & METABALL SCREENSAVER"),
            (RECT { left: 24, top: 854, right: 576, bottom: 944 }, "LIVE IDLE & WAKEUP DIAGNOSTICS (idle_wake.log)"),
        ];

        SelectObject(mem_dc, font_sec as *mut _);
        SetTextColor(mem_dc, col_accent_sec.to_colorref());
        for (crect, title) in &card_titles {
            draw_text(
                mem_dc,
                title,
                RECT {
                    left: crect.left + 16,
                    top: crect.top + 8,
                    right: crect.right - 16,
                    bottom: crect.top + 28,
                },
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
        }

        // Render Live Idle & Wakeup Diagnostics lines inside Card 5
        let font_diag = create_cleartype_font(-12, 600);
        SelectObject(mem_dc, font_diag as *mut _);
        SetTextColor(mem_dc, col_accent_bright.to_colorref());
        draw_text(
            mem_dc,
            idle_state_summary,
            RECT { left: 40, top: 880, right: 560, bottom: 900 },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        SetTextColor(mem_dc, col_text_main.to_colorref());
        draw_text(
            mem_dc,
            &format!("Last Wake: {}", last_wake_event),
            RECT { left: 40, top: 900, right: 560, bottom: 920 },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        SetTextColor(mem_dc, col_text_muted.to_colorref());
        draw_text(
            mem_dc,
            &format!("Prev Event: {}", prev_wake_event),
            RECT { left: 40, top: 920, right: 560, bottom: 938 },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        DeleteObject(font_diag as *mut _);

        for ctrl in &self.controls {
            let is_hovered = self.hovered == Some(ctrl.id) || self.active_drag == Some(ctrl.id);
            match ctrl.kind {
                ControlKind::Toggle => {
                    let on = Self::get_value(cfg, ctrl.id) != 0;
                    SelectObject(mem_dc, font_body as *mut _);
                    SetTextColor(
                        mem_dc,
                        if on { col_text_main } else { col_text_muted }.to_colorref(),
                    );
                    draw_text(
                        mem_dc,
                        ctrl.label,
                        RECT {
                            left: ctrl.rect.left,
                            top: ctrl.rect.top,
                            right: ctrl.rect.right - 64,
                            bottom: ctrl.rect.bottom,
                        },
                        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                    );
                }
                ControlKind::Slider { .. } => {
                    let val = Self::get_value(cfg, ctrl.id);
                    SelectObject(mem_dc, font_body as *mut _);
                    SetTextColor(mem_dc, col_text_muted.to_colorref());
                    draw_text(
                        mem_dc,
                        ctrl.label,
                        RECT {
                            left: ctrl.rect.left,
                            top: ctrl.rect.top,
                            right: ctrl.rect.left + 208,
                            bottom: ctrl.rect.bottom,
                        },
                        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                    );

                    let track = Self::slider_track_rect(ctrl.rect);
                    SelectObject(mem_dc, font_val as *mut _);
                    SetTextColor(mem_dc, col_text_main.to_colorref());
                    let val_str = Self::format_value(ctrl.id, val);
                    draw_text(
                        mem_dc,
                        &val_str,
                        RECT {
                            left: track.right + 10,
                            top: ctrl.rect.top,
                            right: ctrl.rect.right - 4,
                            bottom: ctrl.rect.bottom,
                        },
                        DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
                    );
                }
                ControlKind::Button => {
                    SelectObject(mem_dc, font_val as *mut _);
                    SetTextColor(
                        mem_dc,
                        if is_hovered { col_accent_bright } else { col_text_main }.to_colorref(),
                    );
                    draw_text(
                        mem_dc,
                        ctrl.label,
                        ctrl.rect,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                    );
                }
            }
        }

        // Single flicker-free BitBlt to window DC
        BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);

        DeleteObject(font_title as *mut _);
        DeleteObject(font_sec as *mut _);
        DeleteObject(font_body as *mut _);
        DeleteObject(font_val as *mut _);
        SelectObject(mem_dc, old_bmp);
        DeleteObject(mem_bmp as *mut _);
        DeleteDC(mem_dc);
        let _ = hwnd;
    }
}

fn pt_in_rect(pt: POINT, r: RECT) -> bool {
    pt.x >= r.left && pt.x <= r.right && pt.y >= r.top && pt.y <= r.bottom
}

fn pack_rgb(c: Rgb) -> u32 {
    let r = c.0.round().clamp(0.0, 255.0) as u32;
    let g = c.1.round().clamp(0.0, 255.0) as u32;
    let b = c.2.round().clamp(0.0, 255.0) as u32;
    0xFF000000 | (r << 16) | (g << 8) | b
}

fn blend_pixel(pixels: &mut [u32], idx: usize, src: Rgb, alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    if a <= 0.001 {
        return;
    }
    let dst = pixels[idx];
    let dr = ((dst >> 16) & 0xFF) as f32;
    let dg = ((dst >> 8) & 0xFF) as f32;
    let db = (dst & 0xFF) as f32;

    let nr = dr + (src.0 - dr) * a;
    let ng = dg + (src.1 - dg) * a;
    let nb = db + (src.2 - db) * a;
    pixels[idx] = 0xFF000000 | ((nr as u32) << 16) | ((ng as u32) << 8) | (nb as u32);
}

/// Signed distance to a rounded rectangle centered at `(cx, cy)` with half-size `(hx, hy)` and corner radius `r`.
fn sd_rounded_box(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hx - r);
    let qy = (py - cy).abs() - (hy - r);
    let out_dist = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let in_dist = qx.max(qy).min(0.0);
    out_dist + in_dist - r
}

/// Renders an anti-aliased Neomorphic card with soft drop shadow and 1px bevel highlight
#[allow(clippy::too_many_arguments)]
fn sdf_draw_neo_card(
    pixels: &mut [u32],
    buf_w: i32,
    buf_h: i32,
    rect: RECT,
    radius: f32,
    surface: Rgb,
    bevel_light: Rgb,
    shadow_dark: Rgb,
) {
    let cx = (rect.left + rect.right) as f32 * 0.5;
    let cy = (rect.top + rect.bottom) as f32 * 0.5;
    let hx = (rect.right - rect.left) as f32 * 0.5;
    let hy = (rect.bottom - rect.top) as f32 * 0.5;

    let x0 = (rect.left - 8).clamp(0, buf_w);
    let y0 = (rect.top - 8).clamp(0, buf_h);
    let x1 = (rect.right + 10).clamp(0, buf_w);
    let y1 = (rect.bottom + 10).clamp(0, buf_h);

    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let row = (y * buf_w) as usize;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let idx = row + (x as usize);

            // 1. Soft bottom-right ambient shadow offset (+3.0, +3.0) with 6px feather
            let d_shad = sd_rounded_box(px, py, cx + 3.0, cy + 3.0, hx, hy, radius);
            if d_shad < 6.0 {
                let shad_a = ((1.0 - (d_shad / 6.0)).clamp(0.0, 1.0)).powi(2) * 0.55;
                blend_pixel(pixels, idx, shadow_dark, shad_a);
            }

            // 2. Main card body with sub-pixel anti-aliased edge
            let d = sd_rounded_box(px, py, cx, cy, hx, hy, radius);
            if d < 0.75 {
                let fill_cov = (0.5 - d).clamp(0.0, 1.0);
                blend_pixel(pixels, idx, surface, fill_cov);

                // 1.2px anti-aliased border ring
                let border_cov = (1.1 - d.abs()).clamp(0.0, 1.0);
                if border_cov > 0.0 {
                    blend_pixel(pixels, idx, bevel_light, border_cov * 0.65);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sdf_draw_pill_toggle(
    pixels: &mut [u32],
    buf_w: i32,
    buf_h: i32,
    rect: RECT,
    on: bool,
    hovered: bool,
    trough: Rgb,
    accent: Rgb,
    accent_bright: Rgb,
    bevel: Rgb,
) {
    let cx = (rect.left + rect.right) as f32 * 0.5;
    let cy = (rect.top + rect.bottom) as f32 * 0.5;
    let hx = (rect.right - rect.left) as f32 * 0.5;
    let hy = (rect.bottom - rect.top) as f32 * 0.5;
    let r = hy;

    let x0 = (rect.left - 4).clamp(0, buf_w);
    let y0 = (rect.top - 4).clamp(0, buf_h);
    let x1 = (rect.right + 4).clamp(0, buf_w);
    let y1 = (rect.bottom + 4).clamp(0, buf_h);

    let track_col = if on {
        Rgb(accent.0 * 0.32, accent.1 * 0.32, accent.2 * 0.35)
    } else {
        trough
    };
    let border_col = if on || hovered { accent } else { bevel };

    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let row = (y * buf_w) as usize;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let idx = row + (x as usize);

            let d = sd_rounded_box(px, py, cx, cy, hx, hy, r);
            if d < 0.75 {
                let cov = (0.5 - d).clamp(0.0, 1.0);
                blend_pixel(pixels, idx, track_col, cov);
                let b_cov = (1.0 - d.abs()).clamp(0.0, 1.0);
                blend_pixel(pixels, idx, border_col, b_cov * 0.8);
            }
        }
    }

    // Solid anti-aliased thumb circle (expands slightly when hovered!)
    let knob_r = if hovered { hy - 1.8 } else { hy - 3.0 };
    let knob_cx = if on {
        (rect.right as f32) - hy
    } else {
        (rect.left as f32) + hy
    };
    let knob_col = if on {
        if hovered { accent_bright } else { accent }
    } else if hovered {
        Rgb::from_hex(0xCBD5E1)
    } else {
        Rgb::from_hex(0x94A3B8)
    };

    sdf_draw_solid_circle(pixels, buf_w, buf_h, knob_cx, cy, knob_r, knob_col);
}

#[allow(clippy::too_many_arguments)]
fn sdf_draw_slider(
    pixels: &mut [u32],
    buf_w: i32,
    buf_h: i32,
    track: RECT,
    frac: f32,
    hovered: bool,
    trough: Rgb,
    accent: Rgb,
    accent_bright: Rgb,
) {
    let cx = (track.left + track.right) as f32 * 0.5;
    let cy = (track.top + track.bottom) as f32 * 0.5;
    let hx = (track.right - track.left) as f32 * 0.5;
    let hy = (track.bottom - track.top) as f32 * 0.5;
    let r = hy;

    let x0 = (track.left - 16).clamp(0, buf_w);
    let y0 = (track.top - 12).clamp(0, buf_h);
    let x1 = (track.right + 16).clamp(0, buf_w);
    let y1 = (track.bottom + 12).clamp(0, buf_h);

    // 1. Recessed Trough
    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let row = (y * buf_w) as usize;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let d = sd_rounded_box(px, py, cx, cy, hx, hy, r);
            if d < 0.75 {
                let cov = (0.5 - d).clamp(0.0, 1.0);
                blend_pixel(pixels, row + (x as usize), trough, cov);
            }
        }
    }

    // 2. Active Filled Bar
    let thumb_x = (track.left as f32) + ((track.right - track.left) as f32) * frac;
    let fill_hx = ((thumb_x - (track.left as f32)) * 0.5).max(r);
    let fill_cx = (track.left as f32) + fill_hx;

    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let row = (y * buf_w) as usize;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let d = sd_rounded_box(px, py, fill_cx, cy, fill_hx, hy, r);
            if d < 0.75 {
                let cov = (0.5 - d).clamp(0.0, 1.0);
                blend_pixel(pixels, row + (x as usize), accent, cov);
            }
        }
    }

    // 3. Solid Bright Cap / Thumb (NO dark center hole! Solid vibrant cap that grows smoothly on hover/drag!)
    let thumb_r = if hovered { 9.5 } else { 7.5 };
    let cap_col = if hovered { accent_bright } else { accent };
    // Subtle soft glow halo when hovered
    if hovered {
        sdf_draw_glow_halo(pixels, buf_w, buf_h, thumb_x, cy, thumb_r + 4.5, accent);
    }
    sdf_draw_solid_circle(pixels, buf_w, buf_h, thumb_x, cy, thumb_r, cap_col);
}

fn sdf_draw_solid_circle(
    pixels: &mut [u32],
    buf_w: i32,
    buf_h: i32,
    cx: f32,
    cy: f32,
    radius: f32,
    col: Rgb,
) {
    let x0 = ((cx - radius - 2.0).floor() as i32).clamp(0, buf_w);
    let y0 = ((cy - radius - 2.0).floor() as i32).clamp(0, buf_h);
    let x1 = ((cx + radius + 2.0).ceil() as i32).clamp(0, buf_w);
    let y1 = ((cy + radius + 2.0).ceil() as i32).clamp(0, buf_h);

    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let row = (y * buf_w) as usize;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let dist = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            let d = dist - radius;
            if d < 0.75 {
                let cov = (0.5 - d).clamp(0.0, 1.0);
                blend_pixel(pixels, row + (x as usize), col, cov);
            }
        }
    }
}

fn sdf_draw_glow_halo(
    pixels: &mut [u32],
    buf_w: i32,
    buf_h: i32,
    cx: f32,
    cy: f32,
    max_r: f32,
    col: Rgb,
) {
    let x0 = ((cx - max_r - 1.0).floor() as i32).clamp(0, buf_w);
    let y0 = ((cy - max_r - 1.0).floor() as i32).clamp(0, buf_h);
    let x1 = ((cx + max_r + 1.0).ceil() as i32).clamp(0, buf_w);
    let y1 = ((cy + max_r + 1.0).ceil() as i32).clamp(0, buf_h);

    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let row = (y * buf_w) as usize;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let dist = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            if dist < max_r {
                let t = (1.0 - dist / max_r).clamp(0.0, 1.0);
                blend_pixel(pixels, row + (x as usize), col, t * t * 0.28);
            }
        }
    }
}

unsafe fn create_cleartype_font(height: i32, weight: i32) -> winapi::shared::windef::HFONT {
    let name = to_wstring("Segoe UI");
    CreateFontW(
        height,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        0,
        0,
        0,
        5, // CLEARTYPE_QUALITY (5) + CLEARTYPE_NATURAL_QUALITY (6)
        0,
        name.as_ptr(),
    )
}

unsafe fn draw_text(hdc: HDC, text: &str, mut rect: RECT, flags: u32) {
    let w = to_wstring(text);
    DrawTextW(hdc, w.as_ptr(), (w.len() as i32) - 1, &mut rect, flags);
}
