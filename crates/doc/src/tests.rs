use solidcraft_geom::{Vec2, Vec3};
use solidcraft_kernel::measure;
use solidcraft_sketch::{ConstraintKind, Sketch};

use crate::*;

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-12)
}

/// 40 x 30 rectangle with its width and height driven by `width` / d2.
fn plate_doc() -> (Document, u64) {
    let mut doc = Document::new("plate");
    doc.set_param("width", "40 mm", Some("mm"), None).unwrap();
    let mut sk = Sketch::new();
    let pts = [Vec2::new(0.0, 0.0), Vec2::new(38.0, 0.5), Vec2::new(39.0, 31.0), Vec2::new(0.5, 29.0)];
    let ids: Vec<usize> = pts.iter().map(|p| sk.add_point(*p, None).unwrap()).collect();
    let l: Vec<usize> = (0..4).map(|i| sk.add_line_pts(ids[i], ids[(i + 1) % 4], None).unwrap()).collect();
    use ConstraintKind::*;
    sk.add_constraint(Coincident { p: ids[0], q: 0 }, None).unwrap();
    sk.add_constraint(Horizontal { l: l[0] }, None).unwrap();
    sk.add_constraint(Horizontal { l: l[2] }, None).unwrap();
    sk.add_constraint(Vertical { l: l[1] }, None).unwrap();
    sk.add_constraint(Vertical { l: l[3] }, None).unwrap();
    let d1 = doc.new_model_param("width", "mm");
    sk.add_constraint(Length { l: l[0], value: 40.0 }, Some(d1)).unwrap();
    let d2 = doc.new_model_param("30 mm", "mm");
    sk.add_constraint(Length { l: l[1], value: 30.0 }, Some(d2)).unwrap();
    let s = doc.add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: sk }, None).unwrap();
    doc.add_feature(
        FeatureKind::Extrude {
            thin: None,
            sketch: crate::SketchId(s),
            profiles: ProfileSel::All,
            extent: Extent {
                convention: crate::ExtrudeConvention::LegacyPerSide,
                distance: "20".into(),
                direction: Direction::Positive,
                distance2: None,
                start_offset: None,
                through_all: false,
                taper: None,
                to: None,
                to_offset: None,
                from: None,
            },
            operation: Operation::NewBody,
            targets: vec![],
        },
        None,
    )
    .unwrap();
    (doc, s)
}

#[test]
fn parametric_plate_reevaluates() {
    let (mut doc, _) = plate_doc();
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results);
    let st = m.state();
    assert_eq!(st.bodies.len(), 1);
    assert_eq!(st.bodies[0].name, "Body1");
    assert!(rel(measure(&st.bodies[0].body).unwrap().volume, 24000.0) < 1e-9);
    assert!(st.sketches[0].report.fully_constrained());

    // No change: nothing recomputed.
    m.evaluate(&doc);
    assert_eq!(m.last_recomputed, 0);

    doc.set_param("width", "50 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!(m.last_recomputed, 2);
    assert!(rel(measure(&m.state().bodies[0].body).unwrap().volume, 30000.0) < 1e-9);

    // Changing only the extrude distance re-evaluates only the extrude.
    if let Some(FeatureKind::Extrude { extent, .. }) = doc.features.get_mut(1).map(|f| &mut f.kind) {
        extent.distance = "width / 5".into();
    }
    m.evaluate(&doc);
    assert_eq!(m.last_recomputed, 1);
    assert!(rel(measure(&m.state().bodies[0].body).unwrap().volume, 50.0 * 30.0 * 10.0) < 1e-9);
}

/// Setting a parameter back takes the earlier results from the cache: nothing is recomputed, but
/// the evaluation reports the features it restored (their geometry did change back).
#[test]
fn set_back_reports_restored_features() {
    let (mut doc, _) = plate_doc();
    let mut m = Model::new();
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (2, 0));
    doc.set_param("width", "50 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (2, 0));
    doc.set_param("width", "40 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (0, 2));
    assert!(rel(measure(&m.state().bodies[0].body).unwrap().volume, 24000.0) < 1e-9);
    // No change: nothing recomputed and nothing restored.
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (0, 0));
}

