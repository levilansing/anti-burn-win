use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

fn default_true() -> bool {
    true
}

fn default_turn_off_mins() -> u32 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    /// Launch Anti-Burn OLED Guard automatically when Windows starts
    #[serde(default = "default_true")]
    pub run_at_startup: bool,

    /// Enable taskbar dimming
    pub taskbar_dim_enabled: bool,
    /// Taskbar brightness reduction when mouse is away (0.0 to 1.0, default 0.60 = 60%)
    pub taskbar_dim_amount: f32,
    /// Taskbar contrast reduction when mouse is away (0.0 to 1.0, default 0.15 = 15%)
    pub taskbar_contrast_reduction: f32,
    /// Distance in pixels from taskbar where fade-in begins (default 100 px)
    pub taskbar_proximity_px: i32,

    /// Enable dimming behind active window (Strategy A)
    pub inactive_dim_enabled: bool,
    /// Inactive window / background brightness reduction (0.0 to 1.0, default 0.45 = 45%)
    pub inactive_dim_amount: f32,
    /// Smoothly crossfade old active window out and new active window in
    pub active_window_fade_enabled: bool,
    /// Duration of active-to-inactive window crossfade in milliseconds (default 240 ms)
    pub active_window_fade_ms: u32,

    /// Enable global screen dimming when idle
    pub idle_dim_enabled: bool,
    /// Idle timeout before dimming the screen in seconds (default 180s = 3 minutes)
    pub idle_timeout_secs: u32,
    /// Global brightness reduction when idle (0.0 to 1.0, default 0.70 = 70%)
    pub idle_dim_amount: f32,
    /// Suppress idle dimming & screensaver when watching video or on a conference call
    pub pause_on_media_or_call: bool,

    /// Enable two-stage GPU screensaver (Particle Flow Fields + Drifting Metaballs)
    pub screensaver_enabled: bool,
    /// Idle timeout before starting Stage 1 (semi-transparent) screensaver in seconds (default 300s = 5 mins)
    pub screensaver_stage1_secs: u32,
    /// Background opacity during Stage 1 screensaver (default 0.70 = 30% background brightness)
    pub screensaver_stage1_opacity: f32,
    /// Idle timeout before fading to Stage 2 (100% opaque backdrop) in seconds (default 900s = 15 mins)
    pub screensaver_stage2_secs: u32,
    /// Fade duration from Stage 1 to Stage 2 opaque black in seconds (default 120s)
    pub screensaver_stage2_fade_secs: f32,
    /// Minutes of screensaver runtime before turning screen 100% pure black & putting display to sleep (0 = Never, increments of 15m, default 30m)
    #[serde(default = "default_turn_off_mins")]
    pub screensaver_turn_off_mins: u32,
    /// Visual brightness multiplier of metaballs & flow particles (0.1 to 1.0, default 0.65)
    pub screensaver_vibrancy: f32,
    /// Number of flow field particles coupled to metaballs (default 1800)
    pub screensaver_particle_count: u32,
    /// Number of drifting physics metaballs (default 7)
    pub screensaver_metaball_count: u32,
    /// Whether to render the metaball glow orbs in the background (if false, only flow particles are rendered)
    #[serde(default = "default_true")]
    pub screensaver_show_metaballs: bool,
    /// Whether metaballs are filled with solid bright color instead of having dark centers
    #[serde(default = "default_true")]
    pub screensaver_solid_metaballs: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            run_at_startup: true,
            taskbar_dim_enabled: true,
            taskbar_dim_amount: 0.60,
            taskbar_contrast_reduction: 0.15,
            taskbar_proximity_px: 100,

            inactive_dim_enabled: true,
            inactive_dim_amount: 0.45,
            active_window_fade_enabled: true,
            active_window_fade_ms: 240,

            idle_dim_enabled: true,
            idle_timeout_secs: 180, // 3 minutes
            idle_dim_amount: 0.70,  // 70% brightness reduction
            pause_on_media_or_call: true,

            screensaver_enabled: true,
            screensaver_stage1_secs: 300, // 5 minutes
            screensaver_stage1_opacity: 0.70,
            screensaver_stage2_secs: 900, // 15 minutes
            screensaver_stage2_fade_secs: 120.0,
            screensaver_turn_off_mins: 30, // 30 minutes (15-min increments)
            screensaver_vibrancy: 0.65,
            screensaver_particle_count: 1800,
            screensaver_metaball_count: 7,
            screensaver_show_metaballs: true,
            screensaver_solid_metaballs: true,
        }
    }
}

impl AppConfig {
    pub fn config_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Ok(appdata) = std::env::var("APPDATA") {
            let dir = PathBuf::from(appdata).join("AntiBurnOLED");
            let _ = fs::create_dir_all(&dir);
            paths.push(dir.join("settings.json"));
        }
        paths.push(PathBuf::from("settings.json"));
        paths
    }

    pub fn load() -> Self {
        for path in Self::config_paths() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(cfg) = serde_json::from_str::<AppConfig>(&content) {
                    crate::win_apis::sync_run_at_startup(cfg.run_at_startup);
                    return cfg;
                }
            }
        }
        let cfg = Self::default();
        cfg.save();
        cfg
    }

    pub fn save(&self) {
        crate::win_apis::sync_run_at_startup(self.run_at_startup);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            for path in Self::config_paths() {
                if fs::write(&path, &json).is_ok() {
                    break;
                }
            }
        }
    }
}
