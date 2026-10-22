//! Extrude by dragging: press on a planar face (or a sketch profile) and pull. Once the pointer
//! has moved far enough upward, or along the surface's normal on screen, Extrude starts on the
//! pick; the distance follows the pointer along the normal (snapped like the extrude arrow) and
//! letting go applies it. A release back at zero distance cancels. A press that goes any other
//! way stays a box selection.

use egui::Pos2;
use solidcraft_engine::Sel;
use solidcraft_engine::geom::Vec3;

use crate::SolveApp;
use crate::dialogs::{Kind, planar_face};
use crate::viewport::{Hit, Proj};

/// Screen distance the pointer must travel before a press counts as a pull.
const START_PX: f32 = 6.0;
/// How closely (cosine) a drag must follow the normal's screen direction to start a pull.
const ALIGNED: f32 = 0.8;
/// A distance below this is "no extrude" at release.
const MIN_DISTANCE: f64 = 1e-6;

/// A press on a surface that may turn into a pull.
#[derive(Clone, Debug)]
pub struct Gesture {
    press: Pos2,
    sel: Sel,
    /// A point on the surface where it was pressed, and its unit normal.
    base: Vec3,
    normal: Vec3,
    /// Where along the normal the press was: the distance is measured from here.
    grab: f64,
    /// Extrude has started (the window is open and follows the pointer).
    started: bool,
}

#[derive(Debug, Default)]
pub struct State {
    gesture: Option<Gesture>,
    /// The pull ended with a distance: the dialog applies on its next frame.
    pub(crate) commit: bool,
}

/// A pull has started: the box selection stays out of it.
pub fn started(app: &SolveApp) -> bool {
    app.viewport.drag_extrude.gesture.as_ref().is_some_and(|g| g.started)
}

/// Note a press at `at` when it is on a planar face or a profile.
pub fn press(app: &mut SolveApp, proj: &Proj, at: Pos2) {
    app.viewport.drag_extrude.gesture = None;
    let hits = crate::viewport::pick(app, proj, at);
    let Some((hit, Some(sel))) = crate::viewport::candidate(app, &hits).map(|(h, s)| (h, Some(s))) else { return };
    let (o, d) = proj.ray(at);
    let (base, normal) = match (&hit, &sel) {
        (Hit::Face { point, .. }, Sel::Face { body, index, .. }) => match planar_face(&app.session, body, *index) {
            Some((_, n)) => (*point, n),
            None => return,
        },
        (Hit::Profile { sketch, .. }, Sel::Profile { .. }) => {
            let Some(plane) = crate::canvas::profile_plane(app, *sketch) else { return };
            match plane.intersect_ray(o, d) {
                Some(p) => (p, plane.normal()),
                None => return,
            }
        }
        _ => return,
    };
    let Some(grab) = crate::canvas::along(proj, at, base, normal) else { return };
    app.viewport.drag_extrude.gesture = Some(Gesture { press: at, sel, base, normal, grab, started: false });
}

/// Whether the move from the press to `p` is a pull: upward, or along the normal on screen.
fn pulls(proj: &Proj, g: &Gesture, p: Pos2) -> bool {
    let delta = p - g.press;
    if delta.length() < START_PX {
        return false;
    }
    let unit = delta / delta.length();
    let upward = delta.y < 0.0 && -unit.y > ALIGNED;
    let along_normal = proj
        .to_screen(g.base)
        .zip(proj.to_screen(g.base + g.normal))
        .map(|(a, b)| b - a)
        .filter(|n| n.length() > 1e-3)
        .is_some_and(|n| (n / n.length()).dot(unit).abs() > ALIGNED);
    upward || along_normal
}

/// The pointer is at `p` with the button down.
pub fn to(app: &mut SolveApp, proj: &Proj, p: Pos2) {
    let Some(g) = app.viewport.drag_extrude.gesture.clone() else { return };
    if !g.started {
        if app.dialog.is_some() || !pulls(proj, &g, p) {
            return;
        }
        crate::viewport::select(app, Some(g.sel.clone()), false, false);
        app.start("solid.press_pull");
        if !matches!(app.dialog.as_ref().map(|d| &d.kind), Some(Kind::Extrude { .. })) {
            app.viewport.drag_extrude.gesture = None;
            return;
        }
        app.viewport.drag_extrude.gesture = Some(Gesture { started: true, ..g.clone() });
    }
    let Some(v) = crate::canvas::along(proj, p, g.base, g.normal) else { return };
    let free = crate::tool_modifiers::manipulator_free();
    let step = crate::canvas::snap_step(app.cam.half_height());
    let travelled = v - g.grab;
    let travelled = if free { travelled } else { (travelled / step).round() * step };
    let Some(Kind::Extrude { distance, direction, .. }) = app.dialog.as_mut().map(|d| &mut d.kind) else { return };
    // A reversed extrude measures its distance against the normal.
    let sign = if matches!(*direction, 1 | crate::dialogs::TWO_SIDES_REVERSED) { -1.0 } else { 1.0 };
    *distance = crate::canvas::format_len(travelled * sign);
}

/// The button is up: apply the pull, or drop it when it came back to nothing.
pub fn end(app: &mut SolveApp) {
    let Some(g) = app.viewport.drag_extrude.gesture.take() else { return };
    if !g.started {
        return;
    }
    let length = match app.dialog.as_ref().map(|d| &d.kind) {
        Some(Kind::Extrude { distance, .. }) => app.session.doc.eval(distance, solidcraft_engine::doc::expr::Kind::Length).ok(),
        _ => return,
    };
    if length.is_some_and(|l| l.is_finite() && l.abs() > MIN_DISTANCE) {
        app.viewport.drag_extrude.commit = true;
    } else {
        crate::dialogs::cancel(app);
    }
}
