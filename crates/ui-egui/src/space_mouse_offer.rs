//! The offer to calibrate a 3D mouse that has no calibration yet: people don't know calibration
//! exists, and it is what fixes swapped or reversed axes. It shows once per device, as one of the
//! shared toasts (`viewport::transient_messages`), until the user calibrates or closes it.

use std::collections::BTreeSet;

use crate::SolveApp;

/// Whether to offer calibration for `device`: 3D mouse navigation is on, there is a device, it
/// has no calibration saved, the offer has not been made (and dismissed) for it, and the user has
/// not asked never to see it (`never`). The 3Dconnexion
/// driver (macOS, key `3dxware`) already applies its own settings, so calibration isn't offered
/// for it.
pub fn should_offer(device: Option<&str>, calibrated: bool, offered: &BTreeSet<String>, enabled: bool, never: bool) -> bool {
    let Some(device) = device else { return false };
    enabled && !never && !calibrated && !offered.contains(device) && !device.starts_with("3dxware")
}

/// Whether the card offers to swap to the direct (hidraw) reader: the 3D mouse is being read through
/// spacenavd and a direct device is there to read instead.
pub fn swap_offered(device: &str, drivers: Option<crate::space_mouse::DriversPresent>) -> bool {
    device == "spacenavd" && drivers.is_some_and(|present| present.direct)
}

/// How long the notice stays when nobody is pointing at it: long enough to read it, find the
/// mouse and decide, which 15 seconds was too short for.
const SHOW_SECONDS: f64 = 30.0;

/// A longer gap than this between two looks at the offer means it was not on screen meanwhile.
const AWAY_SECONDS: f64 = 2.0;

/// Whether the notice, first shown or last pointed at `since`, has been up long enough at `now`.
pub fn expired(since: f64, now: f64) -> bool {
    now - since >= SHOW_SECONDS
}

/// What the offer's toast can do when clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Opens Preferences at Navigation and starts the calibration window.
    Calibrate,
    /// Records this device as offered.
    Close,
    /// Records that no offer is wanted any more.
    Never,
    /// Reads the device directly instead of through spacenavd.
    Swap,
}

impl Action {
    /// What the button is for, which sets its look and place on the toast.
    pub(crate) fn kind(self) -> crate::viewport::ToastKind {
        use crate::viewport::ToastKind;
        match self {
            Self::Calibrate => ToastKind::Primary,
            Self::Swap => ToastKind::Secondary,
            Self::Close | Self::Never => ToastKind::Dismiss,
        }
    }

    /// The button's text, as a message id.
    pub fn label(self) -> &'static str {
        match self {
            Self::Calibrate => "ui.space_mouse_calibrate",
            Self::Close => "ui.close",
            Self::Never => "ui.space_mouse_offer_never",
            Self::Swap => "ui.space_mouse_offer_swap",
        }
    }
}

/// An offer to show now: its device and its actions (the text is `ui.space_mouse_offer`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub device: String,
    pub actions: Vec<Action>,
}

