//! The offer to calibrate a 3D mouse that has no calibration yet: people don't know calibration
//! exists, and it is what fixes swapped or reversed axes. It shows once per device, as a notice
//! small card above the History bar, until the user calibrates or closes it.

use std::collections::BTreeSet;

use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit;

/// Whether to offer calibration for `device`: 3D mouse navigation is on, there is a device, it
/// has no calibration saved, and the offer has not been made (and dismissed) for it. The 3Dconnexion
/// driver (macOS, key `3dxware`) already applies its own settings, so calibration isn't offered
/// for it.
pub fn should_offer(device: Option<&str>, calibrated: bool, offered: &BTreeSet<String>, enabled: bool) -> bool {
    let Some(device) = device else { return false };
    enabled && !calibrated && !offered.contains(device) && !device.starts_with("3dxware")
}

/// How long the notice stays when nobody is pointing at it: long enough to read it, find the
/// mouse and decide, which 15 seconds was too short for.
const SHOW_SECONDS: f64 = 30.0;

/// Whether the notice, first shown or last pointed at `since`, has been up long enough at `now`.
pub fn expired(since: f64, now: f64) -> bool {
    now - since >= SHOW_SECONDS
}

/// Draws the offer as a small card in the bottom right of `free` (the part of the view the
/// overlays leave, so above the History bar), and takes the room it uses off `free`. It is due
/// when `should_offer` says so, the window has focus, no picker or load has the app, and neither
/// Preferences nor the calibration window is open. It goes after `SHOW_SECONDS` unless the pointer
/// is on it (then only for this run: it is offered again next time until closed or calibrated).
pub fn toast(app: &mut SolveApp, ui: &egui::Ui, free: &mut egui::Rect) {
    let ctx = ui.ctx();
    let calibrated = app.space_mouse.device.as_ref().is_some_and(|device| app.preferences.space_mouse_calibrations.contains_key(device));
    let Some(device) = app.space_mouse.device.clone().filter(|device| {
        should_offer(Some(device), calibrated, &app.preferences.space_mouse_offered, app.preferences.space_mouse_enabled)
            && !app.space_mouse.offer_hidden.contains(device)
    }) else {
        return;
    };
    if !ctx.input(|i| i.focused)
        || app.file_picker_busy()
        || app.file_load.busy()
        || app.prefs_window.open
        || app.space_mouse.calibrating.is_some()
        || !free.is_finite()
        || free.width() <= 120.0
    {
        return;
    }
    let (t, now) = (Tokens::get(), ctx.input(|i| i.time));
    let since = match &app.space_mouse.offer_since {
        Some((shown, since)) if *shown == device => *since,
        _ => now,
    };
    app.space_mouse.offer_since = Some((device.clone(), since));
    if expired(since, now) {
        app.space_mouse.offer_hidden.insert(device);
        return;
    }
    let (mut calibrate, mut close) = (false, false);
    let width = (free.width() - 2.0 * t.space_8).clamp(120.0, 380.0);
    let shown = egui::Area::new(egui::Id::new("sc_space_mouse_offer"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(free.right_bottom() - egui::vec2(t.space_8, t.space_8))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.dialog_body)
                .stroke(egui::Stroke::new(1.0, t.dialog_rule))
                .corner_radius(t.radius_lg)
                .inner_margin(t.space_12)
                .show(ui, |ui| {
                    ui.set_width(width - 2.0 * t.space_12);
                    ui_kit::text(ui, app.tr("ui.space_mouse_offer"), ui_kit::Text::Report("space_mouse_offer", false));
                    ui.horizontal(|ui| {
                        calibrate = ui_kit::button(ui, app.tr("ui.space_mouse_calibrate"), ui_kit::Button::Normal).clicked();
                        close = ui_kit::button(ui, app.tr("ui.close"), ui_kit::Button::Normal).clicked();
                    });
                });
        });
    let rect = shown.response.rect;
    // Pointing at it keeps it.
    if ctx.input(|i| i.pointer.hover_pos()).is_some_and(|pos| rect.contains(pos)) {
        app.space_mouse.offer_since = app.space_mouse.offer_since.take().map(|(device, _)| (device, now));
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
    crate::scenario::publish_rect("toast:space_mouse_offer", rect);
    free.max.y = rect.top() - t.space_8;
    if (calibrate || close)
        && let Some((device, _)) = app.space_mouse.offer_since.take()
    {
        app.preferences.space_mouse_offered.insert(device);
    }
    if calibrate {
        // The calibration window lives with Preferences, at Navigation.
        crate::prefs::open_at(app, 1);
        crate::space_mouse_calibration::start(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offer_is_made_once_to_an_uncalibrated_device_with_navigation_on() {
        let none = BTreeSet::new();
        let seen: BTreeSet<String> = ["hid 046d:c62b".to_string()].into();
        for device in [Some("hid 046d:c62b"), Some("spacenavd"), Some("web 256f:c63a"), None] {
            for calibrated in [false, true] {
                for offered in [&none, &seen] {
                    for enabled in [false, true] {
                        let expected = device.is_some() && !calibrated && enabled && !device.is_some_and(|d| offered.contains(d));
                        assert_eq!(should_offer(device, calibrated, offered, enabled), expected, "{device:?} {calibrated} {offered:?} {enabled}");
                    }
                }
            }
        }
        // The driver on macOS has its own settings; there is nothing to calibrate.
        assert!(!should_offer(Some("3dxware"), false, &none, true));
    }

    fn press(h: &mut crate::scenario::Harness, widget: &str) {
        h.frames(3);
        h.step(&serde_json::json!({"widget": widget})).unwrap_or_else(|e| panic!("{widget}: {e}"));
        h.frames(2);
    }

    fn offered() -> crate::scenario::Harness {
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        h.app.space_mouse.device = Some("hid 046d:c62b".into());
        h.frames(3);
        h
    }

    #[test]
    fn the_notice_goes_after_thirty_seconds() {
        assert!(!expired(100.0, 129.9) && expired(100.0, 130.0));
    }

    #[test]
    fn closing_the_offer_records_the_device_so_it_does_not_come_back() {
        let mut h = offered();
        assert!(crate::scenario::rect_of("toast:space_mouse_offer").is_some(), "the notice is drawn");
        press(&mut h, "Close");
        assert!(h.app.preferences.space_mouse_offered.contains("hid 046d:c62b"));
        assert!(h.app.space_mouse.calibrating.is_none());
        h.frames(3);
        assert!(h.app.preferences.space_mouse_offered.contains("hid 046d:c62b"), "gone, and it stays gone");
    }

    #[test]
    fn calibrating_from_the_offer_opens_the_calibration_and_records_the_device() {
        let mut h = offered();
        press(&mut h, "Calibrate…");
        h.frames(3);
        assert!(h.app.space_mouse.calibrating.is_some() && h.app.prefs_window.open);
        assert!(h.app.preferences.space_mouse_offered.contains("hid 046d:c62b"));
    }

    #[test]
    fn it_stays_away_from_a_calibrated_device_and_navigation_off() {
        let axes =
            std::array::from_fn(|source| solidcraft_space_mouse::AxisCalibration { source, reversed: false, rest: 0, dead_zone: 0, full: [350; 2] });
        let calibration = solidcraft_space_mouse::Calibration { dead_zone_share: 0.0, kind: solidcraft_space_mouse::Kind::ReturnToCenter, axes };
        for (enabled, calibrated) in [(false, false), (true, true)] {
            let mut h = crate::scenario::Harness::new();
            h.app.home.open = false;
            h.app.space_mouse.device = Some("hid 046d:c62b".into());
            h.app.preferences.space_mouse_enabled = enabled;
            if calibrated {
                h.app.preferences.space_mouse_calibrations.insert("hid 046d:c62b".into(), calibration);
            }
            h.frames(4);
            assert!(h.app.space_mouse.offer_since.is_none(), "no notice for enabled {enabled}, calibrated {calibrated}");
        }
    }
}

#[cfg(test)]
mod preferences_tests {
    /// File, then Preferences…, by clicks: the window must be open, drawn and on top (nothing of ours over it), for settings like a real
    /// user with a 3D mouse (saved calibration, driver choice, picture in the view).
    #[test]
    fn preferences_open_and_draw_with_a_3d_mouse_users_settings() {
        // An uncalibrated device (the notice card is up first), the calibrated one and none.
        for device in [None, Some("hid 256f:c63a"), Some("spacenavd"), Some("hid 046d:c62b")] {
            let mut h = crate::scenario::Harness::new();
            h.app.home.open = false;
            h.app.load_prefs(include_str!("test_data/prefs_space_mouse_user.json"));
            h.app.space_mouse.device = device.map(Into::into);
            h.frames(4);
            // The real way in: File menu, then its Preferences item, by clicks.
            h.app.custom_titlebar = true;
            h.frames(3);
            h.step(&serde_json::json!({"click": {"handle": "appbar:file"}})).unwrap();
            h.frames(3);
            h.step(&serde_json::json!({"widget": "Preferences…"})).unwrap_or_else(|e| panic!("{device:?}: {e}"));
            h.frames(4);
            let window = crate::scenario::rect_of("dialog:window");
            let centre = window.map(|r| r.center());
            let top = centre.and_then(|c| h.ctx.layer_id_at(c));
            assert!(top.is_some_and(|layer| layer.id == egui::Id::new("sc_prefs")), "{device:?}: the layer on top at the window centre is {top:?}");
            assert!(h.app.prefs_window.open, "{device:?}: still open");
            assert!(crate::scenario::rect_of("dialog:window").is_some_and(|r| r.is_positive()), "{device:?}: the window is drawn");
        }
    }

    /// Preferences stays open while the calibration notice comes and goes around it.
    #[test]
    fn the_notice_never_closes_preferences() {
        for (open_first, frames_before) in [(false, 0), (false, 40), (true, 0), (true, 90)] {
            let mut h = crate::scenario::Harness::new();
            h.app.home.open = false;
            h.app.custom_titlebar = true;
            h.app.space_mouse.device = Some("hid 046d:c62b".into());
            h.frames(3);
            if open_first {
                crate::prefs::open_at(&mut h.app, 1);
            }
            for _ in 0..frames_before {
                h.frame();
            }
            if !open_first && frames_before > 0 {
                assert!(h.app.space_mouse.offer_since.is_some(), "the notice is up before Preferences opens");
            }
            if !open_first {
                h.step(&serde_json::json!({"click": {"handle": "appbar:file"}})).unwrap();
                h.frames(3);
                h.step(&serde_json::json!({"widget": "Preferences…"})).unwrap();
            }
            for frame in 0..200 {
                h.frame();
                assert!(h.app.prefs_window.open, "open_first {open_first}, before {frames_before}: closed at frame {frame}");
                let window = crate::scenario::rect_of("dialog:window").expect("drawn");
                // The window sizes itself over its first frames.
                if frame < 4 {
                    continue;
                }
                for f in [0.1_f32, 0.5, 0.9] {
                    for g in [0.1_f32, 0.5, 0.9] {
                        let at = window.min + egui::vec2(window.width() * f, window.height() * g);
                        let top = h.ctx.layer_id_at(at);
                        let order = h.ctx.memory(|m| m.areas().visible_layer_ids());
                        assert!(top.is_some_and(|l| l.id == egui::Id::new("sc_prefs")), "frame {frame} at {at:?}: top is {top:?}; order {order:?}");
                    }
                }
            }
        }
    }

    /// The genuine first run: default settings, the start page up, a 3D mouse that connects, then
    /// File, Preferences… by clicks. It stays open, on top, for 240 frames, with and without a
    /// device (so with and without the calibration notice).
    #[test]
    fn a_fresh_profile_opens_preferences_from_the_start_page_and_keeps_it() {
        for device in [None, Some("hid 046d:c62b"), Some("spacenavd")] {
            let mut h = crate::scenario::Harness::new();
            h.app.custom_titlebar = true;
            h.app.home.open = true;
            h.frames(3);
            h.app.space_mouse.device = device.map(Into::into);
            h.frames(30);
            h.step(&serde_json::json!({"click": {"handle": "appbar:file"}})).unwrap();
            h.frames(3);
            h.step(&serde_json::json!({"widget": "Preferences…"})).unwrap_or_else(|e| panic!("{device:?}: {e}"));
            for frame in 0..240 {
                h.frame();
                assert!(h.app.prefs_window.open, "{device:?}: closed at frame {frame}");
                if frame > 4 {
                    let window = crate::scenario::rect_of("dialog:window").expect("drawn");
                    let top = h.ctx.layer_id_at(window.center());
                    assert!(top.is_some_and(|l| l.id == egui::Id::new("sc_prefs")), "{device:?} frame {frame}: top is {top:?}");
                }
            }
        }
    }
}