/// #1989: what an evaluation ahead on a copy of the model computed is kept for the run that
/// follows on the model itself, which finds it there even once the evaluation cache has dropped
/// it (the cache keeps only a few states of a design with large bodies). Copies keep nothing.
#[test]
fn results_kept_ahead_outlive_the_cache() {
    let (mut doc, _) = plate_doc();
    let mut m = Model::new();
    m.evaluate(&doc);
    doc.set_param("width", "50 mm", None, None).unwrap();
    let kept = std::sync::Arc::new(KeptResults::default());
    let mut ahead = m.clone();
    ahead.keep(Some(kept.clone()));
    assert!(ahead.clone().kept().is_none(), "a copy keeps nothing");
    ahead.evaluate(&doc);
    assert_eq!((ahead.last_recomputed, kept.len()), (2, 2));
    *m.cache.lock().unwrap() = Default::default();
    m.keep(Some(kept));
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (0, 2), "nothing computed again");
    assert!(rel(measure(&m.state().bodies[0].body).unwrap().volume, 30000.0) < 1e-9);
}

fn add_in(doc: &mut Document, component: u64, name: &str, kind: serde_json::Value) {
    let kind: FeatureKind = serde_json::from_value(kind).unwrap();
    let id = doc.add_feature(kind, Some(name)).unwrap();
    doc.feature_mut(id).unwrap().component = component;
}

/// A plate in one component, then a rod in another, slotted by a box cut patterned round it.
fn two_parts(slot_targets: &[&str]) -> Document {
    let mut doc = Document::new("two parts");
    doc.set_param("w", "40 mm", None, None).unwrap();
    doc.set_param("slot", "1 mm", None, None).unwrap();
    let plate = doc.add_component(Some("Plate"), 0).unwrap();
    let rod = doc.add_component(Some("Rod"), 0).unwrap();
    add_in(
        &mut doc,
        plate,
        "Plate",
        serde_json::json!({"type": "box", "corner": ["0", "0", "0"], "length": "w", "width": "30", "height": "5", "operation": "new_body"}),
    );
    add_in(
        &mut doc,
        rod,
        "Rod",
        serde_json::json!({"type": "cylinder", "base": ["0", "0", "0"], "axis": [0.0, 0.0, 1.0], "radius": "4", "height": "20", "operation": "new_body"}),
    );
    add_in(
        &mut doc,
        rod,
        "Slot",
        serde_json::json!({"type": "box", "corner": ["3", "-0.5", "2"], "length": "2", "width": "slot", "height": "20", "operation": "cut"}),
    );
    for f in doc.features.iter_mut().filter(|f| f.name == "Plate" || f.name == "Rod") {
        f.body_names = vec![f.name.clone()];
    }
    if let Some(f) = doc.features.iter_mut().find(|f| f.name == "Slot") {
        f.participants = slot_targets.iter().map(|t| t.to_string()).collect();
    }
    add_in(
        &mut doc,
        rod,
        "Slots",
        serde_json::json!({"type": "pattern", "features": ["Slot"], "pattern": {"type": "circular", "origin": [0.0, 0.0, 0.0], "axis": [0.0, 0.0, 1.0], "count": "4", "angle": "360 deg"}}),
    );
    doc
}

/// The bodies as an evaluation from scratch makes them: names, volumes, face names.
fn summary(st: &ModelState) -> Vec<(String, u64, i64, Vec<String>)> {
    st.bodies
        .iter()
        .map(|b| {
            let volume = measure(&b.body).unwrap().volume;
            (b.name.clone(), b.component, (volume * 1e6).round() as i64, crate::naming::face_names(b).to_vec())
        })
        .collect()
}

