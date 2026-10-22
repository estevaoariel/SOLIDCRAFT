//! Stock size (#137): a body's length × width × thickness along its own directions, as the
//! board or sheet it is cut from. The candidate directions are the normals of its flat faces,
//! largest first, then the principal axes of its surface; the box is the smallest one with a
//! side across one of them ([`solidcraft_geom::stock::tightest_box`]).
//!
//! A body bounded only by flat faces gets an exact box: its tessellation's vertices are its
//! corners. A curved or mesh body gets an approximate one, widened by the chord tolerance on
//! every side so that it still encloses the part.

use serde::Serialize;
use solidcraft_geom::Vec3;
use solidcraft_geom::stock::{MAX_DIRECTIONS, StockBox, eigen_sym3, tightest_box};

use crate::body::Body;
use crate::{KernelError, Result};

/// The most faces whose planes are read for candidate directions.
const MAX_FACES: usize = 50_000;

/// A body's stock size and how it was found.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Stock {
    pub length: f64,
    pub width: f64,
    pub thickness: f64,
    /// Unit directions of the length, width and thickness (right-handed).
    pub axes: [Vec3; 3],
    pub center: Vec3,
    /// True when every face is flat: the box is the part's tightest box, not an estimate.
    pub exact: bool,
    /// `faces` (across a flat face) or `principal_axes` (curved parts with no usable face).
    pub method: &'static str,
}

/// The stock size of `b`, in `b`'s own coordinates (place the body first for world axes).
pub fn stock(b: &Body) -> Result<Stock> {
    let mesh_body = b.triangle_mesh().is_some();
    let tol = (b.size() * 1e-4).max(1e-3);
    let mesh = if mesh_body { b.tessellate(1.0)? } else { b.tessellate_with(tol, true)? };
    let faces = b.face_count();
    let planes: Vec<Option<solidcraft_geom::Plane>> =
        if mesh_body || faces > MAX_FACES { Vec::new() } else { (0..faces).map(|i| b.planar_face(i)).collect() };
    let exact = !mesh_body && faces > 0 && planes.len() == faces && planes.iter().all(Option::is_some);

    // Face areas from the tessellation, and the surface's second moments about its centre.
    let mut area = vec![0.0; planes.len()];
    let (mut total, mut first) = (0.0, Vec3::ZERO);
    let mut tris: Vec<(f64, Vec3)> = Vec::with_capacity(mesh.triangles.len());
    for (t, f) in mesh.triangles.iter().zip(&mesh.tri_face) {
        let Some([p, q, r]) = mesh.tri(t) else { continue };
        let a = (q - p).cross(r - p).len() * 0.5;
        if let Some(slot) = area.get_mut(*f as usize) {
            *slot += a;
        }
        let c = (p + q + r) * (1.0 / 3.0);
        total += a;
        first += c * a;
        tris.push((a, c));
    }

    // Flat faces' normals, parallel ones merged, the largest total area first.
    let mut normals: Vec<(Vec3, f64)> = Vec::new();
    for (plane, a) in planes.iter().zip(&area) {
        let Some(n) = plane.as_ref().and_then(|p| p.normal().normalized()) else { continue };
        match normals.iter_mut().find(|(m, _)| m.dot(n).abs() > 1.0 - 1e-9) {
            Some(m) => m.1 += a,
            None => normals.push((n, *a)),
        }
    }
    normals.sort_by(|x, y| y.1.total_cmp(&x.1));
    let mut directions: Vec<Vec3> = normals.iter().map(|(n, _)| *n).take(MAX_DIRECTIONS - 3).collect();
    let from_faces = directions.len();
    if total > 0.0 {
        let centre = first * (1.0 / total);
        let mut m = [[0.0; 3]; 3];
        for (a, c) in &tris {
            let d = [c.x - centre.x, c.y - centre.y, c.z - centre.z];
            for (i, row) in m.iter_mut().enumerate() {
                for (j, cell) in row.iter_mut().enumerate() {
                    *cell += a * d[i] * d[j];
                }
            }
        }
        if let Some((_, axes)) = eigen_sym3(m) {
            directions.extend(axes);
        }
    }
    if directions.is_empty() {
        directions.extend([Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)]);
    }
    let Some((found, used)) = tightest_box(&mesh.positions, &directions) else {
        return Err(KernelError::Invalid("the body has no measurable size".into()));
    };
    let StockBox { length, width, thickness, axes, center } = found;
    // An approximate box is widened by the chord tolerance on each side a curved face may bound,
    // so it encloses the part; a side lying on a flat face needs no margin.
    let margin = |k: usize, size: f64| {
        if exact || mesh_body {
            return 0.0;
        }
        let axis = axes[k];
        let mid = center.dot(axis);
        let flat = |at: f64| {
            planes.iter().flatten().any(|p| {
                let n = p.normal().normalized().unwrap_or(Vec3::ZERO);
                n.dot(axis).abs() > 1.0 - 1e-9 && (p.origin.dot(axis) - at).abs() <= 1e-9 * (1.0 + at.abs())
            })
        };
        [mid - size / 2.0, mid + size / 2.0].into_iter().filter(|at| !flat(*at)).count() as f64 * tol
    };
    let stock = Stock {
        length: length + margin(0, length),
        width: width + margin(1, width),
        thickness: thickness + margin(2, thickness),
        axes,
        center,
        exact,
        method: if used < from_faces { "faces" } else { "principal_axes" },
    };
    if !(stock.length.is_finite() && stock.width.is_finite() && stock.thickness.is_finite()) || stock.length <= 0.0 {
        return Err(KernelError::Invalid("the body has no measurable size".into()));
    }
    Ok(stock)
}
