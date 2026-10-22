//! How each body of the shown world looks (its colour, opacity and face appearances), worked out
//! once per body and kept until the design or the world state changes.
//!
//! The model scene asked the design for every body's colour, opacity and face colours on every
//! rebuild, and a rebuild comes with hiding a part or a change of display refinement, not only
//! with a change to the design. Face colours scan every face appearance of the design, and work
//! out face names for imported bodies.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use solidcraft_engine::doc::appearance::Look;
use solidcraft_engine::doc::{Document, ModelBody, ModelState};

use crate::SolveApp;

/// One body's look, as [`Document::body_color`], [`Document::model_body_opacity`] and
/// [`Document::face_colors`] give it.
pub(crate) struct BodyLook {
    pub color: Option<[f32; 3]>,
    pub opacity: f64,
    /// Faces with looks of their own, by B-rep face index.
    pub faces: Arc<HashMap<usize, Look>>,
}

impl BodyLook {
    fn of(doc: &Document, b: &ModelBody) -> Self {
        BodyLook { color: doc.body_color(b), opacity: doc.model_body_opacity(b), faces: Arc::new(doc.face_colors(b).into_iter().collect()) }
    }

    /// Neither the body nor any of its faces is see-through.
    pub fn opaque(&self) -> bool {
        self.opacity >= 0.999 && self.faces.values().all(|l| l.opacity >= 0.999)
    }
}

/// The looks of one world state's bodies, by position, each worked out the first time it is
/// asked for. The design and the state are held, so no other one can take their addresses.
pub(crate) struct Looks {
    doc: Arc<Document>,
    world: Arc<ModelState>,
    bodies: Vec<OnceLock<BodyLook>>,
    by_name: OnceLock<HashMap<String, usize>>,
}

impl Looks {
    /// The looks of `world`'s bodies for the open design: the last ones while neither changed.
    pub fn of(app: &SolveApp, world: &Arc<ModelState>) -> Arc<Looks> {
        let doc = &app.session.doc;
        let same = |l: &Arc<Looks>| Arc::ptr_eq(&l.doc, doc) && Arc::ptr_eq(&l.world, world);
        if let Some(l) = app.viewport.looks.lock().ok().and_then(|c| c.as_ref().filter(|l| same(l)).cloned()) {
            return l;
        }
        let looks = Arc::new(Looks {
            doc: doc.clone(),
            world: world.clone(),
            bodies: world.bodies.iter().map(|_| OnceLock::new()).collect(),
            by_name: OnceLock::new(),
        });
        if let Ok(mut c) = app.viewport.looks.lock() {
            *c = Some(looks.clone());
        }
        looks
    }

    /// The look of the body at `index` in the state.
    pub fn at(&self, index: usize) -> Option<&BodyLook> {
        let (b, slot) = (self.world.bodies.get(index)?, self.bodies.get(index)?);
        Some(slot.get_or_init(|| BodyLook::of(&self.doc, b)))
    }

    /// The look of `b`, when it is one of the state's bodies (found by its name).
    pub fn of_body(&self, b: &ModelBody) -> Option<&BodyLook> {
        let by_name = self.by_name.get_or_init(|| self.world.bodies.iter().enumerate().map(|(i, b)| (b.name.clone(), i)).collect());
        let index = *by_name.get(&b.name)?;
        self.world.bodies.get(index).filter(|x| std::ptr::eq(*x, b))?;
        self.at(index)
    }
}
