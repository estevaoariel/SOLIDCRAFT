//! Stock size (#137): the tightest box around a part along its own directions, as the board or
//! sheet it is cut from. Sizes are longest first (length ≥ width ≥ thickness), never world X/Y/Z.
//!
//! [`tightest_box`] tries each candidate direction as one side of the box (a face normal, or a
//! principal axis of inertia from [`eigen_sym3`]) and finds the smallest rectangle across it by
//! the edges of the points' convex hull: exact for a part bounded by flat faces, since one side
//! of its tightest box lies along one of those faces.

use crate::{Vec2, Vec3};

/// The most points or candidate directions [`tightest_box`] looks at.
pub const MAX_POINTS: usize = 2_000_000;
pub const MAX_DIRECTIONS: usize = 64;

/// The box a part is cut from: its sizes longest first, the unit direction of each (length,
/// width, thickness, right-handed) and the box's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StockBox {
    pub length: f64,
    pub width: f64,
    pub thickness: f64,
    pub axes: [Vec3; 3],
    pub center: Vec3,
}

impl StockBox {
    pub fn volume(&self) -> f64 {
        self.length * self.width * self.thickness
    }
}

/// Eigenvalues (largest first) and unit eigenvectors of a symmetric 3 × 3 matrix, such as an
/// inertia tensor, by cyclic Jacobi rotations. The vectors are right-handed, and each points
/// along the positive side of its largest component, so the answer is the same every run.
/// `None` for non-finite input.
pub fn eigen_sym3(m: [[f64; 3]; 3]) -> Option<([f64; 3], [Vec3; 3])> {
    if m.iter().flatten().any(|x| !x.is_finite()) {
        return None;
    }
    let mut a = [[0.0; 3]; 3];
    for (i, row) in a.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = 0.5 * (m[i][j] + m[j][i]);
        }
    }
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..64 {
        let off = a[0][1] * a[0][1] + a[0][2] * a[0][2] + a[1][2] * a[1][2];
        let diag = a[0][0] * a[0][0] + a[1][1] * a[1][1] + a[2][2] * a[2][2];
        if off <= 1e-30 * (diag + off) || off == 0.0 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q] == 0.0 {
                continue;
            }
            // The rotation that zeroes a[p][q] (A' = Jᵀ·A·J).
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            let mut j = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            j[p][p] = c;
            j[q][q] = c;
            j[p][q] = s;
            j[q][p] = -s;
            a = mul(transpose(j), mul(a, j));
            v = mul(v, j);
        }
    }
    let mut pairs: Vec<(f64, Vec3)> = (0..3).map(|k| (a[k][k], canonical(Vec3::new(v[0][k], v[1][k], v[2][k])))).collect();
    pairs.sort_by(|x, y| y.0.total_cmp(&x.0));
    let values = [pairs[0].0, pairs[1].0, pairs[2].0];
    let (x, y) = (pairs[0].1, pairs[1].1);
    if !values.iter().all(|v| v.is_finite()) {
        return None;
    }
    Some((values, [x, y, x.cross(y)]))
}