fn from_scratch(doc: &Document) -> Vec<(String, u64, i64, Vec<String>)> {
    let mut m = Model::new();
    m.no_cache = true;
    m.evaluate(doc);
    summary(&m.state())
}

/// An edit in one component leaves another component's features after it as they were: they
/// are restored, not recomputed, and give what evaluating from scratch gives.
#[test]
fn edit_in_one_component_keeps_the_others_results() {
    let mut doc = two_parts(&[]);
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results.iter().map(|r| &r.error).collect::<Vec<_>>());
    assert_eq!(m.last_recomputed, 4);
    doc.set_param("w", "50 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (1, 3));
    assert_eq!(summary(&m.state()), from_scratch(&doc));
    // An edit in the rod's own component recomputes its features.
    doc.set_param("slot", "1.5 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (2, 0));
    assert_eq!(summary(&m.state()), from_scratch(&doc));
    doc.set_param("w", "60 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (1, 3));
    assert_eq!(summary(&m.state()), from_scratch(&doc));
}

/// A sketch on a fixed plane, the new body extruded from it and that body moved read nothing
/// of another component either.
#[test]
fn sketch_extrude_and_move_keep_their_results() {
    let mut doc = two_parts(&[]);
    let rod = doc.find_component("Rod").unwrap();
    let mut sk = Sketch::new();
    sk.add_circle(Vec2::new(20.0, 0.0), 2.0, None, None).unwrap();
    let id = doc.add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: sk }, Some("Peg sketch")).unwrap();
    doc.feature_mut(id).unwrap().component = rod;
    add_in(
        &mut doc,
        rod,
        "Peg",
        serde_json::json!({"type": "extrude", "sketch": id, "profiles": {"type": "all"}, "extent": {"convention": "total_distance", "direction": "positive", "distance": "8"}, "operation": "new_body"}),
    );
    if let Some(f) = doc.features.iter_mut().find(|f| f.name == "Peg") {
        f.body_names = vec!["Peg".into()];
    }
    add_in(&mut doc, rod, "Peg up", serde_json::json!({"type": "move", "bodies": ["Peg"], "translate": ["0", "0", "slot * 3"]}));
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results.iter().map(|r| &r.error).collect::<Vec<_>>());
    doc.set_param("w", "50 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (1, 6));
    assert_eq!(summary(&m.state()), from_scratch(&doc));
    // The move follows its own parameter.
    doc.set_param("slot", "2 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (3, 2));
    assert_eq!(summary(&m.state()), from_scratch(&doc));
}

/// A feature that cuts a body of another component reads it: it is recomputed when that body
/// changes, and so is what follows it in its own component.
#[test]
fn a_cut_of_another_components_body_follows_it() {
    let mut doc = two_parts(&["Plate", "Rod"]);
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results.iter().map(|r| &r.error).collect::<Vec<_>>());
    doc.set_param("w", "50 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (3, 1));
    assert_eq!(summary(&m.state()), from_scratch(&doc));
}

/// A cut of several bodies cuts again only those that changed: the rod, which `w` does not
/// move, keeps the very body its earlier cut gave, while the cap `w` sizes is cut again.
#[test]
fn a_cut_of_several_bodies_cuts_again_only_those_that_changed() {
    let mut doc = Document::new("rod and cap");
    doc.set_param("w", "6 mm", None, None).unwrap();
    let rod = doc.add_component(Some("Rod"), 0).unwrap();
    add_in(
        &mut doc,
        rod,
        "Rod",
        serde_json::json!({"type": "cylinder", "base": ["0", "0", "0"], "axis": [0.0, 0.0, 1.0], "radius": "4", "height": "20", "operation": "new_body"}),
    );
    add_in(
        &mut doc,
        rod,
        "Cap",
        serde_json::json!({"type": "box", "corner": ["-5", "-5", "20"], "length": "10", "width": "10", "height": "w", "operation": "new_body"}),
    );
    for f in doc.features.iter_mut().filter(|f| f.name == "Rod" || f.name == "Cap") {
        f.body_names = vec![f.name.clone()];
    }
    // The slot runs through both: the component's bodies are its targets.
    add_in(
        &mut doc,
        rod,
        "Slot",
        serde_json::json!({"type": "box", "corner": ["3", "-0.5", "2"], "length": "4", "width": "1", "height": "30", "operation": "cut"}),
    );
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results.iter().map(|r| &r.error).collect::<Vec<_>>());
    let cut_rod = m.state().body("Rod").unwrap().body.clone();
    doc.set_param("w", "8 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!(m.last_recomputed, 2, "the cap and the slot");
    let st = m.state();
    assert!(st.body("Rod").unwrap().body.same(&cut_rod), "the rod's cut is the one made before");
    assert_eq!(summary(&st), from_scratch(&doc));
}

/// A parameter that drives the first feature of the timeline, with the bodies after it each cut
/// by a combine: only the first body and its cut are recomputed, every other cut is restored.
#[test]
fn a_combine_reads_only_its_target_and_tools() {
    let mut doc = Document::new("rows");
    doc.set_param("w", "20 mm", None, None).unwrap();
    let c = doc.add_component(Some("Rows"), 0).unwrap();
    for i in 0..3 {
        let x = (i * 30) as f64;
        let (body, tool, cut) = (format!("Body{i}"), format!("Tool{i}"), format!("Cut{i}"));
        let length = if i == 0 { "w".to_string() } else { "20".to_string() };
        add_in(
            &mut doc,
            c,
            &body,
            serde_json::json!({"type": "box", "corner": [x.to_string(), "0", "0"], "length": length, "width": "15", "height": "5", "operation": "new_body"}),
        );
        add_in(
            &mut doc,
            c,
            &tool,
            serde_json::json!({"type": "cylinder", "base": [(x + 7.0).to_string(), "7.5", "-1"], "axis": [0.0, 0.0, 1.0], "radius": "1.5", "height": "7", "operation": "new_body"}),
        );
        for f in doc.features.iter_mut().filter(|f| f.name == body || f.name == tool) {
            f.body_names = vec![f.name.clone()];
        }
        add_in(&mut doc, c, &cut, serde_json::json!({"type": "combine", "target": body, "tools": [tool], "operation": "cut"}));
    }
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results.iter().map(|r| &r.error).collect::<Vec<_>>());
    assert_eq!(m.last_recomputed, 9);
    doc.set_param("w", "32 mm", None, None).unwrap();
    m.evaluate(&doc);
    assert_eq!((m.last_recomputed, m.last_restored), (2, 7), "the first body and its cut");
    assert_eq!(summary(&m.state()), from_scratch(&doc));
}

