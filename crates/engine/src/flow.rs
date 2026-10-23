//! Airflow studies in a session: their setups (kept beside the design, never in it), their runs
//! on a background thread, and their results. The solver comes from the session's flow backend
//! (the app installs the GPU one; without it the CPU reference runs, for small grids).
//!
//! Openings are picked as faces (a point on the face, as other commands take faces):
//! - a duct's open end or slot is picked by its rim face (a face with a hole in it): the opening is
//!   the hole, and an inlet's air goes into the duct, opposite the rim's outward normal;
//! - a face without holes (a fan's outlet face) is the opening itself, its air leaving along the
//!   face's outward normal.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use solidcraft_doc::ModelState;
use solidcraft_geom::{Mesh, Vec2, Vec3};
use solidcraft_sim as sim;

/// The solver studies run on.
pub type FlowBackend = Arc<dyn sim::Backend>;

/// Most cells a study may have on the CPU reference (it is slow), and on any backend.
pub const CPU_MAX_CELLS: u64 = 2_000_000;
pub const MAX_CELLS: u64 = 40_000_000;

/// Most cells a study may have on `backend`: what its memory holds where it knows (a GPU), so a
/// card with less memory gets coarser cells instead of failing to start.
pub fn max_cells(backend: Option<&FlowBackend>) -> u64 {
    match backend {
        Some(b) => b.max_cells().unwrap_or(MAX_CELLS).min(MAX_CELLS),
        None => CPU_MAX_CELLS,
    }
}

#[derive(Default)]
pub struct Studies {
    pub(crate) backend: Option<FlowBackend>,
    pub list: Vec<Study>,
    /// Goal seeks over the studies' design parameters.
    pub seeks: Vec<crate::seek::Seek>,
}

impl Studies {
    /// For a scratch session: the same backend, no studies.
    pub(crate) fn scratch(&self) -> Studies {
        Studies { backend: self.backend.clone(), list: Vec::new(), seeks: Vec::new() }
    }

    pub fn get(&self, name: &str) -> Option<&Study> {
        self.list.iter().find(|s| s.name == name)
    }

    pub(crate) fn get_mut(&mut self, name: &str) -> Option<&mut Study> {
        self.list.iter_mut().find(|s| s.name == name)
    }

    pub fn names(&self) -> Vec<String> {
        self.list.iter().map(|s| s.name.clone()).collect()
    }
}

/// How an opening's region is taken from its face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// The hole(s) in the face (a duct's rim).
    Hole,
    /// The face itself (a fan's outlet face).
    Face,
}

/// An inlet or outlet as picked.
#[derive(Clone, Debug, PartialEq)]
pub struct OpeningSpec {
    pub name: String,
    pub body: String,
    /// A point on the face, mm.
    pub point: Vec3,
    pub region: Option<Region>,
    /// A section across the duct instead of a face: the plane through `point` with this normal
    /// (the way the air goes), cutting the body; the opening is the passage there around `point`.
    pub section: Option<Vec3>,
    /// Inlets only.
    pub source: Option<sim::Source>,
}

/// A study as set up (all lengths mm).
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub body: String,
    /// The bodies in the air's way (the duct's body among them).
    pub bodies: Vec<String>,
    pub inlets: Vec<OpeningSpec>,
    pub outlets: Vec<OpeningSpec>,
    /// None: chosen from the openings' sizes.
    pub cell: Option<f64>,
    pub pad: f64,
    pub turbulence: bool,
    /// The walls' equivalent sand-grain roughness, mm (0: smooth).
    pub roughness: f64,
    pub max_steps: u64,
}

/// Surface finishes as equivalent sand-grain roughness, mm: smooth (machined, moulded or resin
/// printed walls, whose roughness is far inside the viscous layer) and rough, about three times
/// the arithmetic mean roughness Ra commonly reported for an FDM print's walls (a rough guide; a
/// measured Ra gives a better one, as `roughness_mm`).
pub const SURFACES: [(&str, f64); 2] = [("smooth", 0.0), ("rough", 0.06)];

pub struct Study {
    pub name: String,
    pub spec: Spec,
    pub job: Option<Job>,
    pub result: Option<Arc<FlowResult>>,
    /// Bumped whenever the result changes, so views know to redraw.
    pub revision: u64,
}

/// A finished run.
pub struct FlowResult {
    pub fields: sim::Fields,
    /// The measurements averaged over the end of the run.
    pub average: sim::Sample,
    pub history: Vec<sim::Sample>,
    pub state: sim::State,
    pub steps: u64,
    pub backend: String,
    pub warnings: Vec<sim::Warning>,
    pub inlets: Vec<String>,
    pub outlets: Vec<String>,
    /// Per inlet: its final speed, m/s (a fan's where it settled on its curve).
    pub speeds: Vec<f64>,
    pub units: sim::Units,
    pub cell: f64,
    pub seconds: f64,
    /// Where each inlet's air starts, mm: its cells' centres (streamline seeds).
    pub seeds: Vec<Vec<Vec3>>,
}