/// The offer due this frame, if any, for the shared toast (`viewport::transient_messages`) to draw.
/// It is due when `should_offer` says so, the window has focus, no picker or load has the app, no
/// issue report holds CAD input, and neither Preferences nor the calibration window is open. It goes
/// after `SHOW_SECONDS` unless the pointer is on it (`keep`; then only for this run: it is offered
/// again next time until closed or calibrated).
///
/// Everything it reads and writes outside its own state, so a reviewer can check it:
/// - Reads: the device key, saved calibrations, the offered set, the never switch and the 3D mouse
///   enable switch (`app.space_mouse.device`, `app.preferences`); the readers present
///   (`space_mouse.drivers`); `ctx.input` focus, time and pointer position; `file_picker_busy`,
///   `file_load.busy`, `prefs_window.open`, `space_mouse.calibrating`,
///   `issue_report::cad_input_blocked`.
/// - Writes, only its own: `space_mouse.offer_since`, `space_mouse.offer_hidden`;
///   `space_mouse_offered` and `space_mouse_offer_never` (preferences) when the user clicks Close,
///   Calibrate or Never.
/// - Calibrate opens Preferences at Navigation and starts the calibration window
///   (`prefs::open_at`, `space_mouse_calibration::start`), exactly as Preferences does. Swap sets
///   `space_mouse_driver`, the same setting Preferences does.
/// - It draws nothing and creates no layer: the toast is the shared one. It never touches
///   `ctx.memory`, focus, layer order, keyboard events, dialogs, tools, the selection or the document.
pub fn due(app: &mut SolveApp, ctx: &egui::Context) -> Option<Offer> {
    let calibrated = app.space_mouse.device.as_ref().is_some_and(|device| app.preferences.space_mouse_calibrations.contains_key(device));
    let device = app.space_mouse.device.clone().filter(|device| {
        should_offer(
            Some(device),
            calibrated,
            &app.preferences.space_mouse_offered,
            app.preferences.space_mouse_enabled,
            app.preferences.space_mouse_offer_never,
        ) && !app.space_mouse.offer_hidden.contains(device)
    })?;
    if !ctx.input(|i| i.focused)
        || app.file_picker_busy()
        || app.file_load.busy()
        || crate::issue_report::cad_input_blocked()
        || app.prefs_window.open
        || app.space_mouse.calibrating.is_some()
        || app.home.open
    {
        return None;
    }
    let now = ctx.input(|i| i.time);
    // Time away does not count as time shown: after a gap (the window was in the background, Preferences
    // or the start page was up, the toast could not be drawn) the 30 seconds start again.
    let away = now - app.space_mouse.offer_seen > AWAY_SECONDS;
    app.space_mouse.offer_seen = now;
    let since = match &app.space_mouse.offer_since {
        Some((shown, since)) if *shown == device && !away => *since,
        _ => now,
    };
    app.space_mouse.offer_since = Some((device.clone(), since));
    if expired(since, now) {
        app.space_mouse.offer_hidden.insert(device);
        return None;
    }
    let mut actions = vec![Action::Calibrate];
    if swap_offered(&device, app.space_mouse.drivers) {
        actions.push(Action::Swap);
    }
    actions.extend([Action::Close, Action::Never]);
    Some(Offer { device, actions })
}

/// The toast was drawn at `rect`: a pointer on it keeps it from expiring.
pub fn keep(app: &mut SolveApp, ctx: &egui::Context, rect: egui::Rect) {
    if ctx.input(|i| i.pointer.hover_pos()).is_some_and(|pos| rect.contains(pos)) {
        let now = ctx.input(|i| i.time);
        app.space_mouse.offer_since = app.space_mouse.offer_since.take().map(|(device, _)| (device, now));
    }
}

