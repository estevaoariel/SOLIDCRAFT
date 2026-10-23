//! A closed triangle mesh to solid cells: rays along +x through every (y, z) column of cell
//! centres cross the surface an even number of times, and cells whose centres lie between an
//! odd crossing and the next one are inside the body.

use solidcraft_geom::{Mesh, Vec3};

use crate::lbm::{C, WallLink};
use crate::{Cell, Grid, Result, SimError};

/// Ray offsets in cell sizes. Cell centres often lie exactly on a triangle edge (a box's
/// diagonal) or vertex, where a ray would count one crossing twice or not at all; shifting every
/// ray by the same small, irrational-looking amount keeps it off those lines for the meshes a
/// CAD kernel makes.
const NUDGE: (f64, f64) = (1.234_567e-6, 2.718_281e-6);

/// Mark the cells inside `mesh` as solid (fluid cells elsewhere are left as they are).
pub fn voxelize(mesh: &Mesh, grid: &mut Grid) -> Result<()> {
    if mesh.triangles.is_empty() {
        return Err(SimError::EmptyMesh);
    }
    let [nx, ny, nz] = grid.dims;
    let s = grid.spacing;
    let (oy, oz) = (grid.origin.y + (0.5 + NUDGE.0) * s, grid.origin.z + (0.5 + NUDGE.1) * s);
    // Crossing x per column (j + ny * k).
    let mut crossings: Vec<Vec<f64>> = vec![Vec::new(); ny * nz];
    for t in &mesh.triangles {
        let [a, b, c] = mesh.tri(t).ok_or(SimError::BadMesh)?;
        // Twice the signed area of the triangle seen along x; zero for triangles parallel to x.
        let area = (b.y - a.y) * (c.z - a.z) - (c.y - a.y) * (b.z - a.z);
        if !area.is_finite() || area.abs() < 1e-18 {
            continue;
        }
        let column = |v: f64, o: f64, n: usize| ((v - o) / s).clamp(-1.0, n as f64);
        let (j0, j1) = (column(a.y.min(b.y).min(c.y), oy, ny).ceil(), column(a.y.max(b.y).max(c.y), oy, ny).floor());
        let (k0, k1) = (column(a.z.min(b.z).min(c.z), oz, nz).ceil(), column(a.z.max(b.z).max(c.z), oz, nz).floor());
        if j1 < j0 || k1 < k0 {
            continue;
        }
        for k in (k0.max(0.0) as usize)..=(k1.min(nz as f64 - 1.0) as usize) {
            let z = oz + k as f64 * s;
            for j in (j0.max(0.0) as usize)..=(j1.min(ny as f64 - 1.0) as usize) {
                let y = oy + j as f64 * s;
                // Barycentric weights of (y, z) in the projected triangle.
                let wb = ((y - a.y) * (c.z - a.z) - (c.y - a.y) * (z - a.z)) / area;
                let wc = ((b.y - a.y) * (z - a.z) - (y - a.y) * (b.z - a.z)) / area;
                let wa = 1.0 - wb - wc;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                if let Some(col) = crossings.get_mut(j + ny * k) {
                    col.push(wa * a.x + wb * b.x + wc * c.x);
                }
            }
        }
    }
    for k in 0..nz {
        for j in 0..ny {
            let Some(col) = crossings.get_mut(j + ny * k) else { continue };
            if col.len() % 2 == 1 {
                return Err(SimError::NotClosed);
            }
            col.sort_by(f64::total_cmp);
            for &[x0, x1] in col.as_chunks::<2>().0 {
                let first = (((x0 - grid.origin.x) / s) - 0.5).ceil().max(0.0) as usize;
                let last = (((x1 - grid.origin.x) / s) - 0.5).floor();
                if last < 0.0 {
                    continue;
                }
                for i in first..=(last as usize).min(nx.saturating_sub(1)) {
                    if let Some(cell) = grid.index(i, j, k).and_then(|idx| grid.cells.get_mut(idx)) {
                        *cell = Cell::Solid;
                    }
                }
            }
        }
    }
    seal(mesh, grid)
}