/// What a running study shares with the session.
#[derive(Default)]
pub struct Progress {
    pub steps: u64,
    pub max_steps: u64,
    pub last: Option<sim::Sample>,
    pub backend: String,
    pub cells: usize,
    pub done: Option<std::result::Result<Arc<FlowResult>, String>>,
}

pub struct Job {
    pub progress: Arc<Mutex<Progress>>,
    pub cancel: Arc<AtomicBool>,
}

impl Job {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The face of `body` nearest `point`, its triangles' outward normal (area weighted) and how flat
/// it is.
fn face_of(st: &ModelState, body: &str, point: Vec3) -> std::result::Result<(Arc<Mesh>, u32), String> {
    let mb = st.body(body).ok_or_else(|| format!("no body `{body}`"))?;
    let face = solidcraft_doc::appearance::face_index_at(mb, point).ok_or_else(|| format!("no face of `{body}` at {point:?}"))?;
    Ok((mb.mesh(), u32::try_from(face).map_err(|_| "face index out of range".to_string())?))
}

/// Polylines (the face's edges) chained into closed loops.
fn loops(edges: Vec<&Vec<Vec3>>, tol: f64) -> Vec<Vec<Vec3>> {
    let mut left: Vec<Vec<Vec3>> = edges.into_iter().filter(|e| e.len() >= 2).cloned().collect();
    let mut out = Vec::new();
    while let Some(mut chain) = left.pop() {
        // A closed edge on its own (a circle).
        let closed = |c: &Vec<Vec3>| matches!((c.first(), c.last()), (Some(a), Some(b)) if a.dist(*b) <= tol);
        let mut guard = 0;
        while !closed(&chain) && guard < 10_000 {
            guard += 1;
            let Some(end) = chain.last().copied() else { break };
            let Some(i) = left.iter().position(|e| e.first().is_some_and(|p| p.dist(end) <= tol) || e.last().is_some_and(|p| p.dist(end) <= tol))
            else {
                break;
            };
            let mut next = left.swap_remove(i);
            if next.first().is_some_and(|p| p.dist(end) > tol) {
                next.reverse();
            }
            chain.extend(next.into_iter().skip(1));
        }
        if closed(&chain) {
            chain.pop();
            if chain.len() >= 3 {
                out.push(chain);
            }
        }
    }
    out
}

/// The flat piece of a mesh body's surface (an STL comes in as one face of loose triangles) around
/// the triangle nearest `point`: every triangle joined to it across shared edges in the same plane,
/// so a mesh's flat rim or fan mouth is picked whole. Its triangles and its outline's edges.
fn mesh_flat_piece(mesh: &Mesh, face: u32, point: Vec3, size: f64) -> Option<(Vec<[Vec3; 3]>, Vec<Vec<Vec3>>)> {
    let tris: Vec<(usize, [Vec3; 3])> = mesh
        .triangles
        .iter()
        .zip(&mesh.tri_face)
        .enumerate()
        .filter(|(_, (_, f))| **f == face)
        .filter_map(|(i, (t, _))| Some((i, mesh.tri(t)?)))
        .collect();
    let seed = tris
        .iter()
        .map(|(i, [a, b, c])| (*i, solidcraft_geom::compare::closest_on_triangle(point, *a, *b, *c).dist(point)))
        .fold(None, |best: Option<(usize, f64)>, x| if best.is_none_or(|b| x.1 < b.1) { Some(x) } else { best })?
        .0;
    let tri = |i: usize| mesh.triangles.get(i).and_then(|t| mesh.tri(t));
    let [a, b, c] = tri(seed)?;
    let n = (b - a).cross(c - a).normalized()?;
    // Triangles by shared edge: vertices welded at a ten-thousandth of the part's size.
    let q = 1e-4 * size;
    let key = |v: Vec3| [(v.x / q).round() as i64, (v.y / q).round() as i64, (v.z / q).round() as i64];
    let edge = |u: Vec3, v: Vec3| {
        let (x, y) = (key(u), key(v));
        if x <= y { (x, y) } else { (y, x) }
    };
    let mut by_edge: std::collections::HashMap<([i64; 3], [i64; 3]), Vec<usize>> = std::collections::HashMap::new();
    for (i, [a, b, c]) in &tris {
        for (u, v) in [(*a, *b), (*b, *c), (*c, *a)] {
            by_edge.entry(edge(u, v)).or_default().push(*i);
        }
    }
    let tol = 1e-3 * size + 1e-4;
    let mut inside = std::collections::HashSet::from([seed]);
    let mut stack = vec![seed];
    while let Some(i) = stack.pop() {
        let Some([a, b, c]) = tri(i) else { continue };
        for (u, v) in [(a, b), (b, c), (c, a)] {
            for j in by_edge.get(&edge(u, v)).into_iter().flatten() {
                if inside.contains(j) {
                    continue;
                }
                let Some([x, y, z]) = tri(*j) else { continue };
                let flat = (y - x).cross(z - x).normalized().is_some_and(|m| m.dot(n) > 0.9986);
                if flat && (x - a).dot(n).abs() <= tol {
                    inside.insert(*j);
                    stack.push(*j);
                }
            }
        }
    }
    let piece: Vec<[Vec3; 3]> = inside.iter().filter_map(|i| tri(*i)).collect();
    // The outline: edges with exactly one triangle of the piece on them.
    let mut count: std::collections::HashMap<([i64; 3], [i64; 3]), (usize, Vec3, Vec3)> = std::collections::HashMap::new();
    for [a, b, c] in &piece {
        for (u, v) in [(*a, *b), (*b, *c), (*c, *a)] {
            count.entry(edge(u, v)).or_insert((0, u, v)).0 += 1;
        }
    }
    let outline = count.into_values().filter(|(k, _, _)| *k == 1).map(|(_, u, v)| vec![u, v]).collect();
    Some((piece, outline))
}

/// The passage's cross-section where the plane through `point` with normal `n` cuts `mesh`: the
/// smallest loop of the cut around `point` (the duct's inner wall there), less any loops inside it
/// (a hub, a strut), as triangles.
fn section_piece(mesh: &Mesh, point: Vec3, n: Vec3, size: f64) -> std::result::Result<Vec<[Vec3; 3]>, String> {
    let d = |p: Vec3| (p - point).dot(n);
    let mut segments: Vec<Vec<Vec3>> = Vec::new();
    for t in &mesh.triangles {
        let Some(v) = mesh.tri(t) else { continue };
        let mut cut = Vec::new();
        for i in 0..3 {
            let (a, b) = (v[i], v[(i + 1) % 3]);
            let (da, db) = (d(a), d(b));
            if (da >= 0.0) != (db >= 0.0) {
                cut.push(a + (b - a) * (da / (da - db)));
            }
        }
        if let [a, b] = cut[..]
            && a.dist(b) > 1e-9 * size
        {
            segments.push(vec![a, b]);
        }
        if segments.len() > 2_000_000 {
            return Err("the section cuts too many triangles".into());
        }
    }
    let rings = loops(segments.iter().collect(), 1e-6 * size + 1e-9);
    let u = n.any_perp();
    let v = n.cross(u);
    let flat2 = |p: Vec3| Vec2::new((p - point).dot(u), (p - point).dot(v));
    let lift = |q: Vec2| point + u * q.x + v * q.y;
    let mut flat: Vec<solidcraft_geom::Loop2> = rings
        .iter()
        .map(|r| {
            let l = solidcraft_geom::Loop2::polygon(&r.iter().map(|p| flat2(*p)).collect::<Vec<_>>());
            if l.signed_area() < 0.0 { l.reversed() } else { l }
        })
        .collect();
    flat.sort_by(|a, b| a.signed_area().total_cmp(&b.signed_area()));
    let here = Vec2::new(0.0, 0.0);
    // In the air inside a duct an even number of the cut's loops hold the point (the duct's
    // outside and inside walls); in the wall an odd number, outside the duct none.
    let around = flat.iter().filter(|l| l.contains(here)).count();
    let Some(oi) = flat.iter().position(|l| l.contains(here)).filter(|_| around >= 2 && around % 2 == 0) else {
        return Err("the section's point is not inside the duct there".into());
    };
    let outer = flat.swap_remove(oi);
    // Islands: loops inside the passage that don't hold the point.
    let holes: Vec<solidcraft_geom::Loop2> = flat
        .into_iter()
        .filter(|l| l.signed_area() < outer.signed_area() && !l.contains(here) && l.polyline(1e-3).first().is_some_and(|p| outer.contains(*p)))
        .map(|l| l.reversed())
        .collect();
    let r = solidcraft_geom::Region2 { outer, holes };
    Ok(r.triangulate(1e-3 * size).into_iter().map(|t| t.map(lift)).collect())
}

/// The patch an opening covers: its triangles, the normal its air goes along, and whether the cells
/// behind it are to be closed (an inlet at a duct's open end).
pub fn patch(st: &ModelState, o: &OpeningSpec, inlet: bool) -> std::result::Result<(sim::Patch, bool), String> {
    if let Some(normal) = o.section {
        let mesh = st.body(&o.body).ok_or_else(|| format!("no body `{}`", o.body))?.mesh();
        let size = mesh.bounds().max.dist(mesh.bounds().min).max(1.0);
        let n = normal.normalized().ok_or_else(|| format!("the section of `{}` needs a normal", o.name))?;
        let triangles = section_piece(&mesh, o.point, n, size).map_err(|e| format!("`{}`: {e}", o.name))?;
        if triangles.is_empty() {
            return Err(format!("the opening `{}` covers nothing", o.name));
        }
        // An inlet's section closes the duct behind it, where the fan sits.
        return Ok((sim::Patch { triangles, normal: n }, inlet));
    }
    let (mesh, face) = face_of(st, &o.body, o.point)?;
    let size = mesh.bounds().max.dist(mesh.bounds().min).max(1.0);
    let mut tris = Vec::new();
    let mut normal = Vec3::ZERO;
    for (t, f) in mesh.triangles.iter().zip(&mesh.tri_face) {
        if *f != face {
            continue;
        }
        let Some([a, b, c]) = mesh.tri(t) else { continue };
        normal += (b - a).cross(c - a);
        tris.push([a, b, c]);
    }
    // Flat: every triangle faces within about 3° of the mean.
    let flat = |tris: &[[Vec3; 3]], n: Vec3| tris.iter().all(|[a, b, c]| (*b - *a).cross(*c - *a).normalized().is_none_or(|t| t.dot(n) > 0.9986));
    let mut edges: Vec<Vec<Vec3>> = mesh.edges.iter().zip(&mesh.edge_faces).filter(|(_, fs)| fs.contains(&face)).map(|(e, _)| e.clone()).collect();
    let n = match normal.normalized() {
        Some(n) if flat(&tris, n) => n,
        // A mesh body's one face of loose triangles: the flat piece of it where the point is.
        _ if edges.is_empty() => {
            let (piece, outline) = mesh_flat_piece(&mesh, face, o.point, size).ok_or_else(|| format!("the face `{}` picked is empty", o.name))?;
            let n = piece
                .iter()
                .fold(Vec3::ZERO, |n, [a, b, c]| n + (*b - *a).cross(*c - *a))
                .normalized()
                .ok_or_else(|| format!("the face `{}` picked is empty", o.name))?;
            tris = piece;
            edges = outline;
            n
        }
        Some(_) => return Err(format!("the opening `{}` must be a flat face", o.name)),
        None => return Err(format!("the face `{}` picked is empty", o.name)),
    };
    let edges: Vec<&Vec<Vec3>> = edges.iter().collect();
    let rings = loops(edges, 1e-6 * size + 1e-6);
    let region = o.region.unwrap_or(if rings.len() >= 2 { Region::Hole } else { Region::Face });
    let origin = tris.first().map_or(Vec3::ZERO, |t| t[0]);
    let u = n.any_perp();
    let v = n.cross(u);
    let flat2 = |p: Vec3| Vec2::new((p - origin).dot(u), (p - origin).dot(v));
    let lift = |q: Vec2| origin + u * q.x + v * q.y;
    let area = |l: &[Vec3]| {
        let pts: Vec<Vec2> = l.iter().map(|p| flat2(*p)).collect();
        let mut a = 0.0;
        for (i, p) in pts.iter().enumerate() {
            let q = pts.get((i + 1) % pts.len()).copied().unwrap_or(*p);
            a += p.x * q.y - q.x * p.y;
        }
        0.5 * a
    };
    let triangles = match region {
        Region::Face => tris,
        Region::Hole => {
            if rings.len() < 2 {
                return Err(format!("the face picked for `{}` has no hole in it; pick the rim around the opening", o.name));
            }
            // The outer loop is the largest; every other loop is a hole, but for holes under a
            // tenth of the largest one (a fan flange's screw holes, which go through to the
            // outside, not into the duct).
            let outer = rings.iter().enumerate().map(|(i, l)| (i, area(l).abs())).fold((0, 0.0), |best, x| if x.1 > best.1 { x } else { best }).0;
            let biggest = rings.iter().enumerate().filter(|(i, _)| *i != outer).map(|(_, l)| area(l).abs()).fold(0.0, f64::max);
            let mut out = Vec::new();
            for (i, ring) in rings.iter().enumerate() {
                if i == outer || area(ring).abs() < 0.1 * biggest {
                    continue;
                }
                let pts: Vec<Vec2> = ring.iter().map(|p| flat2(*p)).collect();
                let mut l = solidcraft_geom::Loop2::polygon(&pts);
                if area(ring) < 0.0 {
                    l = l.reversed();
                }
                let r = solidcraft_geom::Region2 { outer: l, holes: Vec::new() };
                out.extend(r.triangulate(1e-3 * size).into_iter().map(|t| t.map(lift)));
            }
            out
        }
    };
    if triangles.is_empty() {
        return Err(format!("the opening `{}` covers nothing", o.name));
    }
    // An inlet's air goes into the duct through a hole (against the rim's outward normal) or out
    // of a fan's face (along it); an outlet's air leaves along the rim's or face's normal.
    let along = if inlet && region == Region::Hole { n * -1.0 } else { n };
    Ok((sim::Patch { triangles, normal: along }, inlet && region == Region::Hole))
}

/// Hydraulic diameter of a patch (4 A / P), mm, from its triangles' area and outline.
fn width(p: &sim::Patch) -> f64 {
    let area = p.area();
    // The outline: edges used by one triangle only.
    let key = |a: Vec3, b: Vec3| {
        let r = |v: Vec3| [(v.x * 1e4).round() as i64, (v.y * 1e4).round() as i64, (v.z * 1e4).round() as i64];
        let (x, y) = (r(a), r(b));
        if x <= y { (x, y) } else { (y, x) }
    };
    let mut count: std::collections::HashMap<_, (usize, f64)> = std::collections::HashMap::new();
    for [a, b, c] in &p.triangles {
        for (x, y) in [(*a, *b), (*b, *c), (*c, *a)] {
            let e = count.entry(key(x, y)).or_insert((0, x.dist(y)));
            e.0 += 1;
        }
    }
    let perimeter: f64 = count.values().filter(|(n, _)| *n == 1).map(|(_, l)| l).sum();
    if perimeter > 0.0 { 4.0 * area / perimeter } else { area.sqrt() }
}

/// The study's setup for the solver: bodies and patches from the design as it is now.
pub fn setup(st: &ModelState, spec: &Spec, max_cells: u64) -> std::result::Result<(sim::FlowSetup, Vec<Arc<Mesh>>), String> {
    let mut meshes = Vec::new();
    for b in &spec.bodies {
        meshes.push(st.body(b).ok_or_else(|| format!("no body `{b}`"))?.mesh());
    }
    let mut inlets = Vec::new();
    for o in &spec.inlets {
        let (patch, cap) = patch(st, o, true)?;
        let source = o.source.clone().ok_or_else(|| format!("the inlet `{}` needs a speed or a fan", o.name))?;
        inlets.push(sim::Inlet { name: o.name.clone(), patch, source, cap });
    }
    let mut outlets = Vec::new();
    for o in &spec.outlets {
        let (patch, _) = patch(st, o, false)?;
        outlets.push(sim::Outlet { name: o.name.clone(), patch });
    }
    let cell = match spec.cell {
        Some(c) => c,
        None => {
            // Eight cells across the narrowest opening and no larger than the thinner walls are
            // thick (or the walls come out a cell thick, and passages beside them narrower), then
            // larger if the grid would be too big.
            let narrow = inlets.iter().map(|i| width(&i.patch)).chain(outlets.iter().map(|o| width(&o.patch))).fold(f64::INFINITY, f64::min);
            let refs: Vec<&Mesh> = meshes.iter().map(|m| m.as_ref()).collect();
            let wall = sim::study::wall_thickness(&refs).unwrap_or(f64::INFINITY);
            let mut c = (narrow / 8.0).min(wall).clamp(0.2, 5.0);
            let b = meshes.iter().map(|m| m.bounds()).reduce(|a, b| solidcraft_geom::Aabb3 { min: a.min.min(b.min), max: a.max.max(b.max) });
            if let Some(b) = b {
                let size = b.max - b.min + Vec3::new(2.0, 2.0, 2.0) * spec.pad;
                let volume = size.x.max(c) * size.y.max(c) * size.z.max(c);
                let cells = volume / (c * c * c);
                if cells > max_cells as f64 * 0.9 {
                    c = (volume / (max_cells as f64 * 0.9)).cbrt();
                }
            }
            // A tidy number: two significant figures.
            let scale = 10f64.powf(c.log10().floor() - 1.0);
            (c / scale).ceil() * scale
        }
    };
    let setup = sim::FlowSetup {
        cell,
        pad: spec.pad,
        air: sim::AIR,
        inlets,
        outlets,
        turbulence: if spec.turbulence { 0.14 } else { 0.0 },
        roughness: spec.roughness,
        max_cells,
    };
    Ok((setup, meshes))
}

/// Set `spec` up on the model `st` and start it: warm from `warm`'s flow where given (no soft
/// start then). Returns the job and the grid's cell count.
#[allow(clippy::too_many_arguments)]
pub fn launch(
    st: &ModelState,
    spec: &Spec,
    backend: FlowBackend,
    max_cells: u64,
    warm: Option<&FlowResult>,
    max_steps: u64,
    tolerance: f64,
    wait: bool,
) -> std::result::Result<(Job, usize), String> {
    let (setup, meshes) = setup(st, spec, max_cells)?;
    let refs: Vec<&Mesh> = meshes.iter().map(|m| m.as_ref()).collect();
    let mut prepared = sim::prepare(&refs, &setup).map_err(|e| e.to_string())?;
    drop(refs);
    let warmed = warm.map(|w| prepared.warm_from(&w.fields)).unwrap_or(0) > 0;
    // Measure about sixteen times per crossing of the box, at least every 50 steps.
    let longest = prepared.grid.dims.iter().copied().max().unwrap_or(1) as f64;
    let check_every = ((longest / sim::study::LATTICE_SPEED) / 16.0).clamp(50.0, 2000.0) as u32;
    let limits = sim::Limits { max_steps, check_every, tolerance, soft_start: !warmed };
    let names = (spec.inlets.iter().map(|o| o.name.clone()).collect(), spec.outlets.iter().map(|o| o.name.clone()).collect());
    let cells = prepared.grid.len();
    Ok((start(prepared, backend, limits, names, wait, Some(Remake { meshes, setup })), cells))
}

/// Run `prepared` to the end on `backend`, reporting into `progress`.
fn run_to_end(
    prepared: sim::Prepared,
    backend: &dyn sim::Backend,
    limits: sim::Limits,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
    names: (Vec<String>, Vec<String>),
) -> std::result::Result<Arc<FlowResult>, Failed> {
    let clock = web_time::Instant::now();
    let seeds: Vec<Vec<Vec3>> = prepared
        .inlets
        .iter()
        .map(|l| {
            l.cells
                .iter()
                .map(|c| {
                    let [i, j, k] = prepared.grid.coords(*c);
                    prepared.grid.center(i, j, k)
                })
                .collect()
        })
        .collect();
    let warnings = prepared.warnings.clone();
    let mut run = sim::Run::new(prepared, backend, limits).map_err(|e| Failed::Other(e.to_string()))?;
    if let Ok(mut p) = progress.lock() {
        p.backend = run.backend.clone();
        p.max_steps = limits.max_steps;
        p.cells = run.prepared.grid.len();
    }
    let state = run
        .finish(|r| {
            if let Ok(mut p) = progress.lock() {
                p.steps = r.steps;
                p.last = r.last().cloned();
            }
            !cancel.load(Ordering::Relaxed)
        })
        .map_err(|e| match e {
            sim::SimError::Unstable { .. } => Failed::Unstable(e.to_string()),
            e => Failed::Other(e.to_string()),
        })?;
    let average = run.average().ok_or_else(|| Failed::Other("the run measured nothing".into()))?;
    Ok(Arc::new(FlowResult {
        fields: run.fields(),
        average,
        history: run.history.clone(),
        state,
        steps: run.steps,
        backend: run.backend.clone(),
        warnings,
        inlets: names.0,
        outlets: names.1,
        speeds: run.prepared.speeds.clone(),
        units: run.prepared.units,
        cell: run.prepared.grid.spacing,
        seconds: clock.elapsed().as_secs_f64(),
        seeds,
    }))
}

/// Start `prepared` on a thread of its own (or here, where no thread can start or `wait`).
/// What a run needs to set its lattice up again, with a shorter time step, when it goes unstable.
#[derive(Clone)]
pub struct Remake {
    pub meshes: Vec<Arc<Mesh>>,
    pub setup: sim::FlowSetup,
}

/// Why a run stopped short.
enum Failed {
    Unstable(String),
    Other(String),
}

/// Most a rerun shortens the time step: up to three reruns, at 2, 4 and 8 times shorter.
const MAX_PACE: f64 = 8.0;

pub fn start(
    prepared: sim::Prepared,
    backend: FlowBackend,
    limits: sim::Limits,
    names: (Vec<String>, Vec<String>),
    wait: bool,
    remake: Option<Remake>,
) -> Job {
    let progress = Arc::new(Mutex::new(Progress::default()));
    let cancel = Arc::new(AtomicBool::new(false));
    let job = Job { progress: progress.clone(), cancel: cancel.clone() };
    let work = move |prepared: sim::Prepared| {
        let attempt = || {
            let (mut prepared, mut limits, mut pace) = (prepared, limits, 1.0);
            loop {
                match run_to_end(prepared, backend.as_ref(), limits, &progress, &cancel, names.clone()) {
                    Ok(r) => return Ok(r),
                    Err(Failed::Other(e)) => return Err(e),
                    // Unstable: set up again with steps twice as short, from rest.
                    Err(Failed::Unstable(e)) => {
                        let Some(again) = remake.as_ref().filter(|_| pace < MAX_PACE && !cancel.load(Ordering::Relaxed)) else { return Err(e) };
                        pace *= 2.0;
                        let refs: Vec<&Mesh> = again.meshes.iter().map(|m| m.as_ref()).collect();
                        let Ok(mut p) = sim::prepare_paced(&refs, &again.setup, pace) else { return Err(e) };
                        p.warnings.push(sim::Warning::ShortSteps { pace });
                        limits.soft_start = true;
                        prepared = p;
                    }
                }
            }
        };
        let done =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(attempt)).unwrap_or_else(|_| Err("internal error in the flow solver".into()));
        if let Ok(mut p) = progress.lock() {
            p.done = Some(done);
        }
    };
    // The prepared lattice moves into the thread; if no thread starts it comes back to run here.
    let cell = Arc::new(Mutex::new(Some(prepared)));
    let spawned = if wait {
        None
    } else {
        let (cell, work) = (cell.clone(), work.clone());
        std::thread::Builder::new()
            .name("solidcraft-flow".into())
            .spawn(move || {
                if let Some(p) = cell.lock().ok().and_then(|mut c| c.take()) {
                    work(p);
                }
            })
            .ok()
    };
    if spawned.is_none()
        && let Some(p) = cell.lock().ok().and_then(|mut c| c.take())
    {
        work(p);
    }
    job
}

