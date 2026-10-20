//! The Exploded View dialog: the step list, the scrubber, the playback and the gizmo that
//! reaches the current step's move.

use serde_json::json;
use solidcraft_engine::Session;

use super::*;
use crate::Services;
use crate::dialogs::apply_commands;
use crate::explode_gizmo::ExplodeDrag;

/// Components A and B, a 20 x 20 x 10 box each, B 30 mm along X.
fn two_boxes() -> SolveApp {
    let mut s = Session::default();
    for (name, x) in [("A", 0), ("B", 30)] {
        s.execute("component.activate", &json!({"component": "root"})).unwrap();
        s.execute("component.create", &json!({ "name": name })).unwrap();
        s.execute("solid.box", &json!({"length": 20, "width": 20, "height": 10, "corner": [x, 0, 0]})).unwrap();
    }
    s.execute("component.activate", &json!({"component": "root"})).unwrap();
    SolveApp::new(s, Services::default())
}

/// The occurrence ids of the design, in the order the dialog lists them.
fn occs(app: &SolveApp) -> Vec<u64> {
    let mut v: Vec<(u64, String)> = app.session.doc.occurrences.iter().map(|o| (o.id, o.name.clone())).collect();
    v.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(id, _)| id).collect()
}

/// The ids of the commands OK runs.
fn ids(cmds: &[(String, Value)]) -> Vec<&str> {
    cmds.iter().map(|(c, _)| c.as_str()).collect()
}

/// The steps of the dialog, ready to be edited.
fn steps_of(d: &mut Dialog) -> &mut Vec<ExplodeStep> {
    let Kind::Motion(Mo::Explode { steps, .. }) = &mut d.kind else { panic!("no Exploded View dialog") };
    steps
}

/// The step list can be edited: a step added, renamed, moved and deleted changes what OK runs.
#[test]
fn the_step_list_can_be_edited() {
    let mut app = two_boxes();
    app.start("explode.create");
    let mut d = app.dialog.clone().unwrap();
    let occ = occs(&app);
    let steps = steps_of(&mut d);
    // A second step, with a move of its own.
    steps.push(ExplodeStep {
        id: 0,
        name: "Lift".into(),
        moves: vec![ExplodeMove { occurrence: occ[1], translate: [0.0, 0.0, 10.0] }],
        duration_ms: 250,
    });
    steps[0].name = "Apart".into();
    steps[0].moves = vec![ExplodeMove { occurrence: occ[0], translate: [5.0, 0.0, 0.0] }];
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(ids(&cmds), vec!["explode.create", "explode.step_edit", "explode.step_add", "explode.show"]);
    // The view's first step holds the first step's moves; the second step is added.
    assert_eq!(cmds[0].1["moves"], json!([{"occurrence": occ[0], "translate": [5.0, 0.0, 0.0]}]));
    assert_eq!(cmds[1].1["name"], "Apart");
    assert_eq!(cmds[1].1["step"], 0);
    assert_eq!(cmds[1].1["moves"], json!([{"occurrence": occ[0], "translate": [5.0, 0.0, 0.0]}]));
    assert_eq!(cmds[2].1["name"], "Lift");
    assert_eq!(cmds[2].1["duration_ms"], 250);
    assert_eq!(cmds[2].1["moves"], json!([{"occurrence": occ[1], "translate": [0.0, 0.0, 10.0]}]));
    // A step moves up the list.
    let steps = steps_of(&mut d);
    assert_eq!(shift_step(steps, 1, true), Some(0));
    assert_eq!(steps[0].name, "Lift");
    assert_eq!(steps[1].name, "Apart");
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(cmds[1].1["name"], "Lift");
    assert_eq!(cmds[2].1["name"], "Apart");
    // A step at the end cannot move down, and the first cannot move up.
    let steps = steps_of(&mut d);
    assert_eq!(shift_step(steps, 1, false), None);
    assert_eq!(shift_step(steps, 0, true), None);
    // A step is deleted.
    steps_of(&mut d).remove(0);
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(ids(&cmds), vec!["explode.create", "explode.step_edit", "explode.show"]);
    assert_eq!(cmds[0].1["moves"], json!([{"occurrence": occ[0], "translate": [5.0, 0.0, 0.0]}]));
}

