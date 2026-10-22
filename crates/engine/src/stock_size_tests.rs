//! Stock size (#137): each body's length × width × thickness along its own directions, in
//! `inspect.measure` and `component.properties`. Every expected value is worked out by hand from
//! the geometry; moving, turning or mirroring a part never changes its size.

use serde_json::{Value, json};

use crate::Session;

/// Exact cases agree to this (mm).
const TOL: f64 = 1e-3;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn body(s: &mut Session, b: &str) -> Value {
    run(s, "inspect.measure", json!({ "bodies": [b] }))["bodies"][0].clone()
}

fn stock(s: &mut Session, b: &str) -> Value {
    let v = body(s, b);
    let stock = v["stock"].clone();
    assert!(stock.is_object(), "{b} has no stock size: {v}");
    stock
}

fn sizes(stock: &Value) -> [f64; 3] {
    let n = |k: &str| stock[k].as_f64().unwrap_or(f64::NAN);
    [n("length"), n("width"), n("thickness")]
}

fn world(b: &Value) -> [f64; 3] {
    let n = |k: &str, i: usize| b["bbox"][k][i].as_f64().unwrap_or(f64::NAN);
    [n("max", 0) - n("min", 0), n("max", 1) - n("min", 1), n("max", 2) - n("min", 2)]
}

fn assert_size(got: [f64; 3], want: [f64; 3], what: &str) {
    let ok = got.iter().zip(want).all(|(g, w)| (g - w).abs() < TOL);
    assert!(ok, "{what}: {got:?}, want {want:?}");
}

/// The world box no longer matches `before`: the move really happened.
fn assert_moved(b: &Value, before: [f64; 3]) {
    let now = world(b);
    assert!(now.iter().zip(before).any(|(n, w)| (n - w).abs() > 1.0), "the move changed nothing: {now:?}");
}

fn exact(stock: &Value) -> bool {
    stock["exact"].as_bool().unwrap_or(false)
}

fn new_box(s: &mut Session, name: &str, l: f64, w: f64, h: f64) {
    run(s, "solid.box", json!({"length": l, "width": w, "height": h, "body_name": name}));
}

fn turn(s: &mut Session, name: &str, axis: [f64; 3], deg: f64) {
    run(s, "solid.move", json!({"bodies": [name], "axis": axis, "angle": format!("{deg} deg")}));
}

/// A plain box is its own stock.
#[test]
fn a_plain_box_is_its_own_stock() {
    let mut s = Session::default();
    new_box(&mut s, "P", 100.0, 50.0, 18.0);
    let st = stock(&mut s, "P");
    assert_size(sizes(&st), [100.0, 50.0, 18.0], "plain box");
    assert!(exact(&st), "{st}");
}

/// The bench's Left end stands on edge, 18 × 591.6 × 878.4 in world X/Y/Z; its stock is
/// longest first, and the world box is unchanged beside it.
#[test]
fn stock_is_longest_first_not_world_order() {
    let mut s = Session::default();
    new_box(&mut s, "Left end", 18.0, 591.6, 878.4);
    assert_size(sizes(&stock(&mut s, "Left end")), [878.4, 591.6, 18.0], "left end");
    assert_size(world(&body(&mut s, "Left end")), [18.0, 591.6, 878.4], "world box");
}

/// Turned 30° in plan, the world box grows to 100·cos 30° + 50·sin 30° by
/// 100·sin 30° + 50·cos 30°; the stock stays 100 × 50 × 18.
#[test]
fn turning_in_plan_keeps_the_stock() {
    let mut s = Session::default();
    new_box(&mut s, "P", 100.0, 50.0, 18.0);
    turn(&mut s, "P", [0.0, 0.0, 1.0], 30.0);
    let (sin, cos) = 30f64.to_radians().sin_cos();
    assert_size(world(&body(&mut s, "P")), [100.0 * cos + 50.0 * sin, 100.0 * sin + 50.0 * cos, 18.0], "world box");
    let st = stock(&mut s, "P");
    assert_size(sizes(&st), [100.0, 50.0, 18.0], "turned 30°");
    assert!(exact(&st), "{st}");
    // The length runs along the 100 mm side, now at 30° in plan.
    let along = st["axes"][0].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect::<Vec<_>>()).unwrap_or_default();
    let dot = along.first().copied().unwrap_or(f64::NAN) * cos + along.get(1).copied().unwrap_or(f64::NAN) * sin;
    assert!((dot.abs() - 1.0).abs() < 1e-6, "length axis {along:?} isn't along the 100 mm side");
}

