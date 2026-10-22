//! Conservative reconstruction of constant-section mechanical mesh parts.
//! Geometry is inferred from closed, manifold input, never from a display mesh.

use std::collections::BTreeMap;

use solidcraft_geom::{Loop2, Plane, Region2, Vec2, Vec3};

use crate::{Body, EdgeSpec, FaceSpec, KernelError, Result, SurfSpec};

const MAX_TRIANGLES: usize = 50_000;
const MAX_BOUNDARY: usize = 8192;
/// Nesting certification runs two planar booleans per pair of boundary loops,
/// so the loop count (not the segment count) bounds its cost: 50 loops (an
/// outline with 49 holes) is at most 1225 pairs and 2450 booleans.
const MAX_LOOPS: usize = 50;

fn invalid(message: &str) -> KernelError {
    KernelError::Invalid(format!("mesh conversion: {message}"))
}

/// A mesh over the triangle budget: how big it is and the limit (#346).
fn over_budget(triangles: usize) -> KernelError {
    invalid(&format!("the mesh has {triangles} triangles, more than the {MAX_TRIANGLES} the conversion can take; simplify it first"))
}

/// Most faces a general conversion may sew: every face left after fusing coplanar triangles is a
/// truck face, and sewing costs time and memory in proportion.
const MAX_FACES: usize = 20_000;

/// Reconstruct a constant-section mesh as analytic planes and, where certified
/// within `tolerance` mm, circular cylinders. The source mesh is not modified.
/// Unsupported shapes are refused instead of silently returning a faceted solid.
pub fn convert_prismatic_mesh(body: &Body, tolerance: f64) -> Result<Body> {
    crate::guard("convert prismatic mesh", || {
        let (welded, axes, _) = prepared(body, tolerance)?;
        let mesh = welded.mesh.as_ref().ok_or_else(|| invalid("empty mesh"))?;
        for (axis, _) in axes.into_iter().take(16) {
            if let Some(result) = extrusion(mesh, axis, tolerance)? {
                return Ok(result);
            }
        }
        Err(invalid("this mesh is not a supported constant-section prism; no geometry was converted"))
    })
}

/// Reconstruct any closed, manifold mesh as a solid body. Triangles sharing a plane fuse into one
/// planar face, holes and all; a triangle on a curved surface (a bore's wall, a fillet) becomes a
/// facet of its own, so a slicer's 3MF or STL gets the operations that need a solid. The exact
/// prismatic reconstruction is tried first, and the source mesh is not modified.
pub fn convert_mesh(body: &Body, tolerance: f64) -> Result<Body> {
    crate::guard("convert mesh", || {
        let (welded, axes, signed_volume) = prepared(body, tolerance)?;
        let mesh = welded.mesh.as_ref().ok_or_else(|| invalid("empty mesh"))?;
        for (axis, _) in axes.into_iter().take(16) {
            if let Some(result) = extrusion(mesh, axis, tolerance)? {
                return Ok(result);
            }
        }
        planar_solid(mesh, tolerance, signed_volume)
    })
}

/// The welded mesh body once it is known to be closed, consistently oriented and facing outwards,
/// the directions its triangles' normals take (the most common first, by area, which the prismatic
/// reconstruction tries as its extrusion axes), and the volume its triangles enclose.
fn prepared(body: &Body, tolerance: f64) -> Result<(Body, Vec<(Vec3, f64)>, f64)> {
    if !tolerance.is_finite() || !(1e-6..=1.).contains(&tolerance) {
        return Err(invalid("tolerance must be between 0.000001 and 1 mm"));
    }
    let input = body.mesh.as_ref().ok_or_else(|| invalid("select a mesh body"))?;
    let welded = weld(input, tolerance)?;
    let mesh = welded.mesh.as_ref().ok_or_else(|| invalid("empty mesh"))?;
    if mesh.triangles.len() > MAX_TRIANGLES {
        return Err(over_budget(mesh.triangles.len()));
    }
    if mesh.triangles.len() < 4 {
        return Err(invalid("a solid needs at least four triangles, two shells at the least"));
    }
    let mut edges: BTreeMap<(u32, u32), (usize, i32)> = BTreeMap::new();
    let mut axes: BTreeMap<(i64, i64, i64), (Vec3, f64)> = BTreeMap::new();
    let mut signed_volume = 0.;
    for triangle in &mesh.triangles {
        let [a, b, c] = *triangle;
        let (Some(pa), Some(pb), Some(pc)) = (mesh.positions.get(a as usize), mesh.positions.get(b as usize), mesh.positions.get(c as usize)) else {
            return Err(invalid("triangle refers to a missing vertex"));
        };
        let cross = (*pb - *pa).cross(*pc - *pa);
        let Some(mut n) = cross.normalized() else { return Err(invalid("degenerate triangle")) };
        signed_volume += pa.dot(pb.cross(*pc)) / 6.;
        for (a, b) in [(a, b), (b, c), (c, a)] {
            let entry = edges.entry((a.min(b), a.max(b))).or_default();
            entry.0 += 1;
            entry.1 += if a < b { 1 } else { -1 };
        }
        if n.x < -1e-8 || (n.x.abs() <= 1e-8 && (n.y < -1e-8 || (n.y.abs() <= 1e-8 && n.z < 0.))) {
            n = -n;
        }
        let key = ((n.x * 1e7).round() as i64, (n.y * 1e7).round() as i64, (n.z * 1e7).round() as i64);
        let entry = axes.entry(key).or_insert((n, 0.));
        entry.1 += cross.len();
    }
    if edges.values().any(|&(count, balance)| count != 2 || balance != 0) {
        return Err(invalid("mesh must be closed and consistently oriented, with two triangles per edge"));
    }
    // A consistently oriented mesh whose triangles face inwards passes the edge check, but
    // the body it describes is turned inside out: its signed volume would be negative.
    if !signed_volume.is_finite() || signed_volume <= 0. {
        return Err(invalid("the mesh's triangles face inwards; flip them before converting"));
    }
    let mut axes: Vec<_> = axes.into_values().collect();
    axes.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok((welded, axes, signed_volume))
}