#[test]
fn box_fillet_cut_timeline() {
    let (mut doc, _) = plate_doc();
    doc.add_feature(
        FeatureKind::Fillet {
            edges: vec![Position(Vec3::new(0.0, 0.0, 10.0))],
            radius: "3".into(),
            body: None,
            style: Default::default(),
            fit: Default::default(),
            corner: Default::default(),
        },
        None,
    )
    .unwrap();
    let mut sk = Sketch::new();
    sk.add_circle(Vec2::new(20.0, 15.0), 5.0, None, None).unwrap();
    let s2 = doc.add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: sk }, None).unwrap();
    doc.add_feature(
        FeatureKind::Extrude {
            thin: None,
            sketch: crate::SketchId(s2),
            profiles: ProfileSel::Points { points: vec![Vec2::new(20.0, 15.0)] },
            extent: Extent {
                convention: crate::ExtrudeConvention::LegacyPerSide,
                distance: "20".into(),
                direction: Direction::Positive,
                distance2: None,
                start_offset: None,
                through_all: false,
                taper: None,
                to: None,
                to_offset: None,
                from: None,
            },
            operation: Operation::Cut,
            targets: vec![],
        },
        None,
    )
    .unwrap();
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results.iter().map(|r| &r.error).collect::<Vec<_>>());
    let st = m.state();
    let v = measure(&st.bodies[0].body).unwrap().volume;
    let expect = 24000.0 - (9.0 - std::f64::consts::PI * 9.0 / 4.0) * 20.0 - std::f64::consts::PI * 25.0 * 20.0;
    assert!(rel(v, expect) < 1e-3, "{v} vs {expect}");
    // Roll back to before the fillet.
    doc.marker = Some(2);
    m.evaluate(&doc);
    assert!(rel(measure(&m.state().bodies[0].body).unwrap().volume, 24000.0) < 1e-9);
    assert!(m.results[3].skipped);
}

