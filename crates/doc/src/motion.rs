//! Assembly extras: contact sets (occurrences that stop each other when joints are driven),
//! motion studies (joint values keyed along a timeline) and exploded views (occurrence moves
//! shown without changing the design).

use serde::{Deserialize, Serialize};
use solidcraft_geom::Vec3;

/// Occurrences whose bodies may touch but not pass through each other while joints move.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContactSet {
    pub id: u64,
    pub name: String,
    pub occurrences: Vec<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suppressed: bool,
}

/// One joint value along a motion study: (step, value) points, values in internal units
/// (radians, mm), linear in between and held before the first and after the last.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudyKey {
    pub joint: u64,
    #[serde(default)]
    pub index: usize,
    pub points: Vec<(f64, f64)>,
}

impl StudyKey {
    pub fn value_at(&self, step: f64) -> Option<f64> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        if step <= first.0 {
            return Some(first.1);
        }
        if step >= last.0 {
            return Some(last.1);
        }
        self.points.windows(2).find_map(|w| {
            let (a, b) = (w.first()?, w.get(1)?);
            (step >= a.0 && step <= b.0).then(|| if b.0 - a.0 < 1e-12 { b.1 } else { a.1 + (b.1 - a.1) * (step - a.0) / (b.0 - a.0) })
        })
    }
}

/// A motion study: a timeline of `steps` steps along which joints follow their keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionStudy {
    pub id: u64,
    pub name: String,
    pub steps: u32,
    pub keys: Vec<StudyKey>,
}

/// How an exploded view blends one step into the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EaseKind {
    Linear,
    /// Slow at the start, quick at the end (the default).
    #[default]
    EaseInOut,
    EaseIn,
    EaseOut,
}

impl EaseKind {
    /// How much of a step's move is done at `t` (0…1): `t` for linear, `t²` for ease in,
    /// `1-(1-t)²` for ease out and the smoothstep `t²(3-2t)` for ease in and out.
    /// A `t` that is not finite is treated as 0.
    pub fn apply(self, t: f64) -> f64 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        match self {
            EaseKind::Linear => t,
            EaseKind::EaseIn => t * t,
            EaseKind::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            EaseKind::EaseInOut => t * t * (3.0 - 2.0 * t),
        }
    }
}

/// One step of an exploded view: the occurrences it moves and how long it takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExplodeStep {
    pub id: u64,
    pub name: String,
    /// (occurrence, world translation) this step adds to the occurrence's offset.
    pub moves: Vec<(u64, Vec3)>,
    /// How long the step takes when the view plays (milliseconds). 0 means the default
    /// ([`DEFAULT_STEP_MS`]).
    #[serde(default)]
    pub duration_ms: u32,
}

/// The default length of a step when it does not say (milliseconds).
pub const DEFAULT_STEP_MS: u32 = 500;

/// The id of a step in a view: the view's id and the step's place in it. The ids are stable
/// (a design keeps them when it is saved again) and unique within the view, where a step is
/// named by its index or by its id.
pub fn step_id(view: u64, index: u64) -> u64 {
    view.saturating_mul(1000).saturating_add(index)
}

/// An exploded view: occurrences moved apart in steps (shown only; the design keeps its
/// placements).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExplodedView {
    pub id: u64,
    pub name: String,
    /// The steps, in order. Each step's moves add to the ones before it.
    #[serde(default)]
    pub steps: Vec<ExplodeStep>,
    /// Draw a dashed line from every moved occurrence back to the place it is assembled in.
    #[serde(default = "trail_lines_on")]
    pub trail_lines: bool,
    /// How the steps blend into one another.
    #[serde(default)]
    pub easing: EaseKind,
}

fn trail_lines_on() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_interpolate_and_hold() {
        let k = StudyKey { joint: 1, index: 0, points: vec![(0.0, 0.0), (10.0, 1.0), (20.0, -1.0)] };
        assert_eq!(k.value_at(-5.0), Some(0.0));
        assert_eq!(k.value_at(5.0), Some(0.5));
        assert_eq!(k.value_at(15.0), Some(0.0));
        assert_eq!(k.value_at(99.0), Some(-1.0));
        assert_eq!(StudyKey { joint: 1, index: 0, points: vec![] }.value_at(1.0), None);
    }

    /// The easing blends a step's progress: nothing is done at 0, all of it at 1, and what is
    /// between is the curve. A progress outside 0…1 is held at the ends, and one that is not
    /// a number at all starts the step.
    #[test]
    fn easing_blends_a_step() {
        for kind in [EaseKind::Linear, EaseKind::EaseIn, EaseKind::EaseOut, EaseKind::EaseInOut] {
            assert_eq!(kind.apply(0.0), 0.0, "{kind:?}");
            assert_eq!(kind.apply(1.0), 1.0, "{kind:?}");
            let middle = kind.apply(0.5);
            assert!((0.0..=1.0).contains(&middle), "{kind:?}: {middle}");
        }
        assert_eq!(EaseKind::Linear.apply(0.5), 0.5);
        assert_eq!(EaseKind::EaseIn.apply(0.5), 0.25);
        assert_eq!(EaseKind::EaseOut.apply(0.5), 0.75);
        assert_eq!(EaseKind::EaseInOut.apply(0.5), 0.5);
        // Ease in is slower than linear, ease out faster, over the whole step.
        assert!(EaseKind::EaseIn.apply(0.25) < EaseKind::Linear.apply(0.25));
        assert!(EaseKind::EaseOut.apply(0.25) > EaseKind::Linear.apply(0.25));
        // A progress outside 0…1 is held at the ends.
        assert_eq!(EaseKind::EaseIn.apply(-1.0), 0.0);
        assert_eq!(EaseKind::EaseOut.apply(2.0), 1.0);
        // A progress that is not a number starts the step.
        assert_eq!(EaseKind::Linear.apply(f64::NAN), 0.0);
        assert_eq!(EaseKind::EaseIn.apply(f64::INFINITY), 0.0);
        assert_eq!(EaseKind::EaseOut.apply(f64::NEG_INFINITY), 0.0);
    }
}
