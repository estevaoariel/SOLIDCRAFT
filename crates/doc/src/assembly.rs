//! Components and occurrences: the design as a tree.
//!
//! The root component (id 0) is the design. Every other component lives in a parent component
//! through one or more occurrences, each with a rigid transform (instances share the component's
//! contents; a "Paste New" copy is a separate component). A component's sketches, construction
//! geometry and features are authored in its own frame (its origin); the model is evaluated in
//! those frames and placed in the world through the occurrence transforms when bodies are
//! gathered for display, measuring and export (`world_bodies`). Commands take world coordinates
//! and the engine maps a new feature's geometry into the active component's frame.

use serde::{Deserialize, Serialize};
use solidcraft_geom::{Plane, Vec3};

use crate::{DocError, Document, FeatureKind, PlaneRef, Result};

/// [`Document::world_body_name`] for many bodies of one state: what a name reads from the whole
/// design (occurrences by id and by name, whether any name is reserved, the state's body names,
/// first placements) is gathered once, not again for every body of every placement (#3720).
pub struct WorldNames<'a> {
    doc: &'a Document,
    by_id: std::collections::HashMap<u64, &'a Occurrence>,
    by_name: std::collections::HashMap<&'a str, Vec<&'a Occurrence>>,
    bodies: std::collections::HashSet<&'a str>,
    /// Slash and reserved-marker names could alias concatenated labels or fallback keys.
    reserved: bool,
    first: std::collections::HashMap<u64, Vec<u64>>,
}

impl<'a> WorldNames<'a> {
    pub fn new(doc: &'a Document, st: &'a crate::ModelState) -> Self {
        let mut by_id = std::collections::HashMap::with_capacity(doc.occurrences.len());
        let mut by_name: std::collections::HashMap<&str, Vec<&Occurrence>> = std::collections::HashMap::new();
        for o in &doc.occurrences {
            by_id.entry(o.id).or_insert(o);
            by_name.entry(o.name.as_str()).or_default().push(o);
        }
        let reserved =
            doc.occurrences.iter().any(|o| o.name.contains('/') || o.name.contains("@occ[")) || st.bodies.iter().any(|b| b.name.contains("@occ["));
        WorldNames {
            doc,
            by_id,
            by_name,
            bodies: st.bodies.iter().map(|b| b.name.as_str()).collect(),
            reserved,
            first: std::collections::HashMap::new(),
        }
    }

    /// The world name of body `source` of `component` placed at `path`. Source names retain
    /// priority; ambiguous labels use full IDs. Resolves label ambiguity with bounded dynamic
    /// counts instead of expanding all placements.
    pub fn name(&mut self, source: &str, component: u64, path: &[u64]) -> String {
        let doc = self.doc;
        if path == self.first.entry(component).or_insert_with(|| doc.first_path(component)).as_slice() {
            return source.to_string();
        }
        let names: Vec<&str> = path.iter().filter_map(|id| self.by_id.get(id)).map(|o| o.name.as_str()).collect();
        let label = names.join("/");
        let candidate = format!("{source} ({label})");
        let mut counts = std::collections::BTreeMap::from([(0, 1usize)]);
        for name in &names {
            let mut next = std::collections::BTreeMap::<u64, usize>::new();
            for o in self.by_name.get(name).into_iter().flatten() {
                if let Some(count) = counts.get(&o.parent) {
                    let n = next.entry(o.component).or_default();
                    *n = (*n + count).min(2);
                }
            }
            counts = next;
        }
        let ambiguous = counts.get(&component).copied().unwrap_or(0) > 1;
        if !ambiguous && !self.reserved && !self.bodies.contains(candidate.as_str()) {
            return candidate;
        }
        let ids = path.iter().map(u64::to_string).collect::<Vec<_>>().join("/");
        let encoded = serde_json::to_string(source).unwrap_or_else(|_| source.to_string());
        let mut key = format!("{encoded}@occ[{ids}]");
        while self.bodies.contains(key.as_str()) {
            key.push('#');
        }
        key
    }
}