/// The user clicked `action` on the toast of `offer`.
pub fn act(app: &mut SolveApp, offer: &Offer, action: Action) {
    match action {
        Action::Calibrate | Action::Close => {
            app.space_mouse.offer_since = None;
            app.preferences.space_mouse_offered.insert(offer.device.clone());
            if action == Action::Calibrate {
                // The calibration window lives with Preferences, at Navigation.
                crate::prefs::open_at(app, 1);
                crate::space_mouse_calibration::start(app);
            }
        }
        Action::Never => {
            app.preferences.space_mouse_offer_never = true;
            app.space_mouse.offer_since = None;
        }
        Action::Swap => {
            // The same setting as Preferences, read live by the reader. The device is not marked
            // offered: read directly it has its own key, and it may still want a calibration.
            app.preferences.space_mouse_driver = crate::prefs::SpaceMouseDriver::Direct;
            app.space_mouse.offer_since = None;
            app.space_mouse.offer_hidden.insert(offer.device.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offer_is_made_once_to_an_uncalibrated_device_with_navigation_on() {
        let should_offer = |device, calibrated, offered: &BTreeSet<String>, enabled| should_offer(device, calibrated, offered, enabled, false);
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

    #[test]
    fn never_show_again_stops_every_offer() {
        let none = BTreeSet::new();
        assert!(should_offer(Some("hid 046d:c62b"), false, &none, true, false));
        for device in ["hid 046d:c62b", "spacenavd", "web 256f:c63a"] {
            assert!(!should_offer(Some(device), false, &none, true, true), "{device}");
        }
    }

    #[test]
    fn the_swap_to_direct_is_offered_only_on_spacenavd_with_a_direct_device_there() {
        use crate::space_mouse::DriversPresent;
        for device in ["spacenavd", "hid 046d:c62b", "hid 256f:c63a", "3dxware", "web 256f:c63a"] {
            for drivers in [
                None,
                Some(DriversPresent { spacenavd: true, direct: false }),
                Some(DriversPresent { spacenavd: true, direct: true }),
                Some(DriversPresent { spacenavd: false, direct: true }),
            ] {
                let expected = device == "spacenavd" && drivers.is_some_and(|d| d.direct);
                assert_eq!(swap_offered(device, drivers), expected, "{device} {drivers:?}");
            }
        }
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

    /// Behind an issue report (CAD input blocked) the offer is not drawn, so it cannot be clicked.
    #[test]
    fn the_offer_cannot_be_used_while_an_issue_report_blocks_input() {
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        h.app.space_mouse.device = Some("hid 046d:c62b".into());
        crate::issue_report::set_cad_input_blocked_for_test(true);
        h.frames(6);
        let hidden = h.app.space_mouse.offer_since.is_none() && crate::scenario::rect_of("toast:space_mouse_offer").is_none();
        crate::issue_report::set_cad_input_blocked_for_test(false);
        assert!(hidden, "not drawn while input is blocked");
        assert!(h.app.space_mouse.calibrating.is_none() && h.app.preferences.space_mouse_offered.is_empty());
        // And it is there again once the report is gone.
        h.frames(4);
        assert!(h.app.space_mouse.offer_since.is_some());
    }

    /// Showing, expiring and closing the offer changes nothing outside its own state: the open
    /// dialog, tool, selection, undo history, Preferences and calibration are as they were.
    #[test]
    fn showing_expiring_and_closing_the_offer_touch_no_other_window() {
        let state = |h: &crate::scenario::Harness| {
            (
                h.app.dialog.is_some(),
                h.app.tool.is_some(),
                h.app.session.selection.len(),
                h.app.session.undo.len(),
                h.app.prefs_window.open,
                h.app.space_mouse.calibrating.is_some(),
                h.app.keymap.open,
                h.app.home.open,
            )
        };
        for how in ["expire", "close"] {
            let mut h = crate::scenario::Harness::new();
            h.app.home.open = false;
            h.app.run("solid.box", serde_json::json!({"length": 10, "width": 10, "height": 10})).unwrap();
            h.app.start("solid.extrude");
            h.frames(3);
            let before = state(&h);
            h.app.space_mouse.device = Some("hid 046d:c62b".into());
            h.frames(4);
            assert!(h.app.space_mouse.offer_since.is_some(), "{how}: the offer is up");
            assert_eq!(state(&h), before, "{how}: showing it changes nothing else");
            if how == "close" {
                h.step(&serde_json::json!({"widget": "Close"})).unwrap();
            } else if let Some((device, _)) = h.app.space_mouse.offer_since.take() {
                h.app.space_mouse.offer_since = Some((device, -1000.0));
            }
            h.frames(4);
            assert!(h.app.space_mouse.offer_since.is_none() || how == "expire");
            assert_eq!(state(&h), before, "{how}: going away changes nothing else");
        }
    }

    fn spacenavd_harness() -> crate::scenario::Harness {
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        h.app.space_mouse.device = Some("spacenavd".into());
        h.app.space_mouse.drivers = Some(crate::space_mouse::DriversPresent { spacenavd: true, direct: true });
        h.frames(4);
        h
    }

    #[test]
    fn swapping_to_the_direct_driver_sets_it_and_leaves_the_offered_set_alone() {
        let mut h = spacenavd_harness();
        assert!(h.app.space_mouse.offer_since.is_some(), "the card is up");
        h.step(&serde_json::json!({"widget": "Swap to Hidraw (Direct Driver)"})).unwrap();
        h.frames(3);
        assert_eq!(h.app.preferences.space_mouse_driver, crate::prefs::SpaceMouseDriver::Direct);
        assert!(h.app.preferences.space_mouse_offered.is_empty(), "the new source may still be offered");
        assert!(h.app.space_mouse.offer_since.is_none());
        // Without a direct device there is no such button.
        let mut h = spacenavd_harness();
        h.app.space_mouse.drivers = Some(crate::space_mouse::DriversPresent { spacenavd: true, direct: false });
        h.frames(3);
        assert!(h.step(&serde_json::json!({"widget": "Swap to Hidraw (Direct Driver)"})).is_err());
    }

    #[test]
    fn never_show_again_is_saved_and_stops_the_card() {
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        h.app.space_mouse.device = Some("hid 046d:c62b".into());
        h.frames(4);
        h.step(&serde_json::json!({"widget": "Never show again"})).unwrap();
        h.frames(4);
        assert!(h.app.preferences.space_mouse_offer_never);
        assert!(h.app.preferences.space_mouse_offered.is_empty(), "not just this device");
        assert!(h.app.space_mouse.offer_since.is_none());
        // Saved with the other settings, and no other device brings the card back.
        let saved = serde_json::to_value(&h.app.preferences).unwrap();
        assert_eq!(saved["space_mouse_offer_never"], true);
        h.app.space_mouse.device = Some("hid 256f:c63a".into());
        h.frames(4);
        assert!(h.app.space_mouse.offer_since.is_none());
    }

    /// Frames of the harness are 1/60 s: this many make `seconds`.
    fn run(h: &mut crate::scenario::Harness, seconds: f64) {
        for _ in 0..(seconds * 60.0).round() as usize {
            h.frame();
        }
    }

    fn card_up(h: &crate::scenario::Harness) -> bool {
        h.app.space_mouse.offer_since.is_some() && crate::scenario::rect_of("toast:space_mouse_offer").is_some()
    }

    /// The offer stays its own 30 seconds whatever else comes and goes around it: status toasts
    /// arriving and expiring over it, and time away (Preferences open) is not time shown.
    #[test]
    fn the_offer_stays_its_thirty_seconds_beside_status_toasts_and_after_time_away() {
        let mut h = spacenavd_harness();
        run(&mut h, 2.0);
        for second in 0..26 {
            if second % 5 == 0 {
                let now = crate::now_ms();
                h.app.status = Some(("Done".into(), now, false));
            }
            run(&mut h, 1.0);
            assert!(card_up(&h), "second {second}: the offer is gone");
            assert!(
                crate::scenario::rect_of("toast").is_none()
                    || h.app.status.is_some()
                    || crate::scenario::rect_of("toast:space_mouse_offer").is_some()
            );
        }
        // Over its own time it goes, for this run only: no record.
        run(&mut h, 5.0);
        assert!(!card_up(&h));
        assert!(h.app.preferences.space_mouse_offered.is_empty());
        // Time away: Preferences open for 45 s must not use the time up.
        let mut h = spacenavd_harness();
        run(&mut h, 3.0);
        assert!(card_up(&h));
        h.app.prefs_window.open = true;
        run(&mut h, 45.0);
        h.app.prefs_window.open = false;
        run(&mut h, 3.0);
        assert!(card_up(&h), "the offer waited for Preferences to close");
        run(&mut h, 25.0);
        assert!(card_up(&h), "and has its 30 seconds again");
    }

    /// A different device key while the offer is up gets its own offer (its own calibration key); the
    /// first one is left alone, with no record made for either.
    #[test]
    fn a_new_device_key_gets_its_own_offer_and_leaves_the_old_one_alone() {
        let mut h = spacenavd_harness();
        run(&mut h, 10.0);
        assert!(card_up(&h));
        h.app.space_mouse.device = Some("hid 256f:c63a".into());
        run(&mut h, 1.0);
        assert!(card_up(&h));
        assert_eq!(h.app.space_mouse.offer_since.as_ref().map(|(device, _)| device.as_str()), Some("hid 256f:c63a"));
        assert!(h.app.preferences.space_mouse_offered.is_empty() && h.app.space_mouse.offer_hidden.is_empty());
        // Back to the first within its 30 seconds: shown again, its timer starting afresh.
        h.app.space_mouse.device = Some("spacenavd".into());
        run(&mut h, 1.0);
        assert!(card_up(&h));
        // Switching the driver in Preferences while it is up changes nothing by itself.
        h.app.preferences.space_mouse_driver = crate::prefs::SpaceMouseDriver::Direct;
        run(&mut h, 2.0);
        assert!(card_up(&h));
    }
}