/// A study's state as `sim.flow.status` reports it.
pub fn status(study: &Study) -> Value {
    let Some(job) = &study.job else {
        return json!({"study": study.name, "state": if study.result.is_some() { "done" } else { "ready" }});
    };
    let Ok(p) = job.progress.lock() else { return json!({"study": study.name, "state": "unknown"}) };
    let state = match &p.done {
        None => "running",
        Some(Ok(_)) => "done",
        Some(Err(_)) => "failed",
    };
    let mut out = json!({"study": study.name, "state": state, "steps": p.steps, "max_steps": p.max_steps, "backend": p.backend, "cells": p.cells});
    if let Some(Err(e)) = &p.done {
        out["error"] = json!(e);
    }
    if let Some(last) = &p.last {
        out["time_s"] = json!(last.time);
        out["inlet_flow_cfm"] = json!(last.inlets.iter().map(|t| t.flow * sim::study::CFM_PER_M3S).sum::<f64>());
    }
    out
}

fn through_json(name: &str, t: &sim::Through) -> Value {
    json!({
        "name": name,
        "flow_m3s": t.flow,
        "flow_cfm": t.flow * sim::study::CFM_PER_M3S,
        "pressure_pa": t.pressure,
        "speed_ms": t.speed,
        "max_speed_ms": t.max_speed,
        "area_mm2": t.area * 1e6,
    })
}