/// Close the walls the cell centres missed. A wall thinner than a cell can fall between two
/// rows of centres and leave no solid cell, and the air would stream straight through it. So
/// any link between two fluid cells that passes through material (between a pair of its
/// crossings, for more than a sliver: a link grazing an edge passes through none) gets the cell
/// nearer that material made solid: every wall is then at least a cell thick on the lattice.
fn seal(mesh: &Mesh, grid: &mut Grid) -> Result<()> {
    let [nx, ny, nz] = grid.dims;
    let s = grid.spacing;
    let centre = |i: usize, j: usize, k: usize| {
        Vec3::new(grid.origin.x + (i as f64 + 0.5) * s, grid.origin.y + (j as f64 + 0.5) * s, grid.origin.z + (k as f64 + 0.5) * s)
    };
    let origin = grid.origin;
    // Crossings per link (the cell it starts from, the direction), as fractions along it.
    let mut crossings: std::collections::HashMap<(usize, usize), Vec<f64>> = std::collections::HashMap::new();
    for t in &mesh.triangles {
        let [a, b, c] = mesh.tri(t).ok_or(SimError::BadMesh)?;
        let span = |lo: f64, hi: f64, o: f64, n: usize| {
            let first = ((lo - o) / s - 1.5).floor().max(0.0);
            let last = ((hi - o) / s + 0.5).ceil().min(n as f64 - 1.0);
            (first as usize, last.max(-1.0) as i64)
        };
        let (i0, i1) = span(a.x.min(b.x).min(c.x), a.x.max(b.x).max(c.x), origin.x, nx);
        let (j0, j1) = span(a.y.min(b.y).min(c.y), a.y.max(b.y).max(c.y), origin.y, ny);
        let (k0, k1) = span(a.z.min(b.z).min(c.z), a.z.max(b.z).max(c.z), origin.z, nz);
        if i1 < 0 || j1 < 0 || k1 < 0 {
            continue;
        }
        for k in k0..=k1 as usize {
            for j in j0..=j1 as usize {
                for i in i0..=i1 as usize {
                    let Some(here) = grid.index(i, j, k) else { continue };
                    if grid.cells.get(here) != Some(&Cell::Fluid) {
                        continue;
                    }
                    // One of each pair of opposite directions: each link once.
                    for (q, e) in C.iter().enumerate().skip(1).step_by(2) {
                        let (u, v, w) = (i as i64 + i64::from(e[0]), j as i64 + i64::from(e[1]), k as i64 + i64::from(e[2]));
                        if u < 0 || v < 0 || w < 0 {
                            continue;
                        }
                        let Some(there) = grid.index(u as usize, v as usize, w as usize) else { continue };
                        if grid.cells.get(there) != Some(&Cell::Fluid) {
                            continue;
                        }
                        let dir = Vec3::new(f64::from(e[0]) * s, f64::from(e[1]) * s, f64::from(e[2]) * s);
                        if let Some(t) = crossing(centre(i, j, k), dir, [a, b, c]) {
                            crossings.entry((here, q)).or_default().push(t);
                        }
                    }
                }
            }
        }
    }
    let mut fill = Vec::new();
    for ((here, q), mut ts) in crossings {
        ts.sort_by(f64::total_cmp);
        // A crossing two triangles share (on their common edge) counts once.
        ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        // Both ends are air, so the crossings come in pairs: in, then out.
        let Some((t0, t1)) = ts.as_chunks::<2>().0.iter().map(|[a, b]| (*a, *b)).find(|(t0, t1)| t1 - t0 > 1e-3) else { continue };
        let Some(e) = C.get(q) else { continue };
        let there = grid.coords(here);
        let (u, v, w) = (there[0] as i64 + i64::from(e[0]), there[1] as i64 + i64::from(e[1]), there[2] as i64 + i64::from(e[2]));
        let Some(there) = (u >= 0 && v >= 0 && w >= 0).then(|| grid.index(u as usize, v as usize, w as usize)).flatten() else { continue };
        fill.push(if (t0 + t1) * 0.5 < 0.5 { here } else { there });
    }
    for cell in fill {
        if let Some(c) = grid.cells.get_mut(cell) {
            *c = Cell::Solid;
        }
    }
    Ok(())
}

