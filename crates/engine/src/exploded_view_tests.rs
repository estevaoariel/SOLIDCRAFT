//! The exploded view on show is view state: it says which view the pending moves belong to, and
//! undo and redo keep the two together.
use crate::Session;
use serde_json::{Value, json};

fn run(session: &mut Session, command: &str, params: Value) -> Value {
    session.execute(command, &params).unwrap_or_else(|error| panic!("{command}: {error}"))
}

/// Two boxes in two components, so an exploded view has something to move.
fn assembly() -> Session {
    let mut s = Session::default();
    for (name, x) in [("A", 0), ("B", 20)] {
        run(&mut s, "component.activate", json!({"component": "root"}));
        run(&mut s, "component.create", json!({"name": name}));
        run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "corner": [x, 0, 0]}));
    }
    run(&mut s, "component.activate", json!({"component": "root"}));
    s
}

/// Showing a view names it in the session; showing none forgets it, and the pending moves go
/// with it. The design is not touched either way.
#[test]
fn showing_a_view_names_it_and_showing_none_forgets_it() {
    let mut s = assembly();
    assert_eq!(s.exploded_view, None);
    let created = run(&mut s, "explode.create", json!({"name": "Apart", "scale": 1}));
    let view = created["view"].as_u64().unwrap();
    run(&mut s, "explode.show", json!({"view": "Apart"}));
    assert_eq!(s.exploded_view, Some(view));
    assert!(!s.pending_moves.is_empty());
    let design = s.doc.clone();
    run(&mut s, "explode.show", json!({"view": null}));
    assert_eq!(s.exploded_view, None);
    assert!(s.pending_moves.is_empty());
    assert_eq!(*s.doc, *design, "shown only");
}

/// Undo and redo carry the view and its moves together: a step taken after the view was shown
/// puts both back as they were.
#[test]
fn undo_and_redo_keep_the_view_and_its_moves_together() {
    let mut s = assembly();
    run(&mut s, "explode.create", json!({"name": "Apart", "scale": 1}));
    run(&mut s, "explode.show", json!({"view": "Apart"}));
    let shown = s.exploded_view;
    let moves = s.pending_moves.clone();
    // A design change after the view was shown: undoing it brings the view back with it.
    run(&mut s, "solid.box", json!({"length": 5, "width": 5, "height": 5, "body_name": "Extra"}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.exploded_view, shown);
    assert_eq!(s.pending_moves, moves);
    // And the same the other way: a redo brings the design change back, view and all.
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.exploded_view, shown);
    assert_eq!(s.pending_moves, moves);
}