fn weld(mesh: &crate::meshbody::TriMesh, tolerance: f64) -> Result<Body> {
    if mesh.triangles.len() > MAX_TRIANGLES {
        return Err(over_budget(mesh.triangles.len()));
    }
    if mesh.positions.len() > MAX_TRIANGLES * 3 {
        return Err(invalid(&format!("the mesh has {} vertices, more than the {} conversion can take", mesh.positions.len(), MAX_TRIANGLES * 3)));
    }
    let bounds = mesh.positions.iter().fold(solidcraft_geom::Aabb3::EMPTY, |mut b, p| {
        b.add(*p);
        b
    });
    let epsilon = (bounds.size().len().max(1.) * 1e-7).min(tolerance * 0.01).max(1e-9);
    if !epsilon.is_finite() || mesh.positions.iter().any(|p| [p.x, p.y, p.z].iter().any(|v| !v.is_finite() || (v / epsilon).abs() > 1e15)) {
        return Err(invalid("mesh coordinates exceed the conversion budget"));
    }
    let key = |p: Vec3| ((p.x / epsilon).floor() as i64, (p.y / epsilon).floor() as i64, (p.z / epsilon).floor() as i64);
    let mut grid = BTreeMap::<_, Vec<u32>>::new();
    let mut positions: Vec<Vec3> = Vec::new();
    let mut remap = Vec::new();
    let mut comparisons = 0usize;
    for &p in &mesh.positions {
        let k = key(p);
        let mut found = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &i in grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)).into_iter().flatten() {
                        comparisons += 1;
                        if comparisons > 8_000_000 {
                            return Err(invalid("vertex welding exceeds its work budget"));
                        }
                        if positions.get(i as usize).is_some_and(|q| q.dist(p) <= epsilon) {
                            found = Some(i);
                            break;
                        }
                    }
                }
            }
        }
        let i = match found {
            Some(i) => i,
            None => {
                let i = u32::try_from(positions.len()).map_err(|_| invalid("too many vertices"))?;
                positions.push(p);
                grid.entry(k).or_default().push(i);
                i
            }
        };
        remap.push(i);
    }
    let triangles = mesh
        .triangles
        .iter()
        .map(|t| {
            let (Some(&a), Some(&b), Some(&c)) = (remap.get(t[0] as usize), remap.get(t[1] as usize), remap.get(t[2] as usize)) else {
                return Err(invalid("missing triangle vertex"));
            };
            Ok([a, b, c])
        })
        .collect::<Result<Vec<_>>>()?;
    crate::mesh_body(&positions, &triangles)
}

