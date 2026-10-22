//! The exploded view's translation gizmo: what the viewport draws it from and what a drag of
//! one of its axes means.
//!
//! The gizmo pulls an occurrence along X, Y or Z while the Exploded View dialog is open. It is
//! display only: the drag changes the view's step, never the occurrence's own placement.

use solidcraft_engine::geom::Vec3;

use crate::gizmo::{self, Change, Drag, Handle};

/// What the exploded view's gizmo shows: the occurrence the user is pulling, where the part is
/// now and the translation the current step's move has for it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExplodePick {
    /// The occurrence the user is pulling (0 when nothing is picked).
    pub occurrence: u64,
    /// Where the part is now — the triad's centre (the occurrence's exploded origin).
    pub center: Vec3,
    /// The move of the current step's move has for it — the triad's axes.
    pub move_vec: Vec3,
}

/// The drag of the exploded view's gizmo: the occurrence being pulled and the translation it
/// has been pulled to. The dialog takes it into the current step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExplodeDrag {
    /// The occurrence the user is pulling.
    pub occurrence: u64,
    /// The move when the drag began.
    pub base: Vec3,
    /// The move now.
    pub move_vec: Vec3,
    /// The drag ended (the viewport keeps the value for one more frame, so the dialog sees the
    /// last translation of the drag).
    pub done: bool,
}

impl ExplodeDrag {
    /// The move a drag of the triad's X, Y or Z arrow means: the base move with that one
    /// component replaced by the drag. A drag along no axis (the centre ball) changes nothing.
    pub fn moved(base: Vec3, delta: Vec3) -> Vec3 {
        let mut m = base;
        if delta.x != 0.0 {
            m.x = base.x + delta.x;
        }
        if delta.y != 0.0 {
            m.y = base.y + delta.y;
        }
        if delta.z != 0.0 {
            m.z = base.z + delta.z;
        }
        m
    }
}

/// The triad the viewport draws for a pick: the part's place now, with the three axis arrows.
pub fn triad(pick: &ExplodePick) -> gizmo::Triad {
    gizmo::Triad { center: pick.center, translate: true, rotate: false }
}

/// What the viewport remembers between the frames of a drag.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    /// The move the drag began from: the base the drag's change is added to.
    base: Vec3,
    /// The occurrence the base belongs to.
    occurrence: u64,
    /// The drag the dialog is told about.
    published: Option<ExplodeDrag>,
    /// Frames the finished drag is still published.
    keep: u8,
}

impl State {
    /// The drag to publish this frame.
    ///
    /// A drag of an axis arrow moves the occurrence along that axis: the move it had when the
    /// drag began plus the drag's change. The other handles of the triad (the plane squares
    /// and the centre ball) move nothing, and a drag that is not a translation changes
    /// nothing.
    ///
    /// The base is the move the drag began from: the pick's move on the first frame of the
    /// drag, the base of the drag in progress after that.
    pub fn publish(&mut self, pick: Option<&ExplodePick>, drag: Option<&Drag>) -> Option<ExplodeDrag> {
        match (pick, drag) {
            (Some(pick), Some(drag)) => {
                let (Handle::Axis(_), Change::Translate(delta)) = (drag.handle, drag.change) else {
                    // Not an axis: the move stays as it is.
                    self.published = None;
                    self.keep = 0;
                    return None;
                };
                let base = if drag.started || self.occurrence != pick.occurrence { pick.move_vec } else { self.base };
                self.base = base;
                self.occurrence = pick.occurrence;
                self.published = Some(ExplodeDrag { occurrence: pick.occurrence, base, move_vec: ExplodeDrag::moved(base, delta), done: drag.done });
                self.keep = if drag.done { 1 } else { 0 };
            }
            (None, _) => {
                // The dialog is closed: nothing to pull.
                self.published = None;
                self.keep = 0;
            }
            (Some(_), None) => {
                // The drag ended: keep the last translation for one more frame, so the dialog
                // sees it, then clear it.
                if self.keep > 0 {
                    self.keep -= 1;
                } else {
                    self.published = None;
                }
            }
        }
        self.published
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A drag along one axis changes that axis only; the others keep the move they had.
    #[test]
    fn a_drag_changes_one_axis_only() {
        let base = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(ExplodeDrag::moved(base, Vec3::new(5.0, 0.0, 0.0)), Vec3::new(6.0, 2.0, 3.0));
        assert_eq!(ExplodeDrag::moved(base, Vec3::new(0.0, -4.0, 0.0)), Vec3::new(1.0, -2.0, 3.0));
        assert_eq!(ExplodeDrag::moved(base, Vec3::new(0.0, 0.0, 0.5)), Vec3::new(1.0, 2.0, 3.5));
        // A drag that is not along an axis (the centre ball) moves nothing.
        assert_eq!(ExplodeDrag::moved(base, Vec3::ZERO), base);
    }
}
