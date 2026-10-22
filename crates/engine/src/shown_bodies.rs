//! Which bodies of a world state are shown, worked out once for every body and read by position.
//!
//! The viewport walks every body several times a frame (picking, display refinement, the model
//! scene, the ground shadow) and asked [`Session::body_visible`] by name each time. With a
//! placement or folder hidden, each of those asks took a lock and looked the name up in a set of
//! names. [`Session::shown_bodies`] answers for the whole state at once and keeps the answer
//! until the design, the state or what is hidden changes.

use std::sync::Arc;

use solidcraft_doc::{Document, ModelState};

use crate::Session;

/// Whether each body of a world state is shown, by its position in the state.
#[derive(Debug, Default, PartialEq)]
pub struct ShownBodies {
    shown: Vec<bool>,
}

impl ShownBodies {
    /// Is the body at `index` shown? A position past the state's bodies is not.
    pub fn at(&self, index: usize) -> bool {
        self.shown.get(index).copied().unwrap_or(false)
    }

    /// How many bodies it answers for.
    pub fn len(&self) -> usize {
        self.shown.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }
}

/// One answer, with everything [`Session::body_visible`] reads: the design and states are held,
/// so no other one can take their addresses while the answer is kept.
struct Entry {
    doc: Arc<Document>,
    model: Arc<ModelState>,
    world: Arc<ModelState>,
    revision: u64,
    active_sketch: Option<u64>,
    hidden_bodies: Vec<String>,
    hidden_occurrences: Vec<Vec<u64>>,
    hidden_folders: Vec<String>,
    shown: Arc<ShownBodies>,
}

/// The last answers (the viewport asks about the shown world and the world state, which are
/// the same state once every part is meshed).
#[derive(Default)]
pub(crate) struct ShownCache {
    entries: Vec<Entry>,
}

const KEPT: usize = 2;

impl Entry {
    fn matches(&self, s: &Session, model: &Arc<ModelState>, world: &Arc<ModelState>) -> bool {
        let v = &s.visibility;
        self.revision == s.revision
            && self.active_sketch == s.active_sketch
            && Arc::ptr_eq(&self.doc, &s.doc)
            && Arc::ptr_eq(&self.model, model)
            && Arc::ptr_eq(&self.world, world)
            && self.hidden_bodies == v.hidden_bodies
            && self.hidden_occurrences == v.hidden_occurrences
            && self.hidden_folders == v.hidden_folders
    }
}

impl Session {
    /// [`Session::body_visible`] for every body of `world` (a world state of this session), by
    /// position, kept until the design, the state or what is hidden changes.
    pub fn shown_bodies(&self, world: &Arc<ModelState>) -> Arc<ShownBodies> {
        let model = self.model.state();
        if let Ok(cache) = self.shown_bodies_cache.lock()
            && let Some(e) = cache.entries.iter().find(|e| e.matches(self, &model, world))
        {
            return e.shown.clone();
        }
        let shown = Arc::new(ShownBodies { shown: world.bodies.iter().map(|b| self.body_visible(&b.name)).collect() });
        if let Ok(mut cache) = self.shown_bodies_cache.lock() {
            let v = &self.visibility;
            cache.entries.retain(|e| Arc::ptr_eq(&e.doc, &self.doc) && e.revision == self.revision);
            if cache.entries.len() >= KEPT {
                cache.entries.remove(0);
            }
            cache.entries.push(Entry {
                doc: self.doc.clone(),
                model,
                world: world.clone(),
                revision: self.revision,
                active_sketch: self.active_sketch,
                hidden_bodies: v.hidden_bodies.clone(),
                hidden_occurrences: v.hidden_occurrences.clone(),
                hidden_folders: v.hidden_folders.clone(),
                shown: shown.clone(),
            });
        }
        shown
    }
}