/// Where `meshes` really cross the links from fluid cells into solid ones: for each such link
/// that a triangle crosses, how far along it (from the fluid cell's centre towards the solid
/// one's) the nearest crossing lies. Links into cells made solid for other reasons (the box's
/// edges, inlet caps) cross no triangle and are left out, so they keep half-way bounce-back.
pub fn wall_links(meshes: &[&Mesh], grid: &Grid) -> Result<Vec<WallLink>> {
    let [nx, ny, nz] = grid.dims;
    let s = grid.spacing;
    let solid = |i: i64, j: i64, k: i64| -> bool {
        // Past the box is not a wall: the lattice doesn't stream there.
        if i < 0 || j < 0 || k < 0 {
            return false;
        }
        grid.index(i as usize, j as usize, k as usize).and_then(|c| grid.cells.get(c)).is_some_and(|c| *c == Cell::Solid)
    };
    let centre = |i: usize, j: usize, k: usize| {
        Vec3::new(grid.origin.x + (i as f64 + 0.5) * s, grid.origin.y + (j as f64 + 0.5) * s, grid.origin.z + (k as f64 + 0.5) * s)
    };
    let mut nearest: std::collections::HashMap<(usize, usize), f64> = std::collections::HashMap::new();
    for mesh in meshes {
        for t in &mesh.triangles {
            let [a, b, c] = mesh.tri(t).ok_or(SimError::BadMesh)?;
            // The cells whose links (one cell long) can reach this triangle.
            let span = |lo: f64, hi: f64, o: f64, n: usize| {
                let first = ((lo - o) / s - 1.5).floor().max(0.0);
                let last = ((hi - o) / s + 0.5).ceil().min(n as f64 - 1.0);
                (first as usize, last.max(-1.0) as i64)
            };
            let (i0, i1) = span(a.x.min(b.x).min(c.x), a.x.max(b.x).max(c.x), grid.origin.x, nx);
            let (j0, j1) = span(a.y.min(b.y).min(c.y), a.y.max(b.y).max(c.y), grid.origin.y, ny);
            let (k0, k1) = span(a.z.min(b.z).min(c.z), a.z.max(b.z).max(c.z), grid.origin.z, nz);
            if i1 < 0 || j1 < 0 || k1 < 0 {
                continue;
            }
            for k in k0..=k1 as usize {
                for j in j0..=j1 as usize {
                    for i in i0..=i1 as usize {
                        let Some(cell) = grid.index(i, j, k) else { continue };
                        if grid.cells.get(cell) != Some(&Cell::Fluid) {
                            continue;
                        }
                        let from = centre(i, j, k);
                        for (q, e) in C.iter().enumerate().skip(1) {
                            if !solid(i as i64 + i64::from(e[0]), j as i64 + i64::from(e[1]), k as i64 + i64::from(e[2])) {
                                continue;
                            }
                            let dir = Vec3::new(f64::from(e[0]) * s, f64::from(e[1]) * s, f64::from(e[2]) * s);
                            if let Some(frac) = crossing(from, dir, [a, b, c]) {
                                let slot = nearest.entry((cell, q)).or_insert(frac);
                                *slot = slot.min(frac);
                            }
                        }
                    }
                }
            }
        }
    }
    let mut links: Vec<WallLink> = nearest.into_iter().map(|((cell, q), frac)| WallLink { cell, q, frac: frac.clamp(1e-3, 1.0) }).collect();
    links.sort_by_key(|l| (l.cell, l.q));
    Ok(links)
}

