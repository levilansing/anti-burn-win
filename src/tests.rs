use crate::config::AppConfig;
use crate::win_apis::{distance_to_rect, ShellAndMediaDetector, MAGCOLOREFFECT};
use winapi::shared::windef::{POINT, RECT};

#[test]
fn test_default_config_values() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.taskbar_dim_amount, 0.60);
    assert_eq!(cfg.taskbar_proximity_px, 100);
    assert_eq!(cfg.idle_timeout_secs, 180); // 3 minutes
    assert_eq!(cfg.idle_dim_amount, 0.70); // 70% reduction
    assert!(cfg.pause_on_media_or_call);
    assert!(cfg.active_window_fade_enabled);
}

#[test]
fn test_distance_to_rect() {
    let rect = RECT {
        left: 0,
        top: 1032,
        right: 1920,
        bottom: 1080,
    };
    // Inside taskbar
    assert_eq!(distance_to_rect(POINT { x: 500, y: 1050 }, rect), 0.0);
    // 50px above taskbar
    assert_eq!(distance_to_rect(POINT { x: 500, y: 982 }, rect), 50.0);
    // 100px above taskbar
    assert_eq!(distance_to_rect(POINT { x: 500, y: 932 }, rect), 100.0);
}

#[test]
fn test_mag_color_matrix() {
    let m = MAGCOLOREFFECT::brightness_and_contrast(0.30, 1.0);
    let expected = 0.30f32.powf(1.85);
    assert!((m.transform[0][0] - expected).abs() < 1e-5);
    assert!((m.transform[1][1] - expected).abs() < 1e-5);
    assert!((m.transform[2][2] - expected).abs() < 1e-5);
    assert!((m.transform[4][0]).abs() < 1e-5);
}

#[test]
fn test_capture_session_detector() {
    let mut detector = ShellAndMediaDetector::new();
    let _in_call = detector.is_watching_video_or_in_call(0.1);
    assert!(detector.last_capture_peak >= 0.0 && detector.last_capture_peak <= 1.0);
}