fn extrusion(mesh: &crate::meshbody::TriMesh, axis: Vec3, tolerance: f64) -> Result<Option<Body>> {
    let (lo, hi) = mesh.positions.iter().map(|p| p.dot(axis)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), h| (lo.min(h), hi.max(h)));
    let height = hi - lo;
    let bounds = mesh.positions.iter().fold(solidcraft_geom::Aabb3::EMPTY, |mut b, p| {
        b.add(*p);
        b
    });
    let epsilon = (bounds.size().len().max(1.) * 1e-7).min(tolerance * 0.01).max(1e-9);
    if height <= epsilon {
        return Ok(None);
    }
    let Some(plane) = Plane::from_normal(axis * lo, axis) else { return Ok(None) };
    let mut caps = [BTreeMap::<(u32, u32), usize>::new(), BTreeMap::new()];
    for triangle in &mesh.triangles {
        let [a, b, c] = *triangle;
        let (Some(pa), Some(pb), Some(pc)) = (mesh.positions.get(a as usize), mesh.positions.get(b as usize), mesh.positions.get(c as usize)) else {
            return Err(invalid("missing triangle vertex"));
        };
        let hs = [pa.dot(axis), pb.dot(axis), pc.dot(axis)];
        let cap = if hs.iter().all(|h| (h - lo).abs() <= epsilon) {
            0
        } else if hs.iter().all(|h| (h - hi).abs() <= epsilon) {
            1
        } else {
            // STL stores coordinates as f32. Test the projected triangle's
            // distance from a line rather than amplifying that rounding into
            // an angular error on narrow side triangles.
            let [a, b, c] = [plane.to_local(*pa), plane.to_local(*pb), plane.to_local(*pc)];
            let length = a.dist(b).max(b.dist(c)).max(c.dist(a));
            if (b - a).cross(c - a).abs() > epsilon * 3. * length {
                return Ok(None);
            }
            continue;
        };
        let Some(edges) = caps.get_mut(cap) else { return Ok(None) };
        for (a, b) in [(a, b), (b, c), (c, a)] {
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let raw_key = |p: Vec2| ((p.x / epsilon).floor() as i64, (p.y / epsilon).floor() as i64);
    let mut positions = BTreeMap::<(i64, i64), Vec2>::new();
    let mut projected = Vec::with_capacity(mesh.positions.len());
    for p in &mesh.positions {
        let p = plane.to_local(*p);
        let k = raw_key(p);
        let mut canonical = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let candidate = (k.0 + dx, k.1 + dy);
                if positions.get(&candidate).is_some_and(|q| q.dist(p) <= epsilon) {
                    canonical = Some(candidate);
                }
            }
        }
        let canonical = canonical.unwrap_or(k);
        positions.entry(canonical).or_insert(p);
        projected.push(canonical);
    }
    let boundary = |cap: usize| -> Option<BTreeMap<(i64, i64), Vec<(i64, i64)>>> {
        let mut adjacency = BTreeMap::<_, Vec<_>>::new();
        for (&(a, b), &count) in caps.get(cap)? {
            if count == 2 {
                continue;
            }
            if count != 1 {
                return None;
            }
            let (a, b) = (*projected.get(a as usize)?, *projected.get(b as usize)?);
            if a == b {
                return None;
            }
            adjacency.entry(a).or_default().push(b);
            adjacency.entry(b).or_default().push(a);
        }
        for neighbours in adjacency.values_mut() {
            neighbours.sort();
        }
        Some(adjacency)
    };
    let (Some(mut adjacency), Some(other)) = (boundary(0), boundary(1)) else { return Ok(None) };
    if adjacency != other || adjacency.is_empty() || adjacency.len() > MAX_BOUNDARY || adjacency.values().any(|n| n.len() != 2) {
        return Ok(None);
    }
    // Every side vertex must lie on an unchanged straight extrusion of the cap.
    // This excludes twisted, tapered and stepped meshes before fitting any circles.
    let point = |k: (i64, i64)| Vec2::new(k.0 as f64 * epsilon, k.1 as f64 * epsilon);
    let boundary_segments: Vec<_> =
        adjacency.iter().flat_map(|(&a, bs)| bs.iter().filter(move |&&b| a < b).map(move |&b| (point(a), point(b)))).collect();
    if mesh.positions.len().saturating_mul(boundary_segments.len()) > 32_000_000 {
        return Err(invalid("boundary certification exceeds its work budget"));
    }
    for p in &mesh.positions {
        let h = p.dot(axis);
        if (h - lo).abs() <= epsilon || (h - hi).abs() <= epsilon {
            continue;
        }
        let p = plane.to_local(*p);
        if !boundary_segments.iter().any(|&(a, b)| {
            solidcraft_geom::Seg2::Line { a, b }.point_at(solidcraft_geom::Seg2::Line { a, b }.closest_param(p)).dist(p) <= epsilon * 3.
        }) {
            return Ok(None);
        }
    }
    let mut loops = Vec::new();
    let mut raw_loops = Vec::new();
    while let Some((&start, _)) = adjacency.first_key_value() {
        let mut current = start;
        let mut previous = None;
        let mut chain = Vec::new();
        for _ in 0..=MAX_BOUNDARY {
            let Some(neighbours) = adjacency.remove(&current) else { return Ok(None) };
            chain.push(*positions.get(&current).unwrap_or(&point(current)));
            let next = neighbours.into_iter().find(|n| Some(*n) != previous);
            let Some(next) = next else { return Ok(None) };
            if next == start {
                break;
            }
            previous = Some(current);
            current = next;
        }
        if chain.len() < 3 || current == start {
            return Ok(None);
        }
        raw_loops.push(Loop2::polygon(&chain));
        loops.push(fit_loop(&chain, tolerance, epsilon));
    }
    certify_nesting(&raw_loops, &loops, epsilon, bounds.size().len())?;
    // Nest by the original polygon, so smoothing cannot change which loop is a hole.
    let mut regions: Vec<Region2> = Vec::new();
    for (i, lp) in loops.iter().enumerate() {
        let Some(probe) = raw_loops.get(i).and_then(|l| l.segs.first()).map(|s| s.start()) else { return Ok(None) };
        let owners: Vec<_> = raw_loops.iter().enumerate().filter(|(j, other)| *j != i && other.contains(probe)).collect();
        if owners.len() % 2 == 0 {
            regions.push(Region2 { outer: lp.ccw(), holes: Vec::new() });
        }
    }
    for (i, lp) in loops.iter().enumerate() {
        let Some(probe) = raw_loops.get(i).and_then(|l| l.segs.first()).map(|s| s.start()) else { return Ok(None) };
        let owners = raw_loops.iter().enumerate().filter(|(j, other)| *j != i && other.contains(probe)).count();
        if owners % 2 == 1 {
            let owner = regions
                .iter()
                .enumerate()
                .filter(|(_, r)| r.outer.contains(probe))
                .min_by(|(_, a), (_, b)| a.outer.signed_area().abs().total_cmp(&b.outer.signed_area().abs()))
                .map(|(i, _)| i);
            let Some(region) = owner.and_then(|i| regions.get_mut(i)) else { return Ok(None) };
            region.holes.push(lp.ccw().reversed());
        }
    }
    let body = crate::extrude_planar_regions(&plane, &regions, 0., height)?;
    if !body.validity().is_empty() {
        return Err(invalid("reconstructed solid did not pass validity checks"));
    }
    let perimeter: f64 = loops.iter().flat_map(|lp| &lp.segs).map(|s| s.length()).sum();
    let original = mesh.to_mesh().measure().volume;
    let rebuilt = crate::measure(&body)?.volume;
    if !original.is_finite() || original <= 0. || (rebuilt - original).abs() > perimeter * height * tolerance + epsilon * bounds.size().len().powi(2)
    {
        return Err(invalid("reconstructed volume is inconsistent with the source mesh"));
    }
    Ok(Some(body))
}