/// The easing and the trail lines are written into the view.
#[test]
fn the_easing_and_the_trail_lines_are_written_into_the_view() {
    let mut app = two_boxes();
    app.start("explode.create");
    let mut d = app.dialog.clone().unwrap();
    let Kind::Motion(Mo::Explode { easing, trail_lines, .. }) = &mut d.kind else { panic!("no Exploded View dialog") };
    *easing = 0;
    *trail_lines = false;
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(cmds[0].0, "explode.create");
    assert_eq!(cmds[0].1["easing"], "linear");
    assert_eq!(cmds[0].1["trail_lines"], false);
    // An existing view is changed in place.
    let mut app = two_boxes();
    app.run("explode.create", json!({"name": "Apart", "scale": 1.0})).unwrap();
    app.start("explode.create");
    let mut d = app.dialog.clone().unwrap();
    let id = app.session.doc.assembly.exploded[0].id;
    let Kind::Motion(Mo::Explode { view, easing, trail_lines, .. }) = &mut d.kind else { panic!("no Exploded View dialog") };
    *view = Some(id);
    *easing = 2;
    *trail_lines = true;
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(ids(&cmds), vec!["explode.edit", "explode.step_edit", "explode.show"]);
    assert_eq!(cmds[0].1["view"], id);
    assert_eq!(cmds[0].1["easing"], "ease_out");
    assert_eq!(cmds[0].1["trail_lines"], true);
}

/// An existing view is loaded to edit it, and its steps are changed in place.
#[test]
fn an_existing_view_is_loaded_to_edit() {
    let mut app = two_boxes();
    app.run("explode.create", json!({"name": "Apart", "scale": 1.0})).unwrap();
    app.start("explode.create");
    let mut d = app.dialog.clone().unwrap();
    // The dialog offers the views of the design.
    let views = explode_views(&mut app);
    assert_eq!(views.len(), 1);
    let (name, steps, easing, trail) = read_view(&views[0]);
    assert_eq!(name, "Apart");
    assert_eq!(steps.len(), 1);
    assert_eq!(easing, DEFAULT_EASING);
    assert!(trail);
    // Picking it loads it: the commands edit it instead of making a new one.
    let id = app.session.doc.assembly.exploded[0].id;
    let Kind::Motion(Mo::Explode { view, name, steps, .. }) = &mut d.kind else { panic!("no Exploded View dialog") };
    *view = Some(id);
    *name = "Apart2".into();
    steps.push(ExplodeStep { id: 0, name: "Second".into(), moves: Vec::new(), duration_ms: 300 });
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(ids(&cmds), vec!["explode.edit", "explode.step_edit", "explode.step_add", "explode.show"]);
    assert_eq!(cmds[0].1["name"], "Apart2");
    assert_eq!(cmds[1].1["step"], 0);
    assert_eq!(cmds[2].1["name"], "Second");
    // A step the design has and the dialog does not is dropped.
    steps_of(&mut d).clear();
    let cmds = apply_commands(&app, &d).unwrap();
    assert_eq!(ids(&cmds), vec!["explode.edit", "explode.step_delete", "explode.show"]);
    assert_eq!(cmds[1].1["step"], 0);
}

/// A new view may not take the name of another one: the commands that follow name the view.
#[test]
fn a_new_view_needs_a_name_of_its_own() {
    let mut app = two_boxes();
    app.run("explode.create", json!({"name": "Apart", "scale": 1.0})).unwrap();
    app.start("explode.create");
    let mut d = app.dialog.clone().unwrap();
    let Kind::Motion(Mo::Explode { name, .. }) = &mut d.kind else { panic!("no Exploded View dialog") };
    *name = "Apart".into();
    let error = apply_commands(&app, &d).unwrap_err();
    assert!(error.contains("Apart"), "{error}");
}

/// Closing the dialog without OK puts the occurrences back where they were.
#[test]
fn closing_without_ok_puts_the_occurrences_back() {
    let mut app = two_boxes();
    app.start("explode.create");
    let mut d = app.dialog.clone().unwrap();
    let mut m = solidcraft_engine::doc::IDENTITY;
    m[3][0] = 12.0;
    let shown: BTreeMap<u64, Mat> = [(1, m)].into_iter().collect();
    let Kind::Motion(Mo::Explode { saved, .. }) = &mut d.kind else { panic!("no Exploded View dialog") };
    display(&mut app, saved, shown.clone());
    assert_eq!(app.session.pending_moves, shown);
    closed(&mut app, &mut d, false);
    assert!(app.session.pending_moves.is_empty(), "the occurrences are back where they were");
    assert!(app.explode_pick.is_none(), "the gizmo goes with the dialog");
}