#[test]
fn errors_are_reported_not_fatal() {
    let (mut doc, s) = plate_doc();
    doc.add_feature(
        FeatureKind::Revolve {
            sketch: crate::SketchId(s),
            profiles: ProfileSel::All,
            axis: AxisRef::SketchAxis { axis: "y".into() },
            angle: "90 deg".into(),
            operation: Operation::NewBody,
            targets: vec![],
            angle2: None,
            symmetric: false,
            convention: Default::default(),
            to: None,
        },
        None,
    )
    .unwrap();
    doc.add_feature(
        FeatureKind::Fillet {
            edges: vec![Position(Vec3::new(500.0, 0.0, 0.0))],
            radius: "1".into(),
            body: None,
            style: Default::default(),
            fit: Default::default(),
            corner: Default::default(),
        },
        None,
    )
    .unwrap();
    doc.add_feature(
        FeatureKind::Box {
            corner: point_expr(Vec3::ZERO),
            length: "nope".into(),
            width: "1".into(),
            height: "1".into(),
            operation: Operation::NewBody,
        },
        None,
    )
    .unwrap();
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results[2].error.is_none(), "{:?}", m.results[2]);
    assert!(m.results[3].error.is_some());
    assert!(m.results[4].error.as_deref().unwrap_or("").contains("nope"));
    assert_eq!(m.state().bodies.len(), 2);
    let quarter = measure(&m.state().bodies[1].body).unwrap().volume;
    // Revolving the 40x30 rectangle (x in [0,40]) 90° about sketch y: π/4 · 40² · 30.
    assert!(rel(quarter, std::f64::consts::FRAC_PI_4 * 1600.0 * 30.0) < 1e-3, "{quarter}");
}

#[test]
fn params_and_serialization() {
    let (mut doc, _) = plate_doc();
    assert!(doc.set_param("bad name", "1", None, None).is_err());
    assert!(doc.set_param("mm", "1", None, None).is_err());
    assert!(doc.set_param("loop", "loop + 1", None, None).is_err());
    assert!(doc.remove_param("width").is_err(), "used by d1");
    doc.set_param("spare", "3", None, None).unwrap();
    doc.remove_param("spare").unwrap();
    let json = doc.to_json();
    let back = Document::from_json(&json).unwrap();
    assert_eq!(back, doc);
    assert!(Document::from_json("{").is_err());
    let gone = doc.delete_feature(doc.features[0].id).unwrap();
    assert_eq!(gone.len(), 2, "the extrude goes with its sketch");
    assert!(doc.param("d1").is_none(), "unused model parameters are pruned");
}