/// The mesh as a solid of planar faces: neighbouring triangles on one plane fuse into a single
/// face (holes and all), and every other triangle stays a facet of its own. The faces share the
/// welded mesh vertices exactly, so they sew along their edges without booleans.
fn planar_solid(mesh: &crate::meshbody::TriMesh, tolerance: f64, signed_volume: f64) -> Result<Body> {
    let (positions, triangles) = (&mesh.positions, &mesh.triangles);
    let bounds = positions.iter().fold(solidcraft_geom::Aabb3::EMPTY, |mut b, p| {
        b.add(*p);
        b
    });
    let size = bounds.size().len().max(1.);
    let epsilon = (size * 1e-7).min(tolerance * 0.01).max(1e-9);
    // Fusing triangles into one face makes that face's boundary hold their vertices, so it is
    // only planar to the depth they sit off the region's plane: keep that well inside the
    // 1e-6 mm truck's `attach_plane` allows, or the face is refused (a slicer's f32 mesh has
    // its flat faces rounded by about 1e-5 mm, so they stay one facet per triangle instead).
    let planar = 1e-7_f64.min(epsilon);
    // Long work reports to whoever waits for it (the import card) and stops when they cancel.
    let watch = crate::watch::current();
    let cancelled = || watch.as_ref().is_some_and(|w| w.cancelled());
    let report = |done: usize, total: usize| {
        if let Some(w) = watch.as_ref() {
            w.progress("converting", done, total);
        }
    };
    let total = triangles.len();
    report(0, total);
    // Which triangles share each edge; the check in `prepared` leaves exactly two.
    let mut owners: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (t, triangle) in triangles.iter().enumerate() {
        let [a, b, c] = *triangle;
        for (u, v) in [(a, b), (b, c), (c, a)] {
            owners.entry((u.min(v), u.max(v))).or_default().push(t);
        }
    }
    let mut normals = Vec::with_capacity(triangles.len());
    for triangle in triangles {
        let [a, b, c] = *triangle;
        let vertex = |i: u32| positions.get(i as usize).copied().ok_or_else(|| invalid("triangle refers to a missing vertex"));
        let (pa, pb, pc) = (vertex(a)?, vertex(b)?, vertex(c)?);
        normals.push((pb - pa).cross(pc - pa).normalized().ok_or_else(|| invalid("degenerate triangle"))?);
    }
    // Fuse neighbouring triangles that lie on one plane, so a flat face (and the wall of a
    // polygonal hole) becomes one face instead of two triangles.
    let mut region = vec![usize::MAX; triangles.len()];
    let mut regions: Vec<Vec<usize>> = Vec::new();
    for seed in 0..triangles.len() {
        if seed % 256 == 0 {
            if cancelled() {
                return Err(crate::watch::cancelled_error("mesh conversion"));
            }
            report(seed, total);
        }
        if region.get(seed).copied() != Some(usize::MAX) {
            continue;
        }
        if regions.len() >= MAX_FACES {
            return Err(too_many_faces());
        }
        let Some(&normal) = normals.get(seed) else { continue };
        let Some(apex) = triangles.get(seed).and_then(|t| t.first()).and_then(|&i| positions.get(i as usize)).copied() else {
            return Err(invalid("triangle refers to a missing vertex"));
        };
        let offset = normal.dot(apex);
        let id = regions.len();
        region[seed] = id;
        let mut members = vec![seed];
        let mut stack = vec![seed];
        while let Some(t) = stack.pop() {
            let Some(&[a, b, c]) = triangles.get(t) else { continue };
            for (u, v) in [(a, b), (b, c), (c, a)] {
                let Some(shared) = owners.get(&(u.min(v), u.max(v))) else { continue };
                for &other in shared {
                    if other == t || region.get(other).copied() != Some(usize::MAX) {
                        continue;
                    }
                    let on_plane = triangles
                        .get(other)
                        .is_some_and(|o| o.iter().all(|&k| positions.get(k as usize).is_some_and(|p| (normal.dot(*p) - offset).abs() <= planar)));
                    if on_plane {
                        region[other] = id;
                        members.push(other);
                        stack.push(other);
                    }
                }
            }
        }
        members.sort_unstable();
        regions.push(members);
    }
    report(total, total);
    if cancelled() {
        return Err(crate::watch::cancelled_error("mesh conversion"));
    }
    let mut faces = Vec::with_capacity(regions.len());
    for members in &regions {
        let loops = face_loops(members, &region, &owners, positions, triangles)?;
        if loops.is_empty() {
            return Err(invalid("a face of the mesh has no boundary"));
        }
        faces.push(FaceSpec { loops, surface: SurfSpec::Plane, probe: None });
    }
    let body = crate::sew(&faces).map_err(|e| match e {
        KernelError::Invalid(detail) => invalid(&format!("the faces cannot be sewn into a solid: {detail}")),
        other => other,
    })?;
    let repairs = body.validity();
    if !repairs.is_empty() {
        return Err(invalid("the converted solid did not pass validity checks"));
    }
    let rebuilt = crate::measure(&body)?.volume;
    if !rebuilt.is_finite() || (rebuilt - signed_volume).abs() > signed_volume.abs() * 1e-6 + size * size * 1e-9 {
        return Err(invalid("the converted solid's volume does not match the mesh"));
    }
    Ok(body)
}

fn too_many_faces() -> KernelError {
    invalid(&format!("the mesh leaves more than {MAX_FACES} faces after fusing coplanar triangles; simplify it before converting"))
}