pub fn warning_text(w: &sim::Warning) -> String {
    match w {
        sim::Warning::Leak => "the walls leak: with the outlets closed the air still gets out (a gap, or a wall thinner than a cell)".into(),
        sim::Warning::ThinWalls(at) => format!(
            "{} place(s) where a wall is thinner than a cell, e.g. at {:.1?}; use smaller cells",
            at.len(),
            at.first().map(|p| [p.x, p.y, p.z]).unwrap_or_default()
        ),
        sim::Warning::DroppedCells { opening, cells } => format!("{cells} cell(s) of `{opening}` lie in a wall and are left out"),
        sim::Warning::LowViscosity { tau } => {
            format!("relaxation time {tau:.5}: the air's real viscosity is resolved by the turbulence model (expected at these speeds)")
        }
        sim::Warning::ShortSteps { pace } => format!(
            "the flow went unstable at the usual time step (jets faster than the named openings let it foresee), so it ran with steps {pace:.0}x shorter; naming the exits as outlets avoids the rerun"
        ),
    }
}

/// A finished study's results as `sim.flow.results` reports them.
pub fn results(name: &str, r: &FlowResult) -> Value {
    let a = &r.average;
    let inlets: Vec<Value> = a.inlets.iter().zip(&r.inlets).map(|(t, n)| through_json(n, t)).collect();
    let outlets: Vec<Value> = a.outlets.iter().zip(&r.outlets).map(|(t, n)| through_json(n, t)).collect();
    let total_out: f64 = a.outlets.iter().map(|t| t.flow).sum();
    let shares: Vec<Value> = a
        .outlets
        .iter()
        .zip(&r.outlets)
        .map(|(t, n)| json!({"name": n, "share": if total_out.abs() > 0.0 { t.flow / total_out } else { 0.0 }}))
        .collect();
    let area_mean = |ts: &[sim::Through]| {
        let area: f64 = ts.iter().map(|t| t.area).sum();
        if area > 0.0 { ts.iter().map(|t| t.pressure * t.area).sum::<f64>() / area } else { 0.0 }
    };
    let speed = r.fields.range(sim::Quantity::Speed).unwrap_or((0.0, 0.0));
    let pressure = r.fields.range(sim::Quantity::Pressure).unwrap_or((0.0, 0.0));
    json!({
        "study": name,
        "state": match r.state { sim::State::Settled => "settled", sim::State::StepLimit => "step limit", sim::State::Running => "running" },
        "steps": r.steps,
        "simulated_s": r.units.time(r.steps),
        "seconds": r.seconds,
        "backend": r.backend,
        "cells": r.fields.fluid.len(),
        "air_cells": r.fields.fluid.iter().filter(|f| **f).count(),
        "cell_mm": r.cell,
        "inlets": inlets,
        "outlets": outlets,
        "outlet_shares": shares,
        "pressure_drop_pa": area_mean(&a.inlets) - area_mean(&a.outlets),
        "inlet_speeds_ms": r.speeds,
        "speed_range_ms": [speed.0, speed.1],
        "pressure_range_pa": [pressure.0, pressure.1],
        "warnings": r.warnings.iter().map(warning_text).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------------------------
// The sidecar: `<design>.sim.json` beside the design file holds its studies and goal seeks (their
// setups and the seeks' cycles; never the flow fields), so the design file itself never changes.

/// The sidecar of the design saved at `design`: `duct.solidcraft` → `duct.sim.json`.
pub fn sidecar_path(design: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(design);
    let stem = p.file_stem().map_or_else(|| "design".into(), |s| s.to_string_lossy().to_string());
    p.with_file_name(format!("{stem}.sim.json"))
}

fn source_json(o: &OpeningSpec) -> Value {
    let p = [o.point.x, o.point.y, o.point.z];
    let mut v = match o.section {
        Some(n) => json!({"name": o.name, "body": o.body, "section": {"point": p, "normal": [n.x, n.y, n.z]}}),
        None => json!({"name": o.name, "body": o.body, "face": p}),
    };
    if let Some(r) = o.region {
        v["region"] = json!(if r == Region::Hole { "hole" } else { "face" });
    }
    match &o.source {
        Some(sim::Source::Speed(u)) => v["speed"] = json!(u),
        Some(sim::Source::Fan(c)) => {
            v["fan"] = json!({"curve": c.points().iter().map(|(q, pa)| [q * sim::study::CFM_PER_M3S, *pa]).collect::<Vec<_>>()})
        }
        None => {}
    }
    v
}

/// A study's setup as `sim.flow.new` takes it.
pub fn spec_params(name: &str, spec: &Spec) -> Value {
    json!({
        "name": name,
        "body": spec.body,
        "bodies": spec.bodies,
        "inlets": spec.inlets.iter().map(source_json).collect::<Vec<_>>(),
        "outlets": spec.outlets.iter().map(source_json).collect::<Vec<_>>(),
        "cell": spec.cell,
        "pad": spec.pad,
        "turbulence": spec.turbulence,
        "roughness_mm": spec.roughness,
        "max_steps": spec.max_steps,
    })
}

/// Write the sidecar of the session's design (when it has a file and studies or seeks; a design
/// without either leaves any sidecar there alone).
pub fn save_sidecar(s: &crate::Session) -> std::result::Result<(), String> {
    let Some(path) = s.path.as_deref() else { return Ok(()) };
    let studies = s.flow_studies();
    if studies.list.is_empty() && studies.seeks.is_empty() {
        return Ok(());
    }
    let seeks: Vec<Value> = studies
        .seeks
        .iter()
        .map(|k| {
            let mut spec = crate::seek::spec_json(&k.spec);
            spec["name"] = json!(k.name);
            json!({"setup": spec, "cycles": k.cycles.iter().enumerate().map(|(i, c)| crate::seek::cycle_json(&k.spec, i, c)).collect::<Vec<_>>()})
        })
        .collect();
    let doc = json!({
        "solidcraft_sim": 1,
        "design": std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()),
        "studies": studies.list.iter().map(|st| spec_params(&st.name, &st.spec)).collect::<Vec<_>>(),
        "seeks": seeks,
    });
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    let out = sidecar_path(path);
    solidcraft_io::vfs::write(&out.to_string_lossy(), text.as_bytes()).map_err(|e| format!("{}: {e}", out.display()))
}

fn cycle_from(spec: &crate::seek::SeekSpec, v: &Value) -> Option<crate::seek::Cycle> {
    let values = spec.vary.iter().map(|x| v.get("values")?.get(&x.parameter)?.as_f64()).collect::<Option<Vec<f64>>>()?;
    let mut measures = std::collections::BTreeMap::new();
    for (k, m) in v.get("measures")?.as_object()? {
        let f = |key: &str| m.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        measures.insert(
            k.clone(),
            crate::seek::Measured { flow: f("flow_m3s"), pressure: f("pressure_pa"), speed: f("speed_ms"), max_speed: f("max_speed_ms") },
        );
    }
    Some(crate::seek::Cycle {
        values,
        measures,
        score: v.get("score").and_then(Value::as_f64).unwrap_or(f64::INFINITY),
        feasible: v.get("feasible").and_then(Value::as_bool).unwrap_or(false),
        error: v.get("error").and_then(Value::as_str).map(str::to_string),
        steps: v.get("steps").and_then(Value::as_u64).unwrap_or(0),
        seconds: v.get("seconds").and_then(Value::as_f64).unwrap_or(0.0),
    })
}

/// Read the sidecar of the session's design back: its studies set up again on the design as it is,
/// its seeks with their cycles. What no longer fits the design (a body gone, a parameter renamed)
/// is left out, and said so in the returned notes.
pub fn load_sidecar(s: &mut crate::Session) -> Vec<String> {
    let Some(path) = s.path.clone() else { return Vec::new() };
    let file = sidecar_path(&path);
    let Ok(bytes) = solidcraft_io::vfs::read(&file.to_string_lossy(), 64 * 1024 * 1024) else { return Vec::new() };
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else { return vec![format!("{}: not a study file", file.display())] };
    let mut notes = Vec::new();
    for st in doc.get("studies").and_then(Value::as_array).into_iter().flatten().take(64) {
        if let Err(e) = s.execute("sim.flow.new", st) {
            notes.push(format!("study {}: {e}", st.get("name").and_then(Value::as_str).unwrap_or("?")));
        }
    }
    for k in doc.get("seeks").and_then(Value::as_array).into_iter().flatten().take(64) {
        let Some(setup) = k.get("setup") else { continue };
        let name = setup.get("name").and_then(Value::as_str).unwrap_or("?").to_string();
        match s.execute("sim.seek.new", setup) {
            Ok(_) => {
                if let Some(seek) = s.flow.seeks.iter_mut().find(|x| x.name == name) {
                    let cycles: Vec<crate::seek::Cycle> =
                        k.get("cycles").and_then(Value::as_array).into_iter().flatten().filter_map(|c| cycle_from(&seek.spec, c)).collect();
                    if !cycles.is_empty() {
                        seek.cycles = cycles;
                        seek.state = crate::seek::SeekState::Done;
                    }
                }
            }
            Err(e) => notes.push(format!("goal seek {name}: {e}")),
        }
    }
    notes
}