#[test]
fn primitives_and_combine() {
    let mut doc = Document::new("p");
    doc.add_feature(
        FeatureKind::Box {
            corner: point_expr(Vec3::ZERO),
            length: "10".into(),
            width: "10".into(),
            height: "10".into(),
            operation: Operation::NewBody,
        },
        None,
    )
    .unwrap();
    doc.add_feature(
        FeatureKind::Cylinder {
            base: point_expr(Vec3::new(5.0, 5.0, 0.0)),
            axis: Dir(Vec3::Z),
            radius: "2".into(),
            height: "20".into(),
            operation: Operation::NewBody,
        },
        None,
    )
    .unwrap();
    doc.add_feature(FeatureKind::Combine { target: "Body1".into(), tools: vec!["Body2".into()], operation: Operation::Cut, keep_tools: false }, None)
        .unwrap();
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().all(|r| r.error.is_none()), "{:?}", m.results);
    let st = m.state();
    assert_eq!(st.bodies.len(), 1);
    assert!(rel(measure(&st.bodies[0].body).unwrap().volume, 1000.0 - std::f64::consts::PI * 40.0) < 1e-3);
}

#[test]
fn hostile_expressions_never_panic() {
    // A deterministic stream of nasty inputs: random tokens, deep nesting, huge numbers.
    let toks = [
        "(",
        ")",
        "+",
        "-",
        "*",
        "/",
        "^",
        ",",
        "1e308",
        "0",
        "-0",
        "mm",
        "deg",
        "x",
        "sqrt",
        "pow",
        "min",
        "max",
        "°",
        "1.2.3",
        "e",
        "pi",
        "in",
        "nan",
        "9999999999999999999999",
    ];
    let mut state: u64 = 0x9e3779b97f4a7c15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..5000 {
        let n = (next() % 24) as usize;
        let s: String = (0..n).map(|_| toks[(next() % toks.len() as u64) as usize]).collect::<Vec<_>>().join(" ");
        let _ = crate::expr::eval_with(&s, &|n| {
            if n == "x" { Ok(crate::expr::Value::length(2.0)) } else { Err(crate::DocError::Expr("unknown".into())) }
        });
    }
    let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    assert!(crate::expr::eval_with(&deep, &|_| Err(crate::DocError::Expr("x".into()))).is_err());
    assert!(crate::expr::eval_with("pow(2 mm, 3)", &|_| Err(crate::DocError::Expr("x".into()))).is_ok());
    assert!(crate::expr::eval_with("pow(2 mm, 300)", &|_| Err(crate::DocError::Expr("x".into()))).is_err());
}

#[test]
fn parameter_cycles_are_found() {
    use crate::expr::Kind;
    let list = vec![
        ("a".to_string(), "b + 1".to_string(), Kind::Length),
        ("b".to_string(), "c * 2".to_string(), Kind::Length),
        ("c".to_string(), "a".to_string(), Kind::Length),
        ("d".to_string(), "5".to_string(), Kind::Length),
        ("e".to_string(), "d + e".to_string(), Kind::Length),
    ];
    let c = crate::cycles(&list);
    assert_eq!(c.into_iter().collect::<Vec<_>>(), vec!["a", "b", "c", "e"]);
    let defs: Vec<crate::expr::ParamDef> =
        list.iter().map(|(n, e, _)| crate::expr::ParamDef { name: n.clone(), expr: e.clone(), unit: "mm".into() }).collect();
    let (vals, errs) = crate::expr::eval_params(&defs, 1.0);
    assert!(errs["a"].contains("circular reference: a → b → c → a"), "{}", errs["a"]);
    assert!(vals.contains_key("d") && errs.contains_key("a") && errs.contains_key("e"));
}

/// Features that use a sketch's points or regions go with it (not only profile and path users).
#[test]
fn input_sketches_cover_points_and_patches() {
    let hole: FeatureKind = serde_json::from_value(serde_json::json!({
        "type": "hole", "position": [0, 0, 0], "direction": [0, 0, -1], "diameter": "3",
        "points": {"sketch": 7, "ids": ["p1"]},
    }))
    .unwrap();
    assert_eq!(hole.input_sketches(), vec![7]);
    let patch: FeatureKind = serde_json::from_value(serde_json::json!({"type": "patch", "sketch": 9})).unwrap();
    assert_eq!(patch.input_sketches(), vec![9]);
    let pipe: FeatureKind = serde_json::from_value(serde_json::json!({"type": "pipe", "path_sketch": 72, "path": ["l1"], "diameter": "4"})).unwrap();
    assert_eq!(pipe.input_sketches(), vec![72]);
}