/// Column-major 4×4 affine transform (`m[col][row]`).
pub type Mat = [[f64; 4]; 4];

pub const IDENTITY: Mat = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

pub fn mat_mul(a: &Mat, b: &Mat) -> Mat {
    let mut r = [[0.0; 4]; 4];
    for (c, col) in r.iter_mut().enumerate() {
        for (row, cell) in col.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[k][row] * b[c][k]).sum();
        }
    }
    r
}

pub fn apply_point(m: &Mat, p: Vec3) -> Vec3 {
    Vec3::new(
        m[0][0] * p.x + m[1][0] * p.y + m[2][0] * p.z + m[3][0],
        m[0][1] * p.x + m[1][1] * p.y + m[2][1] * p.z + m[3][1],
        m[0][2] * p.x + m[1][2] * p.y + m[2][2] * p.z + m[3][2],
    )
}

pub fn apply_vector(m: &Mat, v: Vec3) -> Vec3 {
    Vec3::new(
        m[0][0] * v.x + m[1][0] * v.y + m[2][0] * v.z,
        m[0][1] * v.x + m[1][1] * v.y + m[2][1] * v.z,
        m[0][2] * v.x + m[1][2] * v.y + m[2][2] * v.z,
    )
}

pub fn is_identity(m: &Mat) -> bool {
    m.iter().flatten().zip(IDENTITY.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-12)
}

/// Inverse of an affine transform.
pub fn mat_inverse(m: &Mat) -> Option<Mat> {
    let a = |r: usize, c: usize| m[c][r];
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if det.abs() < 1e-14 || !det.is_finite() {
        return None;
    }
    let inv = |r: usize, c: usize| -> f64 {
        // Cofactor of (c, r) / det.
        let (r0, r1) = match c {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let (c0, c1) = match r {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let minor = a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0);
        let sign = if (r + c).is_multiple_of(2) { 1.0 } else { -1.0 };
        sign * minor / det
    };
    let mut out = IDENTITY;
    for (c, col) in out.iter_mut().enumerate().take(3) {
        for (r, cell) in col.iter_mut().enumerate().take(3) {
            *cell = inv(r, c);
        }
    }
    let t = Vec3::new(m[3][0], m[3][1], m[3][2]);
    let it = apply_vector(&out, t);
    out[3] = [-it.x, -it.y, -it.z, 1.0];
    Some(out)
}

/// A rigid transform: rotation by `angle` about `axis` through `origin`, then a translation.
pub fn rigid(translate: Vec3, origin: Vec3, axis: Vec3, angle: f64) -> Mat {
    let k = axis.normalized().unwrap_or(Vec3::Z);
    let (s, c) = angle.sin_cos();
    let t = 1.0 - c;
    let r = [
        [t * k.x * k.x + c, t * k.x * k.y + s * k.z, t * k.x * k.z - s * k.y],
        [t * k.x * k.y - s * k.z, t * k.y * k.y + c, t * k.y * k.z + s * k.x],
        [t * k.x * k.z + s * k.y, t * k.y * k.z - s * k.x, t * k.z * k.z + c],
    ];
    let mut m = IDENTITY;
    for (mc, rc) in m.iter_mut().zip(r) {
        mc[..3].copy_from_slice(&rc);
    }
    let ro = apply_vector(&m, origin);
    let tr = origin - ro + translate;
    m[3] = [tr.x, tr.y, tr.z, 1.0];
    m
}

/// A placement of a component inside a parent component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Occurrence {
    pub id: u64,
    pub component: u64,
    /// The component it sits in (0 = the root).
    #[serde(default)]
    pub parent: u64,
    pub name: String,
    #[serde(default = "identity")]
    pub transform: Mat,
    /// Grounded occurrences don't move.
    #[serde(default)]
    pub grounded: bool,
}

fn identity() -> Mat {
    IDENTITY
}