/// The boundary loops of one planar region, as the analytic edges of one face: every boundary
/// edge runs with the region on its left, so the outer loop is counter-clockwise about the face's
/// outward normal and a hole is clockwise, as sewing wants them.
fn face_loops(
    members: &[usize],
    region: &[usize],
    owners: &BTreeMap<(u32, u32), Vec<usize>>,
    positions: &[Vec3],
    triangles: &[[u32; 3]],
) -> Result<Vec<Vec<EdgeSpec>>> {
    let id = members.first().and_then(|t| region.get(*t)).copied().unwrap_or(usize::MAX);
    let mut next: BTreeMap<u32, u32> = BTreeMap::new();
    let mut boundary_edges = 0usize;
    for &t in members {
        let Some(&[a, b, c]) = triangles.get(t) else { continue };
        for (u, v) in [(a, b), (b, c), (c, a)] {
            let Some(shared) = owners.get(&(u.min(v), u.max(v))) else {
                return Err(invalid("the mesh is not closed"));
            };
            // The face keeps the edge when the triangle on its other side is another region's.
            if shared.iter().all(|&other| other == t || region.get(other).copied() == Some(id)) {
                continue;
            }
            boundary_edges += 1;
            if boundary_edges > MAX_BOUNDARY {
                return Err(invalid("a face of the mesh has too long a boundary to convert"));
            }
            if next.insert(u, v).is_some() {
                return Err(invalid("the mesh's face boundary branches at a vertex; split that point before converting"));
            }
        }
    }
    let mut loops = Vec::new();
    while let Some((&start, _)) = next.iter().next() {
        let mut u = start;
        let mut points = Vec::new();
        for _ in 0..=MAX_BOUNDARY {
            let Some(v) = next.remove(&u) else { break };
            points.push(positions.get(u as usize).copied().ok_or_else(|| invalid("triangle refers to a missing vertex"))?);
            u = v;
            if u == start {
                break;
            }
        }
        if u != start {
            return Err(invalid("the mesh's face boundary is open"));
        }
        // Every boundary vertex stays: a merged face must share each mesh edge with the face on
        // its other side exactly, or a vertex dropped along a straight edge (a T-junction) would
        // leave the shell open.
        // Only a point repeating the one before it is dropped (a zero-length edge is refused).
        let mut kept: Vec<Vec3> = Vec::with_capacity(points.len());
        for p in points {
            if kept.last().is_none_or(|q| q.dist(p) > 0.0) {
                kept.push(p);
            }
        }
        while kept.len() > 1 && kept.first().is_some_and(|a| kept.last().is_some_and(|b| a.dist(*b) <= 0.0)) {
            kept.pop();
        }
        if kept.len() < 3 {
            return Err(invalid("a face of the mesh is degenerate"));
        }
        let loop_edges =
            kept.iter().copied().zip(kept.iter().copied().cycle().skip(1)).take(kept.len()).map(|(a, b)| EdgeSpec::Line { a, b }).collect();
        loops.push(loop_edges);
    }
    Ok(loops)
}

fn certify_nesting(raw: &[Loop2], fitted: &[Loop2], epsilon: f64, size: f64) -> Result<()> {
    if raw.len() > MAX_LOOPS {
        return Err(KernelError::Invalid(format!(
            "mesh conversion: too complex to convert ({} boundary loops; at most {MAX_LOOPS} are supported)",
            raw.len()
        )));
    }
    let mut work = 0usize;
    for (i, a) in raw.iter().enumerate() {
        for (j, b) in raw.iter().enumerate().skip(i + 1) {
            work = work.saturating_add(a.segs.len().saturating_mul(b.segs.len()));
            if work > 16_000_000 {
                return Err(invalid("loop certification exceeds its work budget"));
            }
            let contains = |a: &Loop2, b: &Loop2| b.segs.first().is_some_and(|s| a.contains(s.start()));
            let (ab, ba) = (contains(a, b), contains(b, a));
            if ab && ba {
                return Err(invalid("overlapping boundary loops"));
            }
            for (a, b) in
                [(a, b), (fitted.get(i).ok_or_else(|| invalid("missing fitted loop"))?, fitted.get(j).ok_or_else(|| invalid("missing fitted loop"))?)]
            {
                let region = |l: &Loop2| Region2 { outer: l.ccw(), holes: Vec::new() };
                let overlap = crate::planar_regions_boolean(&[region(a)], &[region(b)], crate::BoolOp::Intersect, epsilon.clamp(1e-9, 1.))?;
                let area: f64 = overlap.iter().map(Region2::area).sum();
                let expected = if ab {
                    b.signed_area().abs()
                } else if ba {
                    a.signed_area().abs()
                } else {
                    0.
                };
                if (area - expected).abs() > epsilon * size.max(1.) * 10. {
                    return Err(invalid("conversion would cross or merge boundary loops"));
                }
            }
        }
    }
    Ok(())
}

