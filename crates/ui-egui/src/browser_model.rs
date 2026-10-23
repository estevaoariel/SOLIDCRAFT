//! Model-only browser queries, reused while navigating. Resolving a placement walks the
//! definition's bodies; doing that for every offscreen row and folder made large imports
//! quadratic on every frame. Keep the document and evaluated state alive with the index so
//! neither a new document nor a later allocation can accidentally reuse its answers.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use solidcraft_engine::Session;
use solidcraft_engine::doc::{Document, FeatureKind, Mat, ModelState};
use solidcraft_engine::occurrences::OccurrencePlacement;

pub(super) struct Model {
    doc: Arc<Document>,
    state: Arc<ModelState>,
    pending_moves: BTreeMap<u64, Mat>,
    pub bodies: HashMap<u64, Vec<usize>>,
    pub sketches: HashMap<u64, Vec<usize>>,
    pub planes: HashMap<u64, Vec<usize>>,
    pub features: HashMap<u64, Vec<usize>>,
    pub children: HashMap<u64, Vec<u64>>,
    placements: HashMap<Vec<u64>, Option<Arc<OccurrencePlacement>>>,
    /// Every placement has been resolved into `placements`, in one pass.
    filled: bool,
    /// Each placed body's placement path, by the body's world name (built on first use).
    body_paths: Option<HashMap<String, Vec<u64>>>,
}

impl Model {
    pub fn new(session: &Session) -> Self {
        let state = session.model.state();
        let mut bodies: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, body) in state.bodies.iter().enumerate() {
            bodies.entry(session.doc.model_body_component(body)).or_default().push(i);
        }
        let mut sketches: HashMap<u64, Vec<usize>> = HashMap::new();
        let mut planes: HashMap<u64, Vec<usize>> = HashMap::new();
        let mut features: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, feature) in session.doc.features.iter().enumerate() {
            if !matches!(feature.kind, FeatureKind::Sketch { .. }) {
                features.entry(feature.component).or_default().push(i);
            }
            let index = match feature.kind {
                FeatureKind::Sketch { .. } => &mut sketches,
                FeatureKind::ConstructionPlane { .. } => &mut planes,
                _ => continue,
            };
            index.entry(feature.component).or_default().push(i);
        }
        let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
        for occurrence in &session.doc.occurrences {
            children.entry(occurrence.parent).or_default().push(occurrence.id);
        }
        Self {
            doc: session.doc.clone(),
            state,
            pending_moves: session.pending_moves.clone(),
            bodies,
            sketches,
            planes,
            features,
            children,
            placements: HashMap::new(),
            filled: false,
            body_paths: None,
        }
    }

    pub fn matches(&self, session: &Session) -> bool {
        self.same_design(session) && self.pending_moves == session.pending_moves
    }

    fn same_design(&self, session: &Session) -> bool {
        Arc::ptr_eq(&self.doc, &session.doc) && Arc::ptr_eq(&self.state, &session.model.state())
    }

    /// Follow the session's uncaptured moves when only they changed (a drag): placements, which
    /// hold where each one is, are resolved again; the tree and which placement holds each body
    /// stay. Rebuilding those walked every placement of a large assembly on every step of a drag
    /// (#3720). False when the design or the model changed (make a new model then).
    pub fn follow_moves(&mut self, session: &Session) -> bool {
        if !self.same_design(session) {
            return false;
        }
        if self.pending_moves != session.pending_moves {
            self.pending_moves = session.pending_moves.clone();
            self.placements.clear();
            self.filled = false;
        }
        true
    }

    /// Has every placement been walked for [`Model::body_path`] (tests: a drag keeps it).
    #[cfg(test)]
    pub fn walked(&self) -> bool {
        self.body_paths.is_some()
    }

    pub fn placement(&mut self, session: &Session, path: &[u64]) -> Option<Arc<OccurrencePlacement>> {
        if !self.filled {
            self.fill(session);
        }
        self.placements.entry(path.to_vec()).or_insert_with(|| session.occurrence_placement(path).ok().map(Arc::new)).clone()
    }

    /// Resolve every placement in one pass over the design, as deep and as far as the browser
    /// draws them (and the first time, which placement holds each body): the rows and the
    /// highlights ask for most of them, and each resolved on its own read the whole design again.
    fn fill(&mut self, session: &Session) {
        self.filled = true;
        let mut paths = self.body_paths.is_none().then(HashMap::<String, Vec<u64>>::new);
        for placement in session.occurrence_placements(100_000) {
            if let Some(paths) = paths.as_mut() {
                for b in &placement.bodies {
                    paths.entry(b.world.clone()).or_insert_with(|| placement.path.clone());
                }
            }
            self.placements.entry(placement.path.clone()).or_insert_with(|| Some(Arc::new(placement)));
        }
        if paths.is_some() {
            self.body_paths = paths;
        }
    }

    /// The placement a placed body (by its world name) belongs to, as the browser's occurrence
    /// rows nest; `None` for a body of the top-level design.
    pub fn body_path(&mut self, session: &Session, body: &str) -> Option<Vec<u64>> {
        if self.body_paths.is_none() {
            self.fill(session);
        }
        self.body_paths.as_ref()?.get(body).cloned()
    }
}
