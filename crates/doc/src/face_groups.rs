//! Faces that are picked as one (#3745 follow-up): a modelled thread is thousands of small faces
//! (its flanks, and the pieces of the surface it was cut into), but it is one surface to point at,
//! light and select; so are the pieces of one face an imported file had (a face wrapping all the
//! way round, split in two for the kernel).
//!
//! The groups come from what made the faces, never from their shape: a face made by a Thread
//! feature is named after it (`F<id>:…`), and a piece of a face the thread cut names it among the
//! faces around it (`…[F<id>:…]`). Working the names out for a large body takes a while, so it is
//! done on a worker thread, once per body, and asked for without waiting.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::{Document, FeatureKind, ModelBody};

/// Each face's group (an index into `groups`), and the groups: faces picked together, in order.
#[derive(Debug, Default)]
pub struct FaceGroups {
    of_face: Vec<Option<u32>>,
    groups: Vec<Arc<Vec<usize>>>,
}

impl FaceGroups {
    /// The group of `face` (`None`: it is picked alone).
    pub fn group_of(&self, face: usize) -> Option<u32> {
        self.of_face.get(face).copied().flatten()
    }

    /// The faces picked together with `face` (`None`: it is picked alone).
    pub fn of(&self, face: usize) -> Option<Arc<Vec<usize>>> {
        let g = self.of_face.get(face).copied().flatten()?;
        self.groups.get(g as usize).cloned()
    }

    /// Work out the groups of `b` (the names of its faces are worked out here: slow for a big body).
    pub fn of_body(threads: &[u64], b: &ModelBody) -> FaceGroups {
        let mut keyed: HashMap<(u8, u64), Vec<usize>> = HashMap::new();
        if !threads.is_empty() {
            let names = crate::naming::face_names(b);
            let marks: Vec<(u64, String, String)> = threads.iter().map(|id| (*id, format!("F{id}:"), format!("[F{id}:"))).collect();
            for (i, name) in names.iter().enumerate() {
                // Made by the thread, or a piece of a face the thread cut (it names the thread's faces
                // around it).
                let own = |p: &str, inside: &str| name.starts_with(p) || name.contains(inside) || name.contains(&format!(",{p}"));
                if let Some((id, _, _)) = marks.iter().find(|(_, p, inside)| own(p, inside)) {
                    keyed.entry((0, *id)).or_default().push(i);
                }
            }
        }
        // Pieces of one face of an imported file.
        if let Some(sources) = b.body.face_sources() {
            for (i, s) in sources.iter().enumerate() {
                if let Some(solidcraft_kernel::FaceSource::Imported { face, .. }) = s
                    && !keyed.values().any(|faces| faces.contains(&i))
                {
                    keyed.entry((1, *face)).or_default().push(i);
                }
            }
        }
        let mut out = FaceGroups { of_face: vec![None; b.body.face_count()], groups: Vec::new() };
        let mut groups: Vec<Vec<usize>> = keyed.into_values().filter(|faces| faces.len() > 1).collect();
        groups.sort();
        for faces in groups {
            let g = u32::try_from(out.groups.len()).unwrap_or(u32::MAX);
            for &f in &faces {
                if let Some(slot) = out.of_face.get_mut(f) {
                    *slot = Some(g);
                }
            }
            out.groups.push(Arc::new(faces));
        }
        out
    }
}

/// The modelled threads of `doc`, by feature id.
pub fn thread_features(doc: &Document) -> Vec<u64> {
    doc.features.iter().filter(|f| matches!(f.kind, FeatureKind::Thread { modeled: true, .. })).map(|f| f.id).collect()
}

/// Groups worked out (or being worked out: `None`), by the identity of a body's names and the
/// threads they were grouped by.
type Cache = HashMap<(usize, Vec<u64>), Option<Arc<FaceGroups>>>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
/// Bodies whose groups are kept; the oldest go first.
const KEPT: usize = 256;

/// The groups of `b` in `doc` if they are worked out; otherwise they are worked out on a worker
/// thread (once) and `None` comes back meanwhile. The browser build has no threads: there they
/// are worked out now.
pub fn face_groups_soon(doc: &Document, b: &ModelBody) -> Option<Arc<FaceGroups>> {
    let threads = thread_features(doc);
    if threads.is_empty() && b.body.face_sources().is_none() {
        return Some(Arc::new(FaceGroups::default()));
    }
    let key = (b.names.id(), threads.clone());
    {
        let Ok(mut cache) = CACHE.lock() else { return None };
        let cache = cache.get_or_insert_with(HashMap::new);
        if let Some(entry) = cache.get(&key) {
            return entry.clone();
        }
        if cache.len() >= KEPT {
            cache.clear();
        }
        cache.insert(key.clone(), None);
    }
    let work = move |b: ModelBody| {
        let groups = Arc::new(FaceGroups::of_body(&threads, &b));
        if let Ok(mut cache) = CACHE.lock()
            && let Some(slot) = cache.as_mut().and_then(|c| c.get_mut(&key))
        {
            *slot = Some(groups.clone());
        }
        groups
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let body = b.clone();
        let work = work.clone();
        if std::thread::Builder::new().name("face-groups".into()).spawn(move || drop(work(body))).is_ok() {
            return None;
        }
    }
    Some(work(b.clone()))
}