/// The gizmo's drag becomes the current step's move for that occurrence.
#[test]
fn the_gizmos_drag_reaches_the_step() {
    let mut app = two_boxes();
    let mut steps = vec![ExplodeStep { id: 0, name: "Explode".into(), moves: Vec::new(), duration_ms: DEFAULT_STEP_MS }];
    let mut taken = None;
    app.explode_drag = Some(ExplodeDrag { occurrence: 7, base: Vec3::ZERO, move_vec: Vec3::new(1.0, 2.0, 3.0), done: false });
    take_drag(&app, &mut steps, 0, &mut taken);
    assert_eq!(steps[0].moves, vec![ExplodeMove { occurrence: 7, translate: [1.0, 2.0, 3.0] }]);
    // The same drag is not taken twice (the viewport keeps it for a frame after it ends).
    take_drag(&app, &mut steps, 0, &mut taken);
    assert_eq!(steps[0].moves.len(), 1);
    // A drag of another occurrence is a move of its own.
    app.explode_drag = Some(ExplodeDrag { occurrence: 9, base: Vec3::ZERO, move_vec: Vec3::new(4.0, 0.0, 0.0), done: true });
    take_drag(&app, &mut steps, 0, &mut taken);
    assert_eq!(steps[0].moves.len(), 2);
    // The drag reaches the step that is being edited.
    let mut steps = vec![new_step(), new_step()];
    let mut taken = None;
    take_drag(&app, &mut steps, 1, &mut taken);
    assert!(steps[0].moves.is_empty());
    assert_eq!(steps[1].moves.len(), 1);
}

/// The dialog shows its rows: the view, the easing, the trail lines, the steps and the
/// playback.
#[test]
fn the_dialog_shows_its_rows() {
    let mut h = crate::scenario::Harness::new();
    h.call("ui.resize", json!({"width": 1400, "height": 900}));
    h.app.run("component.activate", json!({"component": "root"})).unwrap();
    h.app.run("component.create", json!({ "name": "A" })).unwrap();
    h.app.run("solid.box", json!({"length": 20, "width": 20, "height": 10})).unwrap();
    h.app.start("explode.create");
    h.frames(3);
    for label in ["Name", "Easing", "Trail lines", "Explode", "Playback", "Sequence", "Add step"] {
        assert!(h.widget(label).is_some(), "missing {label}");
    }
    // The step list is there, with the step's name and its length.
    assert!(h.widget("Explode").is_some());
}

/// The dialog publishes the occurrence the user has picked: that is what the gizmo is drawn
/// from.
#[test]
fn the_dialog_publishes_the_gizmos_pick() {
    let mut h = crate::scenario::Harness::new();
    h.app.run("component.activate", json!({"component": "root"})).unwrap();
    h.app.run("component.create", json!({ "name": "A" })).unwrap();
    h.app.run("solid.box", json!({"length": 20, "width": 20, "height": 10})).unwrap();
    h.app.start("explode.create");
    h.frames(2);
    let occurrence = h.app.session.doc.occurrences.first().map(|o| o.id).unwrap();
    h.app.session.selection = vec![Sel::Occurrence { path: vec![occurrence] }];
    h.frames(1);
    let pick = h.app.explode_pick.expect("the dialog publishes the pick");
    assert_eq!(pick.occurrence, occurrence);
}

/// The scrubber moves the pose: the parts are assembled at the start of the sequence and apart
/// at its end.
#[test]
fn the_scrubber_moves_the_pose() {
    let mut h = crate::scenario::Harness::new();
    for (name, x) in [("A", 0), ("B", 30)] {
        h.app.run("component.activate", json!({"component": "root"})).unwrap();
        h.app.run("component.create", json!({ "name": name })).unwrap();
        h.app.run("solid.box", json!({"length": 20, "width": 20, "height": 10, "corner": [x, 0, 0]})).unwrap();
    }
    h.app.run("component.activate", json!({"component": "root"})).unwrap();
    h.app.start("explode.create");
    h.frames(3);
    // The automatic explosion fills the first step: both parts move apart.
    assert_eq!(h.app.session.pending_moves.len(), 2, "the dialog shows the explosion ({})", error_of(&h));
    // At the start of the sequence the parts are where they are.
    set_time(&mut h, 0.0);
    h.frames(2);
    assert!(h.app.session.pending_moves.is_empty(), "at the start the parts are assembled");
    // At its end they are apart.
    set_time(&mut h, 500.0);
    h.frames(2);
    assert_eq!(h.app.session.pending_moves.len(), 2, "at the end the parts are apart");
}

/// The automatic explosion moves every top-level occurrence away from the centre.
#[test]
fn the_automatic_explosion_moves_the_parts() {
    let app = two_boxes();
    let moves = automatic_moves(&app, 1.0).unwrap();
    assert_eq!(moves.len(), 2, "{moves:?}");
}

/// Put the scrubber at `ms` into the sequence.
fn set_time(h: &mut crate::scenario::Harness, ms: f64) {
    let Some(d) = h.app.dialog.as_mut() else { panic!("no dialog") };
    let Kind::Motion(Mo::Explode { time, .. }) = &mut d.kind else { panic!("no Exploded View dialog") };
    *time = ms;
}

/// What the dialog says went wrong, if anything.
fn error_of(h: &crate::scenario::Harness) -> String {
    let Some(d) = h.app.dialog.as_ref() else { return "no dialog".into() };
    match &d.kind {
        Kind::Motion(Mo::Explode { error, .. }) => error.clone().unwrap_or_default(),
        _ => "no Exploded View dialog".into(),
    }
}
