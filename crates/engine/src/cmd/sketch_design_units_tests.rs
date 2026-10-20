//! A dimension added without a value restates what it measures in the design's units (#3812):
//! `1 in` for a 25.4 mm line in an inch design, so the value box opens in the units the
//! dimension's text shows. Millimetre designs keep `… mm`, and nothing moves.

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A sketch with a 25.4 mm line `l1` in a design in `units`.
fn line(units: &str) -> Session {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY"}));
    run(&mut s, "sketch.line", json!({"points": [[0, 0], [25.4, 0]], "ids": ["l1"]}));
    run(&mut s, "document.units", json!({"units": units}));
    s
}

fn param(s: &Session, name: &str) -> (String, String) {
    let p = s.doc.param(name).unwrap();
    (p.expr.clone(), p.unit.clone())
}

fn end(s: &Session) -> f64 {
    let state = s.model.state();
    let sk = &state.sketch(s.active_sketch.unwrap()).unwrap().sketch;
    sk.point(sk.resolve_point("l1.end").unwrap()).unwrap().x
}

#[test]
fn a_dimension_without_a_value_is_written_in_the_design_units() {
    for (units, expr) in [("in", "1 in"), ("mm", "25.4 mm"), ("cm", "2.54 cm"), ("ft", "0.083333333 ft"), ("m", "0.0254 m")] {
        let mut s = line(units);
        let r = run(&mut s, "sketch.dimension", json!({"entities": ["l1"], "scale_sketch": true}));
        let name = r["param"].as_str().unwrap();
        assert_eq!(param(&s, name), (expr.to_string(), units.to_string()), "{units}");
        assert!((end(&s) - 25.4).abs() < 1e-5, "{units}: the restated dimension moves nothing ({})", end(&s));
    }
}

#[test]
fn a_typed_bare_number_still_means_the_design_units() {
    let mut s = line("in");
    let r = run(&mut s, "sketch.dimension", json!({"entities": ["l1"], "value": "2"}));
    assert_eq!(param(&s, r["param"].as_str().unwrap()).0, "2 in");
    assert!((end(&s) - 50.8).abs() < 1e-6);
}

#[test]
fn a_driven_dimension_made_driving_and_autoconstrain_dimensions_use_the_design_units() {
    let mut s = line("in");
    run(&mut s, "sketch.dimension", json!({"entities": ["l1"], "driven": true}));
    let state = s.model.state();
    let id = state.sketch(s.active_sketch.unwrap()).unwrap().sketch.constraints.iter().find(|c| c.driven).unwrap().id.clone();
    run(&mut s, "sketch.toggle_driven", json!({"constraint": id, "driven": false}));
    let made = s.doc.params.iter().find(|p| p.model).unwrap();
    assert_eq!((made.expr.as_str(), made.unit.as_str()), ("1 in", "in"));

    let mut s = line("in");
    run(&mut s, "sketch.auto_constrain", json!({}));
    let lengths: Vec<_> = s.doc.params.iter().filter(|p| p.model && p.unit != "deg").collect();
    assert!(!lengths.is_empty(), "AutoConstrain dimensioned the line");
    assert!(lengths.iter().all(|p| p.unit == "in" && p.expr.ends_with(" in")), "{lengths:?}");
}