/// Turned 17° about X, then 23° about Z.
#[test]
fn turning_on_a_skew_axis_keeps_the_stock() {
    let mut s = Session::default();
    new_box(&mut s, "P", 100.0, 50.0, 18.0);
    turn(&mut s, "P", [1.0, 0.0, 0.0], 17.0);
    turn(&mut s, "P", [0.0, 0.0, 1.0], 23.0);
    assert_moved(&body(&mut s, "P"), [100.0, 50.0, 18.0]);
    let st = stock(&mut s, "P");
    assert_size(sizes(&st), [100.0, 50.0, 18.0], "skew turn");
    assert!(exact(&st), "{st}");
}

/// #137's case: an end panel splayed 5°.
#[test]
fn a_splayed_end_keeps_its_stock() {
    let mut s = Session::default();
    new_box(&mut s, "Left end", 18.0, 591.6, 878.4);
    turn(&mut s, "Left end", [0.0, 1.0, 0.0], 5.0);
    assert_moved(&body(&mut s, "Left end"), [18.0, 591.6, 878.4]);
    let st = stock(&mut s, "Left end");
    assert_size(sizes(&st), [878.4, 591.6, 18.0], "splayed 5°");
    assert!(exact(&st), "{st}");
}

/// A 10 mm chamfer on one end edge only removes material: the board is still 600 × 90 × 19.
#[test]
fn a_chamfered_end_keeps_the_board_size() {
    let mut s = Session::default();
    new_box(&mut s, "Board", 600.0, 90.0, 19.0);
    run(&mut s, "solid.chamfer", json!({"edges": [[600, 45, 19]], "distance": 10, "body": "Board"}));
    let st = stock(&mut s, "Board");
    assert_size(sizes(&st), [600.0, 90.0, 19.0], "chamfered board");
    assert!(exact(&st), "{st}");
}

/// An L-bracket (legs 100 and 60, 10 thick, 40 deep): its largest face isn't the thickness.
#[test]
fn an_l_bracket_is_measured_along_its_legs() {
    let mut s = Session::default();
    new_box(&mut s, "L", 100.0, 10.0, 40.0);
    run(&mut s, "solid.box", json!({"length": 10, "width": 60, "height": 40, "operation": "join", "targets": ["L"]}));
    let st = stock(&mut s, "L");
    assert_size(sizes(&st), [100.0, 60.0, 40.0], "L-bracket");
    assert!(exact(&st), "{st}");
}

/// A square panel: length = width, and the same axes every time.
#[test]
fn a_square_panel_is_stable() {
    let mut s = Session::default();
    new_box(&mut s, "Sq", 50.0, 50.0, 18.0);
    let a = stock(&mut s, "Sq");
    let b = stock(&mut s, "Sq");
    assert_size(sizes(&a), [50.0, 50.0, 18.0], "square panel");
    assert_eq!(a, b, "two measures differ");
}

/// A dowel, ø 20 × 300, is curved: approximate, and its box still encloses it.
#[test]
fn a_dowel_is_approximate_and_enclosed() {
    let mut s = Session::default();
    run(&mut s, "solid.cylinder", json!({"diameter": 20, "height": 300, "axis": "X", "body_name": "Dowel"}));
    let st = stock(&mut s, "Dowel");
    let [l, w, t] = sizes(&st);
    assert!(!exact(&st), "a cylinder can't be exact: {st}");
    assert!((l - 300.0).abs() < TOL, "{st}");
    for d in [w, t] {
        assert!((20.0 - 1e-6..=20.5).contains(&d), "the box must enclose the ø 20 dowel: {st}");
    }
}

/// An 18 mm panel bent on a 500 mm radius through 60°, 600 tall: approximate, and the box
/// is never smaller than the part.
#[test]
fn a_curved_panel_is_approximate_and_enclosed() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XZ", "name": "Section"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [500, 0], "p1": [518, 600]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.revolve", json!({"sketch": "Section", "axis": "Z", "angle": "60 deg", "body_name": "Bent"}));
    let b = body(&mut s, "Bent");
    let st = stock(&mut s, "Bent");
    let [l, w, t] = sizes(&st);
    assert!(!exact(&st), "a bent panel can't be exact: {st}");
    assert!(l >= 600.0 - TOL, "{st}");
    let volume = b["volume_mm3"].as_f64().unwrap_or(f64::NAN);
    assert!(l * w * t >= volume, "the box ({l} × {w} × {t}) is smaller than the part ({volume} mm³)");
    // At least as tight as the world box (with 1 mm slack for the approximate box's margin).
    let [x, y, z] = world(&b);
    assert!(l * w * t <= (x + 1.0) * (y + 1.0) * (z + 1.0), "the box ({l} × {w} × {t}) is looser than the world box ({x} × {y} × {z})");
}