#[test]
fn timeline_clones_share_meshes_even_when_cloned_before_tessellation() {
    let body = solidcraft_kernel::box_solid(Vec3::ZERO, Vec3::new(10., 10., 10.)).unwrap();
    let a = crate::eval::ModelBody::new("box".into(), body, 1);
    let b = a.clone();
    assert!(std::sync::Arc::ptr_eq(&a.mesh(), &b.mesh()));
    assert!(!a.mesh().triangles.is_empty());
}

#[test]
fn feature_inserted_at_a_marker_inside_a_timeline_folder_joins_it() {
    let mut doc = Document::new("folder insertion");
    let first = doc.add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: Sketch::new() }, Some("First")).unwrap();
    let second =
        doc.add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: Sketch::new() }, Some("Second")).unwrap();
    doc.timeline_groups.push(TimelineGroup { id: 100, name: "Pair".into(), parent: None, features: vec![first, second] });
    doc.marker = Some(1);

    let inserted =
        doc.add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: Sketch::new() }, Some("Inserted")).unwrap();

    assert_eq!(doc.features.iter().map(|feature| feature.id).collect::<Vec<_>>(), vec![first, inserted, second]);
    assert_eq!(doc.timeline_groups[0].features, vec![first, inserted, second]);
    assert_eq!(doc.marker, Some(2));
    assert!(non_contiguous_timeline_group(&doc.features, &doc.timeline_groups).is_none());
    assert!(Document::from_json(&doc.to_json()).is_ok(), "the inserted feature must leave a reopenable design");
}

/// A construction plane that is its own side resolves through the model and back into the
/// document. The depth count has to survive that hop, or the recursion overflows the stack and
/// aborts the process, which no panic guard can intercept.
#[test]
fn self_referencing_construction_planes_are_errors() {
    let selfref = |name: &str| Box::new(construct::GeoRef::Plane { plane: PlaneRef::Construction { name: name.into() } });
    let xz = || Box::new(construct::GeoRef::Plane { plane: PlaneRef::Origin { name: "XZ".into() } });

    // Directly its own first side.
    let mut doc = Document::new("selfplane");
    doc.add_feature(FeatureKind::ConstructionPlane { plane: PlaneRef::Midplane { a: selfref("P1"), b: xz() } }, Some("P1")).unwrap();
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().any(|r| r.error.is_some()), "a plane that is its own side must report an error: {:?}", m.results);

    // Two planes that are each other's side.
    let mut doc = Document::new("twoplanes");
    doc.add_feature(FeatureKind::ConstructionPlane { plane: PlaneRef::Midplane { a: selfref("P2"), b: xz() } }, Some("P1")).unwrap();
    doc.add_feature(FeatureKind::ConstructionPlane { plane: PlaneRef::Midplane { a: selfref("P1"), b: xz() } }, Some("P2")).unwrap();
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().any(|r| r.error.is_some()), "a two-plane cycle must report an error: {:?}", m.results);

    // A perpendicular plane whose base is itself, and a sketch placed on it.
    let mut doc = Document::new("perpplane");
    let line = Box::new(construct::GeoRef::World { axis: "X".into() });
    doc.add_feature(FeatureKind::ConstructionPlane { plane: PlaneRef::Perpendicular { plane: selfref("P1"), line } }, Some("P1")).unwrap();
    doc.add_feature(FeatureKind::Sketch { plane: PlaneRef::Construction { name: "P1".into() }, sketch: Sketch::new() }, Some("Sketch1")).unwrap();
    let mut m = Model::new();
    m.evaluate(&doc);
    assert!(m.results.iter().any(|r| r.error.is_some()), "a cyclic perpendicular plane must report an error: {:?}", m.results);
}