/// How deep and how many placements [`Document::placements`] walks.
pub const MAX_PLACEMENT_DEPTH: usize = 64;
pub const MAX_PLACEMENTS: usize = 100_000;

fn map_plane(m: &Mat, p: &Plane) -> Plane {
    let (o, x, y) = (apply_point(m, p.origin), apply_vector(m, p.x), apply_vector(m, p.y));
    Plane::new(o, x, y).unwrap_or(*p)
}

fn map_plane_ref(m: &Mat, p: &mut PlaneRef) {
    match p {
        PlaneRef::Custom { plane } => *plane = map_plane(m, plane),
        PlaneRef::Placed { origin, x_dir, y_dir } => {
            map_point_expr(m, origin);
            *x_dir = apply_vector(m, *x_dir);
            *y_dir = apply_vector(m, *y_dir);
        }
        PlaneRef::Face { plane, at, .. } => {
            *plane = map_plane(m, plane);
            *at = apply_point(m, *at);
        }
        PlaneRef::Offset { base, .. } | PlaneRef::Flipped { base, .. } => map_plane_ref(m, base),
        PlaneRef::AtAngle { base, axis_origin, axis_dir, .. } => {
            map_plane_ref(m, base);
            *axis_origin = apply_point(m, *axis_origin);
            *axis_dir = apply_vector(m, *axis_dir);
        }
        // Named origin planes and construction planes are the component's own.
        PlaneRef::Origin { .. } | PlaneRef::Construction { .. } => {}
        PlaneRef::Tangent { face, at } => {
            *at = apply_point(m, *at);
            map_geo_ref(m, face);
        }
        PlaneRef::Midplane { .. }
        | PlaneRef::TwoEdges { .. }
        | PlaneRef::ThreePoints { .. }
        | PlaneRef::AlongPath { .. }
        | PlaneRef::Perpendicular { .. } => {
            for r in p.refs_mut() {
                map_geo_ref(m, r);
            }
        }
    }
}

fn map_geo_ref(m: &Mat, g: &mut crate::construct::GeoRef) {
    g.map_points(&mut |p: &mut Vec3| *p = apply_point(m, *p), &mut |pl: &mut PlaneRef| map_plane_ref(m, pl));
}

/// Map a point given as expressions by `m`: numbers stay numbers; a coordinate that is an
/// expression stays one (`m · (x, y, z) + t`, written out), so it still follows its parameters.
pub fn map_point_expr(m: &Mat, p: &mut crate::PointExpr) {
    let num = |e: &crate::Expr| e.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    if let [Some(x), Some(y), Some(z)] = [num(&p[0]), num(&p[1]), num(&p[2])] {
        *p = crate::point_expr(apply_point(m, Vec3::new(x, y, z)));
        return;
    }
    let old = p.clone();
    for (row, out) in p.iter_mut().enumerate() {
        let mut constant = m[3][row];
        let mut terms: Vec<String> = Vec::new();
        for (col, e) in old.iter().enumerate() {
            let c = m[col][row];
            if c.abs() < 1e-12 {
                continue;
            }
            match num(e) {
                Some(v) => constant += c * v,
                None if (c - 1.0).abs() < 1e-12 => terms.push(format!("({e})")),
                None => terms.push(format!("{c} * ({e})")),
            }
        }
        let mut e = terms.join(" + ");
        if terms.is_empty() {
            e = format!("{constant} mm");
        } else if constant.abs() >= 1e-12 {
            e = format!("{e} {} {} mm", if constant < 0.0 { '-' } else { '+' }, constant.abs());
        }
        *out = e.into();
    }
}

impl FeatureKind {
    /// Map the feature's world geometry (points, directions, explicit planes) by `m`.
    pub fn transform_geometry(&mut self, m: &Mat) {
        use crate::feature_geometry::Geo;
        use crate::walk::Walk;
        self.walk_geometry_mut(&mut |g| match g {
            Geo::Pick(p) | Geo::Pos(p) => *p = apply_point(m, *p),
            Geo::Dir(v) => *v = apply_vector(m, *v),
            Geo::Plane(p) => map_plane_ref(m, p),
            Geo::Construct(r) => map_geo_ref(m, r),
            Geo::Placement(e) => map_point_expr(m, e),
            // An Emboss cylinder is picked in its component's frame (`EmbossFace`).
            Geo::Pinned(_) | Geo::Profiles(..) => {}
        });
    }
}

