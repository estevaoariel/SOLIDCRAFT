//! An exploded view keeps its explosion as steps: a list of steps, each with the occurrences it
//! moves and how long it takes, and the view's easing and trail lines. Older designs need a
//! transformation: a view with one flat list of moves gets one step holding those moves, so the
//! explosion is shown exactly as it was. The last release asks for an update instead of reading
//! a view whose explosion is a list of steps.
use serde_json::Value;

use crate::motion::{DEFAULT_STEP_MS, step_id};

pub fn step(v: &mut Value) -> crate::Result<()> {
    let Some(Value::Array(views)) = v.get_mut("assembly").and_then(|a| a.get_mut("exploded")) else {
        return Ok(());
    };
    for view in views.iter_mut() {
        let Some(o) = view.as_object_mut() else { continue };
        // A view that already has steps is left exactly as it is.
        if o.contains_key("steps") {
            continue;
        }
        let moves = o.remove("moves");
        let id = o.get("id").and_then(Value::as_u64).unwrap_or(0);
        let steps = match moves {
            // A view without moves has nothing to show: an empty list of steps.
            None => Vec::new(),
            Some(moves) => {
                if !moves.is_array() {
                    return Err(crate::DocError::Invalid("damaged design: an exploded view's moves are not a list".into()));
                }
                vec![serde_json::json!({
                    "id": step_id(id, 0),
                    "name": "Explode",
                    "moves": moves,
                    "duration_ms": DEFAULT_STEP_MS,
                })]
            }
        };
        o.insert("steps".into(), Value::Array(steps));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// A view with a flat list of moves becomes a view with one step holding those moves; a
    /// view that already has steps is left exactly as it is.
    #[test]
    fn older_exploded_views_become_one_step() {
        let mut v = json!({"assembly": {"exploded": [
            {"id": 3, "name": "Apart", "moves": [[1, [10, 0, 0]], [2, [0, 20, 0]]]},
            {"id": 4, "name": "Already stepped", "steps": [{"id": 4000, "name": "Burst", "moves": [[7, [1, 2, 3]]], "duration_ms": 250}]},
            {"id": 5, "name": "Nothing"}
        ]}});
        super::step(&mut v).unwrap();
        let views = v["assembly"]["exploded"].as_array().unwrap();
        // The old view keeps its moves, now in one step.
        assert_eq!(views[0]["steps"].as_array().unwrap().len(), 1);
        assert_eq!(views[0]["steps"][0]["moves"], json!([[1, [10, 0, 0]], [2, [0, 20, 0]]]));
        assert_eq!(views[0]["steps"][0]["duration_ms"], json!(500));
        // Its old flat list of moves is gone.
        assert!(views[0].get("moves").is_none(), "{}", views[0]);
        // A view that already has steps keeps them, untouched.
        assert_eq!(views[1]["steps"], json!([{"id": 4000, "name": "Burst", "moves": [[7, [1, 2, 3]]], "duration_ms": 250}]));
        // A view with no moves at all gets an empty list of steps.
        assert_eq!(views[2]["steps"], json!([]));
        // A design without exploded views is left alone.
        let mut empty = json!({"name": "Untitled"});
        super::step(&mut empty).unwrap();
        assert_eq!(empty, json!({"name": "Untitled"}));
    }
}