/// Where along `from + t·dir` (0 ≤ t ≤ 1) the ray meets the triangle (Möller & Trumbore 1997).
fn crossing(from: Vec3, dir: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f64> {
    let (e1, e2) = (b - a, c - a);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if !det.is_finite() || det.abs() < 1e-18 {
        return None;
    }
    let to = from - a;
    let u = to.dot(p) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qv = to.cross(e1);
    let v = dir.dot(qv) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(qv) / det;
    (t.is_finite() && (0.0..=1.0).contains(&t)).then_some(t)
}

#[cfg(test)]
pub(crate) mod tests {
    use solidcraft_geom::Vec3;
    use solidcraft_geom::{Aabb3, Mesh};

    use super::*;

    /// A closed axis-aligned box, triangles counter-clockwise from outside.
    pub(crate) fn add_box(mesh: &mut Mesh, min: [f64; 3], max: [f64; 3]) {
        let base = mesh.positions.len() as u32;
        for k in 0..8 {
            let pick = |bit: usize, axis: usize| if k & bit == 0 { min[axis] } else { max[axis] };
            mesh.positions.push(Vec3 { x: pick(1, 0), y: pick(2, 1), z: pick(4, 2) });
        }
        let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        for [a, b, c, d] in quads {
            mesh.triangles.push([base + a, base + b, base + c]);
            mesh.triangles.push([base + a, base + c, base + d]);
        }
    }

    /// A square duct along x, 40 long, 20 × 20 outside with 4-thick walls (four boxes that meet
    /// only along faces parallel to x).
    pub(crate) fn square_duct() -> Mesh {
        let mut m = Mesh::default();
        add_box(&mut m, [0.0, 0.0, 0.0], [40.0, 20.0, 4.0]);
        add_box(&mut m, [0.0, 0.0, 16.0], [40.0, 20.0, 20.0]);
        add_box(&mut m, [0.0, 0.0, 4.0], [40.0, 4.0, 16.0]);
        add_box(&mut m, [0.0, 16.0, 4.0], [40.0, 20.0, 16.0]);
        m
    }

    fn bounds(m: &Mesh) -> Aabb3 {
        m.bounds()
    }

    #[test]
    fn a_cube_fills_its_volume_in_cells() {
        let mut m = Mesh::default();
        add_box(&mut m, [0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        let mut g = Grid::around(bounds(&m), 1.0, 2, 1_000_000).unwrap();
        voxelize(&m, &mut g).unwrap();
        assert_eq!(g.dims, [14, 14, 14]);
        assert_eq!(g.count(Cell::Solid), 1000);
        assert_eq!(g.get(2, 2, 2), Some(Cell::Solid));
        assert_eq!(g.get(1, 2, 2), Some(Cell::Fluid));
    }

    #[test]
    fn a_duct_wall_is_solid_and_its_bore_stays_fluid() {
        let m = square_duct();
        let mut g = Grid::around(bounds(&m), 2.0, 2, 1_000_000).unwrap();
        voxelize(&m, &mut g).unwrap();
        // (20 × 20 − 12 × 12) × 40 mm³ of wall in 8 mm³ cells.
        assert_eq!(g.count(Cell::Solid), 1280);
        let mid = |x: f64, y: f64, z: f64| {
            let c = |v: f64, o: f64| ((v - o) / 2.0).floor() as usize;
            g.get(c(x, g.origin.x), c(y, g.origin.y), c(z, g.origin.z))
        };
        assert_eq!(mid(20.0, 10.0, 10.0), Some(Cell::Fluid), "the bore");
        assert_eq!(mid(20.0, 2.0, 10.0), Some(Cell::Solid), "a side wall");
    }

    /// A closed box with walls a quarter of a cell thick: no cell centre lies in them, but the
    /// air inside still never meets the air outside, along any of the lattice's links.
    #[test]
    fn walls_thinner_than_a_cell_still_close() {
        let (o, w) = (20.0, 0.5);
        let mut m = Mesh::default();
        add_box(&mut m, [0.0, 0.0, 0.0], [o, o, w]);
        add_box(&mut m, [0.0, 0.0, o - w], [o, o, o]);
        add_box(&mut m, [0.0, 0.0, w], [o, w, o - w]);
        add_box(&mut m, [0.0, o - w, w], [o, o, o - w]);
        add_box(&mut m, [0.0, w, w], [w, o - w, o - w]);
        add_box(&mut m, [o - w, w, w], [o, o - w, o - w]);
        let mut g = Grid::around(bounds(&m), 2.0, 2, 1_000_000).unwrap();
        voxelize(&m, &mut g).unwrap();
        let c = |v: f64, origin: f64| ((v - origin) / 2.0).floor() as usize;
        let inside = [c(10.0, g.origin.x), c(10.0, g.origin.y), c(10.0, g.origin.z)];
        assert_eq!(g.get(inside[0], inside[1], inside[2]), Some(Cell::Fluid));
        let region = crate::fluid_region(&g, inside, |_, _, _| false).unwrap();
        assert!(!region.touches_edge, "the inside leaks out");
        assert!(region.count >= 512, "the inside stays air: {}", region.count);
    }

    #[test]
    fn an_open_surface_and_hostile_sizes_are_refused() {
        let mut m = Mesh::default();
        add_box(&mut m, [0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        m.triangles.truncate(10);
        let mut g = Grid::around(bounds(&m), 1.0, 1, 1_000_000).unwrap();
        assert_eq!(voxelize(&m, &mut g), Err(SimError::NotClosed));
        for spacing in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(Grid::around(bounds(&m), spacing, 1, 1_000).err(), Some(SimError::InvalidSpacing), "{spacing}");
        }
        assert!(matches!(Grid::around(bounds(&m), 1e-6, 1, 1_000_000), Err(SimError::TooManyCells { .. })));
        m.triangles.push([0, 1, 999]);
        assert_eq!(voxelize(&m, &mut g), Err(SimError::BadMesh));
        assert_eq!(voxelize(&Mesh::default(), &mut g), Err(SimError::EmptyMesh));
    }
}