/// The box of `points` along three orthonormal `axes`, its sizes sorted longest first with their
/// axes (made right-handed). `None` for no points, non-finite values or axes that are not
/// orthonormal.
pub fn box_along(points: &[Vec3], axes: [Vec3; 3]) -> Option<StockBox> {
    if points.is_empty() || points.len() > MAX_POINTS || !orthonormal(axes) {
        return None;
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in points {
        if !p.is_finite() {
            return None;
        }
        for k in 0..3 {
            let d = p.dot(axes[k]);
            lo[k] = lo[k].min(d);
            hi[k] = hi[k].max(d);
        }
    }
    let mut sides: Vec<(f64, Vec3, f64)> = (0..3).map(|k| (hi[k] - lo[k], axes[k], 0.5 * (lo[k] + hi[k]))).collect();
    let center = sides.iter().fold(Vec3::ZERO, |c, (_, axis, mid)| c + *axis * *mid);
    sides.sort_by(|x, y| y.0.total_cmp(&x.0));
    let (a, b) = (sides[0].1, sides[1].1);
    let b = StockBox { length: sides[0].0, width: sides[1].0, thickness: sides[2].0, axes: [a, b, a.cross(b)], center };
    (b.volume().is_finite() && center.is_finite()).then_some(b)
}

/// The smallest box around `points` with one side across one of `directions` (unit or not): for
/// each direction, the tightest rectangle of the points seen along it, then the smallest volume.
/// Returns the box and the index of the direction it used (the first on a tie). `None` for no
/// usable points or directions.
pub fn tightest_box(points: &[Vec3], directions: &[Vec3]) -> Option<(StockBox, usize)> {
    if points.is_empty() || points.len() > MAX_POINTS || points.iter().any(|p| !p.is_finite()) {
        return None;
    }
    let mut best: Option<(StockBox, usize)> = None;
    for (i, d) in directions.iter().enumerate().take(MAX_DIRECTIONS) {
        let Some(n) = d.normalized() else { continue };
        let Some(b) = box_across(points, n) else { continue };
        if best.as_ref().is_none_or(|(old, _)| b.volume() < old.volume() * (1.0 - 1e-12)) {
            best = Some((b, i));
        }
    }
    best
}

/// The tightest box with one side across `n`: the points seen along `n`, their convex hull, and
/// the smallest rectangle with a side along one of the hull's edges.
fn box_across(points: &[Vec3], n: Vec3) -> Option<StockBox> {
    let helper = if n.x.abs() <= n.y.abs() && n.x.abs() <= n.z.abs() {
        Vec3::new(1.0, 0.0, 0.0)
    } else if n.y.abs() <= n.z.abs() {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let u = helper.cross(n).normalized()?;
    let v = n.cross(u);
    let flat: Vec<Vec2> = points.iter().map(|p| Vec2::new(p.dot(u), p.dot(v))).collect();
    let hull = convex_hull(flat);
    let mut best: Option<(f64, Vec2)> = None;
    let edges: Vec<Vec2> = if hull.len() < 2 {
        vec![Vec2::new(1.0, 0.0)]
    } else {
        (0..hull.len()).filter_map(|k| (hull[(k + 1) % hull.len()] - hull[k]).normalized()).collect()
    };
    for e in edges {
        let w = Vec2::new(-e.y, e.x);
        let (mut a0, mut a1, mut b0, mut b1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for p in &hull {
            let (a, b) = (p.dot(e), p.dot(w));
            a0 = a0.min(a);
            a1 = a1.max(a);
            b0 = b0.min(b);
            b1 = b1.max(b);
        }
        let area = (a1 - a0).max(0.0) * (b1 - b0).max(0.0);
        if best.is_none_or(|(old, _)| area < old * (1.0 - 1e-12)) {
            best = Some((area, e));
        }
    }
    let (_, e) = best?;
    let side = (u * e.x + v * e.y).normalized()?;
    box_along(points, [side, n.cross(side), n])
}

/// The convex hull of 2D points, counter-clockwise, without repeated or collinear points
/// (Andrew's monotone chain).
fn convex_hull(mut pts: Vec<Vec2>) -> Vec<Vec2> {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let turn = |o: Vec2, a: Vec2, b: Vec2| (a - o).cross(b - o);
    let mut hull: Vec<Vec2> = Vec::with_capacity(2 * pts.len());
    for pass in 0..2 {
        let start = hull.len();
        let ordered: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for &p in ordered {
            while hull.len() >= start + 2 && turn(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

/// A unit vector pointing along the positive side of its largest component (the first on a tie).
fn canonical(v: Vec3) -> Vec3 {
    let k = if v.x.abs() >= v.y.abs() && v.x.abs() >= v.z.abs() {
        v.x
    } else if v.y.abs() >= v.z.abs() {
        v.y
    } else {
        v.z
    };
    let v = v.normalized().unwrap_or(Vec3::new(1.0, 0.0, 0.0));
    if k < 0.0 { v * -1.0 } else { v }
}

fn orthonormal(axes: [Vec3; 3]) -> bool {
    let unit = axes.iter().all(|a| a.is_finite() && (a.dot(*a) - 1.0).abs() < 1e-9);
    unit && axes[0].dot(axes[1]).abs() < 1e-9 && axes[0].dot(axes[2]).abs() < 1e-9 && axes[1].dot(axes[2]).abs() < 1e-9
}

type M3 = [[f64; 3]; 3];

fn mul(a: M3, b: M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn transpose(a: M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[j][i];
        }
    }
    out
}

#[cfg(test)]
#[path = "stock_tests.rs"]
mod tests;
