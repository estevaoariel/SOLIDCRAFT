//! Constraint tool picks: the Symmetry tool's third pick may be the origin point or an origin axis
//! (#1075); a model edge or another sketch's curve is projected into the sketch first.

use serde_json::json;
use solidcraft_engine::geom::Vec2;

use crate::scenario::Harness;

/// An XY sketch with points p (10, 5) and q (-12, -3), the Symmetry tool started and both picked.
fn two_points_picked() -> Harness {
    let mut h = Harness::new();
    h.app.ui.show_origin = true;
    h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
    h.app.run("sketch.point", json!({"point":[10,5],"id":"p"})).unwrap();
    h.app.run("sketch.point", json!({"point":[-12,-3],"id":"q"})).unwrap();
    h.app.cam.distance = 120.0;
    h.settle();
    h.app.start("sketch.constraint.symmetry");
    for xy in [[10, 5], [-12, -3]] {
        let at = h.call("ui.at", json!({ "sketch": xy }))["result"].clone();
        h.step(&json!({ "click": at })).unwrap();
    }
    h
}

fn point(h: &Harness, id: &str) -> Vec2 {
    let sk = h.app.session.doc.sketch(h.app.session.active_sketch.unwrap()).unwrap();
    sk.point(sk.resolve_point(id).unwrap()).unwrap()
}

#[test]
fn symmetry_about_the_origin_point() {
    let mut h = two_points_picked();
    let at = h.call("ui.at", json!({"sketch":[0,0]}))["result"].clone();
    h.step(&json!({ "click": at })).unwrap();
    assert!((point(&h, "p") + point(&h, "q")).len() < 1e-6, "{:?} {:?}", point(&h, "p"), point(&h, "q"));
}

#[test]
fn symmetry_about_an_origin_axis() {
    let mut h = two_points_picked();
    // On the Y axis, away from the origin point and from p and q.
    let at = h.call("ui.at", json!({"sketch":[0,6]}))["result"].clone();
    h.step(&json!({ "click": at })).unwrap();
    let (p, q) = (point(&h, "p"), point(&h, "q"));
    assert!((p.x + q.x).abs() < 1e-6 && (p.y - q.y).abs() < 1e-6, "mirrored in the Y axis: {p:?} {q:?}");
}

/// A 40 × 30 × 10 box (corner at the origin) and a sketch on its top face's plane holding the
/// a line from (10, 10) to (20, 12), the camera looking down on it.
fn box_and_line() -> Harness {
    let mut h = Harness::new();
    h.app.run("solid.box", json!({"length":40,"width":30,"height":10})).unwrap();
    h.app.run("sketch.create", json!({"plane":"XY","offset":10})).unwrap();
    h.app.run("sketch.line", json!({"points":[[10,10],[20,12]]})).unwrap();
    h.app.cam.distance = 120.0;
    h.settle();
    h
}

fn click_world(h: &mut Harness, at: [f64; 3]) {
    let at = h.call("ui.at", json!({ "world": at }))["result"].clone();
    h.step(&json!({ "click": at })).unwrap();
}

#[test]
fn coincident_onto_a_model_edge_projects_the_edge_first() {
    let mut h = box_and_line();
    h.app.start("sketch.constraint.coincident");
    let end = h.call("ui.at", json!({"sketch":[20,12]}))["result"].clone();
    h.step(&json!({ "click": end })).unwrap();
    // The box's top edge along y = 0, away from its corners.
    click_world(&mut h, [30.0, 0.0, 10.0]);
    let sk = h.app.session.doc.sketch(h.app.session.active_sketch.unwrap()).unwrap();
    assert_eq!(sk.links.len(), 1, "the edge was projected: {:?}", h.app.status);
    assert!(
        sk.constraints.iter().any(|c| matches!(c.kind, solidcraft_engine::sketch::ConstraintKind::PointOnCurve { .. })),
        "the end is on the projected edge: {:?}",
        h.app.status
    );
    let line = sk.curves.iter().find(|c| c.link.is_none()).unwrap().id.clone();
    let end = point(&h, &format!("{line}.end"));
    assert!(end.y.abs() < 1e-6, "{end:?}");
}

#[test]
fn another_sketchs_curve_is_projected_not_named_in_this_sketch() {
    let mut h = Harness::new();
    h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
    h.app.run("sketch.circle.center", json!({"center":[0,0],"radius":10})).unwrap();
    h.app.finish_sketch();
    h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
    h.app.run("sketch.line", json!({"points":[[20,20],[30,25]]})).unwrap();
    h.app.cam.distance = 120.0;
    h.settle();
    h.app.start("sketch.constraint.tangent");
    let line = h.call("ui.at", json!({"sketch":[25,22.5]}))["result"].clone();
    h.step(&json!({ "click": line })).unwrap();
    let circle = h.call("ui.at", json!({"sketch":[-10,0]}))["result"].clone();
    h.step(&json!({ "click": circle })).unwrap();
    let sk = h.app.session.doc.sketch(h.app.session.active_sketch.unwrap()).unwrap();
    assert_eq!(sk.links.len(), 1, "{:?}", h.app.status);
    assert!(sk.constraints.iter().any(|c| matches!(c.kind, solidcraft_engine::sketch::ConstraintKind::Tangent { .. })), "{:?}", h.app.status);
}