/// Turning an occurrence 40° about Y leaves its body's stock unchanged.
#[test]
fn turning_an_occurrence_keeps_the_stock() {
    let mut s = Session::default();
    let c = run(&mut s, "component.create", json!({"name": "Panel"}));
    new_box(&mut s, "P", 100.0, 50.0, 18.0);
    let before = run(&mut s, "component.properties", json!({"component": c["component"]}))["bodies"][0]["stock"].clone();
    run(&mut s, "occurrence.move", json!({"occurrence": c["occurrence"], "axis": [0, 1, 0], "angle": "40 deg"}));
    let moved = run(&mut s, "component.properties", json!({"component": c["component"]}))["bodies"][0].clone();
    assert_moved(&moved, [100.0, 50.0, 18.0]);
    let after = moved["stock"].clone();
    assert!(before.is_object(), "no stock size in component.properties: {before}");
    assert_size(sizes(&before), [100.0, 50.0, 18.0], "before the move");
    assert_size(sizes(&after), [100.0, 50.0, 18.0], "after the move");
}

/// A mirrored copy has the same stock as the original.
#[test]
fn a_mirrored_copy_has_the_same_stock() {
    let mut s = Session::default();
    new_box(&mut s, "P", 100.0, 50.0, 18.0);
    turn(&mut s, "P", [1.0, 0.0, 0.0], 17.0);
    turn(&mut s, "P", [0.0, 0.0, 1.0], 23.0);
    run(&mut s, "solid.mirror", json!({"bodies": ["P"], "plane": "XZ"}));
    let all = run(&mut s, "inspect.measure", json!({}));
    let Some(copy) = all["bodies"].as_array().and_then(|b| b.iter().find(|b| b["name"] != "P")).cloned() else {
        panic!("no mirrored body: {all}");
    };
    assert_size(sizes(&copy["stock"]), [100.0, 50.0, 18.0], "mirrored copy");
    assert!(exact(&copy["stock"]), "{copy}");
}

/// `inspect.measure` and `component.properties` report the same stock for a body.
#[test]
fn both_commands_agree() {
    let mut s = Session::default();
    new_box(&mut s, "P", 100.0, 50.0, 18.0);
    turn(&mut s, "P", [1.0, 0.0, 0.0], 17.0);
    turn(&mut s, "P", [0.0, 0.0, 1.0], 23.0);
    let a = stock(&mut s, "P");
    let b = run(&mut s, "component.properties", json!({"component": "root"}))["bodies"][0]["stock"].clone();
    assert_size(sizes(&b), sizes(&a), "component.properties against inspect.measure");
    assert_eq!(a["exact"], b["exact"]);
}

/// Kind of stock: a panel, a board, a tube (constant cross-section) and a sphere.
#[test]
#[ignore = "the kind of stock (SS-4) lands after the stock box"]
fn each_body_has_a_kind_of_stock() {
    let mut s = Session::default();
    new_box(&mut s, "Panel", 600.0, 400.0, 18.0);
    run(&mut s, "solid.box", json!({"length": 1000, "width": 89, "height": 38, "corner": [0, 1000, 0], "body_name": "Board"}));
    run(&mut s, "solid.box", json!({"length": 1000, "width": 25, "height": 25, "corner": [0, 2000, 0], "body_name": "Tube"}));
    run(&mut s, "solid.box", json!({"length": 1002, "width": 21, "height": 21, "corner": [-1, 2002, 2], "operation": "cut", "targets": ["Tube"]}));
    run(&mut s, "solid.sphere", json!({"radius": 30, "center": [0, 3000, 0], "body_name": "Ball"}));
    for (name, kind) in [("Panel", "panel"), ("Board", "board"), ("Tube", "length"), ("Ball", "irregular")] {
        let st = stock(&mut s, name);
        assert_eq!(st["kind"], kind, "{name}: {st}");
    }
}

/// The same body measured twice gives identical numbers and axes.
#[test]
fn measuring_twice_is_identical() {
    let mut s = Session::default();
    new_box(&mut s, "P", 100.0, 50.0, 18.0);
    turn(&mut s, "P", [1.0, 1.0, 0.0], 33.0);
    let a = stock(&mut s, "P");
    let b = stock(&mut s, "P");
    assert_eq!(a, b);
}