fn fit_loop(points: &[Vec2], tolerance: f64, epsilon: f64) -> Loop2 {
    // Fit a complete circular boundary only with enough samples to distinguish it
    // from a polygon. Certify the chord deviation, not just the vertex residual.
    if points.len() >= 12 {
        let (Some(&a), Some(&b), Some(&c)) = (points.first(), points.get(points.len() / 3), points.get(2 * points.len() / 3)) else {
            return Loop2::polygon(points);
        };
        let u = b - a;
        let v = c - a;
        let d = 2. * u.cross(v);
        if d.abs() > epsilon * epsilon {
            let center = a + Vec2::new(v.y * u.len2() - u.y * v.len2(), u.x * v.len2() - v.x * u.len2()) / d;
            let radius = center.dist(a);
            if radius.is_finite() && radius > epsilon && points.iter().all(|p| (center.dist(*p) - radius).abs() <= epsilon * 3.) {
                let mut sweep = 0.;
                let mut direction = 0f64;
                let valid = (0..points.len()).all(|i| {
                    let (Some(a), Some(b)) = (points.get(i), points.get((i + 1) % points.len())) else { return false };
                    let signed = ((*a - center).cross(*b - center)).atan2((*a - center).dot(*b - center));
                    if direction == 0. {
                        direction = signed.signum();
                    }
                    if signed * direction <= 0. {
                        return false;
                    }
                    sweep += signed;
                    let angle = signed.abs();
                    angle <= std::f64::consts::PI / 6. + 1e-8 && radius * (1. - (angle * 0.5).cos()) + epsilon * 10. <= tolerance
                });
                if valid && (sweep.abs() - std::f64::consts::TAU).abs() <= 1e-7 {
                    return Loop2::circle(center, radius);
                }
            }
        }
    }
    let mut points = points.to_vec();
    let linear_tolerance = epsilon / points.len().max(1) as f64;
    loop {
        let n = points.len();
        if n <= 3 {
            break;
        }
        let remove = (0..n).find(|&i| {
            let (Some(a), Some(b), Some(c)) = (points.get((i + n - 1) % n), points.get(i), points.get((i + 1) % n)) else { return false };
            let edge = solidcraft_geom::Seg2::Line { a: *a, b: *c };
            edge.point_at(edge.closest_param(*b)).dist(*b) <= linear_tolerance
        });
        let Some(index) = remove else { break };
        points.remove(index);
    }
    Loop2::polygon(&points)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh_of(body: &Body) -> Body {
        let mesh = body.tessellate(0.005).unwrap();
        crate::mesh_body(&mesh.positions, &mesh.triangles).unwrap()
    }

    #[test]
    fn reconstructs_planes_holes_and_round_walls_in_arbitrary_orientation() {
        let region = Region2 {
            outer: Loop2::polygon(&[
                Vec2::new(0., 0.),
                Vec2::new(40., 0.),
                Vec2::new(40., 20.),
                Vec2::new(20., 20.),
                Vec2::new(20., 30.),
                Vec2::new(0., 30.),
            ]),
            holes: vec![Loop2::circle(Vec2::new(10., 10.), 4.).reversed()],
        };
        let source = crate::extrude_planar_regions(&Plane::XY, &[region], 0., 15.).unwrap();
        let source = crate::transform(&source, Vec3::new(31., -17., 9.), Vec3::ZERO, Vec3::new(1., 2., 3.), 0.73).unwrap();
        let converted = convert_prismatic_mesh(&mesh_of(&source), 0.05).unwrap();
        assert!(!converted.is_mesh());
        assert!(converted.validity().is_empty());
        let expected = (1000. - std::f64::consts::PI * 16.) * 15.;
        let fine_volume = |body: &Body| body.tessellate_with(1e-5, true).unwrap().measure().volume;
        assert!((fine_volume(&converted) - expected).abs() < 0.02);
        assert!((crate::measure(&converted).unwrap().volume - crate::measure(&source).unwrap().volume).abs() < 1e-7);
        assert_eq!(crate::measure(&converted).unwrap().merged.face_types.get("cylinder"), Some(&1));
        assert!(converted.faces(0.01).unwrap().len() < 15);
        let step = crate::step_export(&[&converted], "SolidCraft regression").unwrap();
        let reopened = crate::step_import(&step).unwrap();
        assert!((fine_volume(&reopened.bodies[0].body) - expected).abs() < 0.02);
    }

    #[test]
    fn planar_boxes_are_exact_and_coarse_polygons_are_not_rounded() {
        let source = crate::box_solid(Vec3::ZERO, Vec3::new(10., 20., 30.)).unwrap();
        let result = convert_prismatic_mesh(&mesh_of(&source), 0.01).unwrap();
        assert_eq!(result.faces(0.01).unwrap().len(), 6);
        assert!((crate::measure(&result).unwrap().volume - 6000.).abs() < 1e-6);
        let points: Vec<_> = (0..6)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / 6.;
                Vec2::new(angle.cos(), angle.sin()) * 10.
            })
            .collect();
        assert_eq!(fit_loop(&points, 1., 1e-6).segs.len(), 6);
        let points: Vec<_> = (0..16)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / 16.;
                Vec2::new(angle.cos(), angle.sin()) * 10.
            })
            .collect();
        assert_eq!(fit_loop(&points, 0.001, 1e-6).segs.len(), 16);
        assert_eq!(fit_loop(&points, 0.2, 1e-6).segs.len(), 2);
    }

    /// A stepped block: a box with a smaller box standing on it, which no single profile
    /// extruded straight can make, so the general conversion (not the prismatic one) builds it.
    fn stepped_block() -> Body {
        let base = crate::box_solid(Vec3::ZERO, Vec3::new(30., 20., 10.)).unwrap();
        let boss = crate::box_solid(Vec3::new(5., 5., 10.), Vec3::new(20., 15., 25.)).unwrap();
        crate::boolean(&base, &boss, crate::BoolOp::Union).unwrap().unwrap()
    }

    fn mesh_at(body: &Body, tolerance: f64) -> Body {
        let mesh = body.tessellate_with(tolerance, true).unwrap();
        crate::mesh_body(&mesh.positions, &mesh.triangles).unwrap()
    }

    #[test]
    fn a_nonprismatic_mesh_converts_to_a_solid_that_takes_booleans() {
        let source = stepped_block();
        let expected = crate::measure(&source).unwrap().volume;
        let converted = convert_mesh(&mesh_of(&source), 0.05).unwrap();
        assert!(!converted.is_mesh());
        assert!(converted.validity().is_empty(), "{:?}", converted.validity());
        assert!((crate::measure(&converted).unwrap().volume - expected).abs() < 1e-6);
        // The flat faces fused: a stepped block is 12 planes, not 2 triangles each.
        assert_eq!(converted.faces(0.01).unwrap().len(), 11);
        // The whole point of converting: the solid body takes the operations that refuse a mesh.
        let tool = crate::cylinder(Vec3::new(25., 15., -5.), Vec3::new(0., 0., 20.), 3., 20.).unwrap();
        let cut = crate::boolean(&converted, &tool, crate::BoolOp::Cut).unwrap().unwrap();
        // The hole is measured through the new cylindrical face, so allow its tessellation.
        let hole = std::f64::consts::PI * 9. * 10.;
        assert!((crate::measure(&cut).unwrap().volume - (expected - hole)).abs() < 0.5, "{:?}", crate::measure(&cut).unwrap().volume);
    }

    #[test]
    fn a_flat_face_meeting_a_rounded_one_converts_without_t_junctions() {
        let boxed = crate::box_solid(Vec3::ZERO, Vec3::new(40., 30., 20.)).unwrap();
        // Round one vertical edge: the flat faces then meet facets along shared vertices, and a
        // fused flat face must keep the ones along a straight edge (dropping them was a
        // T-junction that left the shell open on a real 3MF).
        let filleted = crate::fillet(&boxed, &[Vec3::new(40., 0., 10.)], 8.).unwrap();
        let mesh = mesh_of(&filleted);
        let mesh_volume = mesh.mesh.as_ref().unwrap().to_mesh().measure().volume;
        let converted = convert_mesh(&mesh, 0.05).unwrap();
        assert!(converted.validity().is_empty(), "{:?}", converted.validity());
        // The converted solid is the mesh (its fillet faceted), so it measures the mesh's volume.
        assert!((crate::measure(&converted).unwrap().volume - mesh_volume).abs() < mesh_volume * 1e-6);
        assert!((mesh_volume - crate::measure(&filleted).unwrap().volume).abs() < 1.0);
    }

    #[test]
    fn a_mesh_rounded_like_f32_converts_without_asking_for_one_plane() {
        // A slicer's STL or 3MF rounds coordinates to f32, so a flat face's points are no longer
        // exactly on one plane: they stay one facet per triangle, rather than fusing into a face
        // truck refuses ("cannot attach a plane to a wire that is not on one plane").
        let mesh = mesh_of(&crate::box_solid(Vec3::ZERO, Vec3::new(60., 40., 20.)).unwrap());
        let inner = mesh.mesh.as_ref().unwrap();
        let positions: Vec<Vec3> = inner
            .positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let d = 3e-6 * ((i % 5) as f64 - 2.0);
                Vec3::new(p.x + d, p.y - d * 0.5, p.z + d * 0.25)
            })
            .collect();
        let rounded = crate::mesh_body(&positions, &inner.triangles).unwrap();
        let converted = convert_mesh(&rounded, 0.05).unwrap();
        assert!(converted.validity().is_empty(), "{:?}", converted.validity());
        let expected = rounded.mesh.as_ref().unwrap().to_mesh().measure().volume;
        assert!((crate::measure(&converted).unwrap().volume - expected).abs() < expected * 1e-6);
    }

    #[test]
    fn a_freeform_mesh_converts_as_facets_and_keeps_its_volume() {
        let sphere = crate::sphere(Vec3::ZERO, 10.).unwrap();
        let mesh = mesh_at(&sphere, 0.8);
        let source_volume = mesh.mesh.as_ref().unwrap().to_mesh().measure().volume;
        let converted = convert_mesh(&mesh, 0.05).unwrap();
        assert!(!converted.is_mesh());
        assert!(converted.validity().is_empty(), "{:?}", converted.validity());
        assert!((crate::measure(&converted).unwrap().volume - source_volume).abs() < source_volume * 1e-6);
        assert!(converted.faces(0.01).unwrap().len() > 4, "a curved surface stays faceted");
    }

    /// A watch that records the progress it is told, and cancels at its Nth check.
    struct Probe {
        checks: std::sync::atomic::AtomicUsize,
        cancel_at: usize,
        last: std::sync::Mutex<(usize, usize)>,
    }

    impl crate::watch::Watch for Probe {
        fn cancelled(&self) -> bool {
            self.checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= self.cancel_at
        }
        fn progress(&self, stage: &'static str, done: usize, total: usize) {
            assert_eq!(stage, "converting");
            *self.last.lock().unwrap() = (done, total);
        }
    }

    #[test]
    fn a_watched_conversion_reports_progress_and_stops_when_cancelled() {
        let mesh = mesh_of(&stepped_block());
        let triangles = mesh.mesh.as_ref().map(|m| m.triangles.len()).unwrap_or(0);
        // It reports its triangles as it goes...
        let probe = std::sync::Arc::new(Probe { checks: Default::default(), cancel_at: usize::MAX, last: Default::default() });
        let converted = crate::watch::watching(probe.clone(), || convert_mesh(&mesh, 0.05)).unwrap();
        assert!(!converted.is_mesh());
        assert_eq!(*probe.last.lock().unwrap(), (triangles, triangles));
        // ...and stops at the next check once the watch cancels it.
        let probe = std::sync::Arc::new(Probe { checks: Default::default(), cancel_at: 0, last: Default::default() });
        let err = crate::watch::watching(probe.clone(), || convert_mesh(&mesh, 0.05)).unwrap_err().to_string();
        assert!(err.contains("mesh conversion cancelled"), "{err}");
        assert_eq!(probe.last.lock().unwrap().1, triangles, "it said how much work it had taken on");
    }

    #[test]
    fn a_mesh_wound_inwards_is_turned_outward_before_converting() {
        let mesh = mesh_of(&crate::box_solid(Vec3::ZERO, Vec3::new(10., 10., 10.)).unwrap());
        let inner = mesh.mesh.as_ref().unwrap();
        let flipped: Vec<[u32; 3]> = inner.triangles.iter().map(|[a, b, c]| [*a, *c, *b]).collect();
        let inside_out = crate::mesh_body(&inner.positions, &flipped).unwrap();
        let converted = convert_mesh(&inside_out, 0.05).unwrap();
        assert!(!converted.is_mesh());
        assert!((crate::measure(&converted).unwrap().volume - 1000.).abs() < 1e-6);
    }

    #[test]
    fn refuses_open_nonprismatic_and_hostile_inputs() {
        let open = crate::mesh_body(&[Vec3::ZERO, Vec3::X, Vec3::Y], &[[0, 1, 2]]).unwrap();
        assert!(convert_prismatic_mesh(&open, 0.05).is_err());
        let solid = crate::box_solid(Vec3::ZERO, Vec3::new(10., 10., 10.)).unwrap();
        assert!(convert_prismatic_mesh(&solid, 0.05).is_err());
        for tolerance in [f64::NAN, f64::INFINITY, -1., 0., 2.] {
            assert!(convert_prismatic_mesh(&mesh_of(&solid), tolerance).is_err());
        }
        let sphere = crate::sphere(Vec3::ZERO, 10.).unwrap();
        assert!(convert_prismatic_mesh(&mesh_of(&sphere), 0.05).is_err());
        for coordinate in [f64::NAN, f64::INFINITY, f64::MAX, 1e30] {
            let mesh = crate::meshbody::TriMesh { positions: vec![Vec3::new(coordinate, 0., 0.)], triangles: vec![] };
            assert!(weld(&mesh, 0.05).is_err());
        }
    }

    #[test]
    fn a_mesh_over_the_budget_says_how_big_it_is() {
        // #346: a mesh just over the limit (a strip of 50001 triangles) is refused with its size
        // and the limit, so the reason is clear.
        let n = MAX_TRIANGLES + 1;
        let positions: Vec<Vec3> = (0..n + 2).map(|k| Vec3::new((k / 2) as f64, (k % 2) as f64, 0.)).collect();
        let triangles: Vec<[u32; 3]> = (0..n as u32).map(|i| [i, i + 1, i + 2]).collect();
        let body = crate::mesh_body(&positions, &triangles).unwrap();
        let err = convert_mesh(&body, 0.05).unwrap_err().to_string();
        assert!(err.contains("50001 triangles") && err.contains("50000"), "{err}");
        assert!(err.contains("simplify"), "{err}");
        let mesh = crate::meshbody::TriMesh { positions: vec![Vec3::ZERO; MAX_TRIANGLES * 3 + 1], triangles: vec![] };
        let err = weld(&mesh, 0.05).unwrap_err().to_string();
        assert!(err.contains("150001 vertices") && err.contains("150000"), "{err}");
    }

    #[test]
    fn fitting_cannot_open_a_hole_through_a_nearby_wall() {
        let outer = Loop2::polygon(&[Vec2::new(-1., -1.), Vec2::new(1., -1.), Vec2::new(1., 1.), Vec2::new(-1., 1.)]);
        let points: Vec<_> = (0..12)
            .map(|i| {
                let angle = (i as f64 + 0.5) * std::f64::consts::TAU / 12.;
                Vec2::new(angle.cos(), angle.sin()) * 1.02
            })
            .collect();
        let inner = Loop2::polygon(&points);
        let fitted = fit_loop(&points, 0.05, 1e-6);
        assert_eq!(fitted.segs.len(), 2);
        assert!(certify_nesting(&[outer.clone(), inner], &[outer, fitted], 1e-6, 3.).is_err());
    }

    fn perforated_plate(rows: usize, cols: usize) -> Body {
        let w = cols as f64 * 4. + 2.;
        let h = rows as f64 * 4. + 2.;
        let holes = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .map(|(r, c)| {
                let (x, y) = (c as f64 * 4. + 2., r as f64 * 4. + 2.);
                Loop2::polygon(&[Vec2::new(x, y), Vec2::new(x + 2., y), Vec2::new(x + 2., y + 2.), Vec2::new(x, y + 2.)]).reversed()
            })
            .collect();
        let outer = Loop2::polygon(&[Vec2::new(0., 0.), Vec2::new(w, 0.), Vec2::new(w, h), Vec2::new(0., h)]);
        crate::extrude_planar_regions(&Plane::XY, &[Region2 { outer, holes }], 0., 3.).unwrap()
    }

    #[test]
    fn many_holes_are_refused_promptly_instead_of_stalling() {
        // 400 holes would be ~80k loop pairs and ~160k planar booleans.
        let mesh = mesh_of(&perforated_plate(20, 20));
        let start = std::time::Instant::now();
        let err = convert_prismatic_mesh(&mesh, 0.05).unwrap_err().to_string();
        assert!(err.contains("too complex to convert"), "{err}");
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "took {:?}", start.elapsed());
        // At the limit, a plate still converts in bounded time.
        let mesh = mesh_of(&perforated_plate(7, 7));
        let result = convert_prismatic_mesh(&mesh, 0.05).unwrap();
        assert!((crate::measure(&result).unwrap().volume - (30. * 30. - 49. * 4.) * 3.).abs() < 1e-6);
    }
}