impl Document {
    /// The first occurrence of a component (its placement for authoring).
    pub fn occurrence_of(&self, component: u64) -> Option<&crate::Occurrence> {
        self.occurrences.iter().find(|o| o.component == component)
    }

    /// World transform of a component's frame (through its first occurrence and its parents').
    pub fn component_transform(&self, component: u64) -> Mat {
        let mut m = IDENTITY;
        let mut c = component;
        for _ in 0..1000 {
            if c == 0 {
                break;
            }
            let Some(o) = self.occurrence_of(c) else { break };
            m = mat_mul(&o.transform, &m);
            c = o.parent;
        }
        m
    }

    /// One placement: the component the occurrence `path` from the root leads to (the root for
    /// an empty path) and its world transform. `None` when the path no longer exists.
    pub fn path_placement(&self, path: &[u64]) -> Option<(u64, Mat)> {
        if path.len() > MAX_PLACEMENT_DEPTH + 1 {
            return None;
        }
        let (mut m, mut c) = (IDENTITY, 0);
        for id in path {
            let o = self.occurrences.iter().find(|o| o.id == *id && o.parent == c)?;
            m = mat_mul(&m, &o.transform);
            c = o.component;
        }
        Some((c, m))
    }

    /// Every placement of each component: (component, occurrence path ids, world transform).
    /// A branch stops past [`MAX_PLACEMENT_DEPTH`] levels, the walk past [`MAX_PLACEMENTS`] placements.
    pub fn placements(&self) -> Vec<(u64, Vec<u64>, Mat)> {
        self.placements_with(&std::collections::BTreeMap::new())
    }

    /// [`Document::placements`] with `moves` (by occurrence id) in place of those occurrences'
    /// own transforms: moves not captured yet, as a drag leaves them.
    pub fn placements_with(&self, moves: &std::collections::BTreeMap<u64, Mat>) -> Vec<(u64, Vec<u64>, Mat)> {
        // Each component's occurrences, in the design's order, found once rather than by a scan
        // of every occurrence for every placement.
        let mut children: std::collections::HashMap<u64, Vec<&Occurrence>> = std::collections::HashMap::new();
        for o in &self.occurrences {
            children.entry(o.parent).or_default().push(o);
        }
        let mut out = vec![(0, Vec::new(), IDENTITY)];
        let mut stack: Vec<(u64, Vec<u64>, Mat)> = vec![(0, Vec::new(), IDENTITY)];
        while let Some((comp, path, m)) = stack.pop() {
            if out.len() > MAX_PLACEMENTS {
                break;
            }
            // Only this branch stops past the depth limit; the others are still walked.
            if path.len() > MAX_PLACEMENT_DEPTH {
                continue;
            }
            for o in children.get(&comp).into_iter().flatten() {
                let mut p = path.clone();
                p.push(o.id);
                let w = mat_mul(&m, moves.get(&o.id).unwrap_or(&o.transform));
                out.push((o.component, p.clone(), w));
                stack.push((o.component, p, w));
            }
        }
        out
    }

    /// Every body as the world shows it: its world name ("Body1", or "Body1 (Occ:2)" for a
    /// further instance), its component, the occurrence path, and the transform from the frame
    /// its geometry is in to the world.
    pub fn body_frames(&self, st: &crate::ModelState) -> Vec<(String, u64, Vec<u64>, Mat)> {
        self.body_frames_with_source(st).into_iter().map(|(world, _, component, path, matrix)| (world, component, path, matrix)).collect()
    }

    /// Body frames with explicit definition names; callers must not infer them from labels.
    pub fn body_frames_with_source(&self, st: &crate::ModelState) -> Vec<(String, String, u64, Vec<u64>, Mat)> {
        let mut out = Vec::new();
        let owned = self.bodies_by_component(st);
        let mut names = WorldNames::new(self, st);
        for (comp, path, m) in self.placements() {
            for b in owned.get(&comp).into_iter().flatten() {
                let name = names.name(&b.name, comp, &path);
                let w = match self.model_body_offset(b) {
                    Some(o) => mat_mul(&m, o),
                    None => m,
                };
                out.push((name, b.definition_name().to_string(), comp, path.clone(), w));
            }
        }
        out
    }

    /// Computed placement name. Source names retain priority; ambiguous labels use full IDs.
    /// Resolves label ambiguity with bounded dynamic counts instead of expanding all placements.
    pub fn world_body_name(&self, st: &crate::ModelState, source: &str, component: u64, path: &[u64]) -> String {
        WorldNames::new(self, st).name(source, component, path)
    }

    /// The bodies of `st` by the component that owns them ([`Document::model_body_component`]),
    /// each in the state's order, with the timeline looked up once rather than once per body.
    pub fn bodies_by_component<'a>(&self, st: &'a crate::ModelState) -> std::collections::HashMap<u64, Vec<&'a crate::ModelBody>> {
        self.body_indices_by_component(st).into_iter().map(|(c, bodies)| (c, bodies.into_iter().filter_map(|i| st.bodies.get(i)).collect())).collect()
    }

    /// [`Document::bodies_by_component`], each body by its index in `st.bodies`.
    pub fn body_indices_by_component(&self, st: &crate::ModelState) -> std::collections::HashMap<u64, Vec<usize>> {
        let features: std::collections::HashMap<u64, u64> = self.features.iter().rev().map(|f| (f.id, f.component)).collect();
        let mut out: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
        for (i, b) in st.bodies.iter().enumerate() {
            let component = if b.transferred || b.source_name.is_some() {
                b.component
            } else {
                self.body_components.get(&b.name).copied().unwrap_or_else(|| features.get(&b.feature).copied().unwrap_or(0))
            };
            out.entry(component).or_default().push(i);
        }
        out
    }

    /// The occurrence path of a component's first placement.
    pub fn first_path(&self, component: u64) -> Vec<u64> {
        let mut path = Vec::new();
        let mut c = component;
        for _ in 0..1000 {
            if c == 0 {
                break;
            }
            let Some(o) = self.occurrence_of(c) else { break };
            path.insert(0, o.id);
            c = o.parent;
        }
        path
    }

    /// Is `a` inside `b` (or `b` itself)?
    pub fn component_within(&self, a: u64, b: u64) -> bool {
        let mut c = a;
        for _ in 0..1000 {
            if c == b {
                return true;
            }
            if c == 0 {
                return false;
            }
            match self.components.iter().find(|x| x.id == c) {
                Some(x) => c = x.parent,
                None => return false,
            }
        }
        false
    }

    /// Add an occurrence of an existing component into `parent`.
    pub fn add_occurrence(&mut self, component: u64, parent: u64, transform: Mat) -> Result<u64> {
        if component == 0 || !self.components.iter().any(|c| c.id == component) {
            return Err(DocError::Unknown(format!("component {component}")));
        }
        if self.component_within(parent, component) {
            return Err(DocError::Invalid("a component can't contain itself".into()));
        }
        if self.occurrences.len() >= 100_000 {
            return Err(DocError::Invalid("too many occurrences".into()));
        }
        let id = self.next_id.max(1);
        self.next_id = id.checked_add(1).ok_or_else(|| DocError::Invalid("too many items in the design".into()))?;
        let cname = self.components.iter().find(|c| c.id == component).map(|c| c.name.clone()).unwrap_or_default();
        let k = self.occurrences.iter().filter(|o| o.component == component).count() + 1;
        self.occurrences.push(crate::Occurrence { id, component, parent, name: format!("{cname}:{k}"), transform, grounded: false });
        Ok(id)
    }
}
