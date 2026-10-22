//! Stock-size math (#137): the eigen-solver for inertia tensors and the box of a point set along
//! given axes. Every expected value is worked out by hand.

use super::*;

const TOL: f64 = 1e-9;

fn rot_z(deg: f64) -> [Vec3; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    [Vec3::new(c, s, 0.0), Vec3::new(-s, c, 0.0), Vec3::new(0.0, 0.0, 1.0)]
}

/// R · diag(d) · Rᵀ, with R's columns the given axes.
fn tensor(axes: [Vec3; 3], d: [f64; 3]) -> [[f64; 3]; 3] {
    let col = |v: Vec3| [v.x, v.y, v.z];
    let mut m = [[0.0; 3]; 3];
    for (axis, value) in axes.into_iter().zip(d) {
        let a = col(axis);
        for (i, row) in m.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell += value * a[i] * a[j];
            }
        }
    }
    m
}

fn orthonormal(axes: [Vec3; 3]) -> bool {
    let unit = axes.iter().all(|a| (a.dot(*a) - 1.0).abs() < 1e-9);
    let square = (0..3).all(|i| ((i + 1)..3).all(|j| axes[i].dot(axes[j]).abs() < 1e-9));
    unit && square
}

fn corners(l: f64, w: f64, t: f64, axes: [Vec3; 3]) -> Vec<Vec3> {
    let mut out = Vec::new();
    for a in [0.0, l] {
        for b in [0.0, w] {
            for c in [0.0, t] {
                out.push(axes[0] * a + axes[1] * b + axes[2] * c);
            }
        }
    }
    out
}

/// A diagonal tensor has its diagonal as eigenvalues, along the world axes.
#[test]
fn diagonal_tensor_gives_its_diagonal() {
    let Some((values, axes)) = eigen_sym3([[3.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 1.0]]) else {
        panic!("no eigen-decomposition");
    };
    assert!((values[0] - 3.0).abs() < TOL && (values[1] - 2.0).abs() < TOL && (values[2] - 1.0).abs() < TOL, "{values:?}");
    assert!((axes[0].x.abs() - 1.0).abs() < TOL && (axes[1].y.abs() - 1.0).abs() < TOL && (axes[2].z.abs() - 1.0).abs() < TOL, "{axes:?}");
}

/// R · diag(3, 2, 1) · Rᵀ with R = 30° about Z gives the same eigenvalues along R's columns.
#[test]
fn rotated_tensor_gives_the_rotation_axes() {
    let r = rot_z(30.0);
    let Some((values, axes)) = eigen_sym3(tensor(r, [3.0, 2.0, 1.0])) else { panic!("no eigen-decomposition") };
    assert!((values[0] - 3.0).abs() < 1e-9 && (values[1] - 2.0).abs() < 1e-9 && (values[2] - 1.0).abs() < 1e-9, "{values:?}");
    for (got, want) in axes.iter().zip(r) {
        assert!((got.dot(want).abs() - 1.0).abs() < 1e-9, "{got:?} is not along {want:?}");
    }
}

/// Two equal eigenvalues still give an orthonormal basis, and the same one every run.
#[test]
fn repeated_eigenvalues_give_one_stable_basis() {
    let m = [[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 1.0]];
    let (Some(a), Some(b)) = (eigen_sym3(m), eigen_sym3(m)) else { panic!("no eigen-decomposition") };
    assert!(orthonormal(a.1), "{:?}", a.1);
    assert_eq!(a, b);
}

/// A zero tensor (a point, or no mass) has a defined answer, not a panic.
#[test]
fn zero_tensor_has_a_defined_answer() {
    let Some((values, axes)) = eigen_sym3([[0.0; 3]; 3]) else { panic!("no eigen-decomposition") };
    assert_eq!(values, [0.0; 3]);
    assert!(orthonormal(axes), "{axes:?}");
}

/// Non-finite input is refused.
#[test]
fn non_finite_tensors_are_refused() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut m = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        m[0][1] = bad;
        m[1][0] = bad;
        assert!(eigen_sym3(m).is_none(), "{bad} was accepted");
    }
}

/// The corners of a 100 × 50 × 18 box turned 30° about Z, measured along that turn's axes.
#[test]
fn box_along_its_own_axes_is_its_size() {
    let r = rot_z(30.0);
    let Some(b) = box_along(&corners(100.0, 50.0, 18.0, r), r) else { panic!("no box") };
    assert!((b.length - 100.0).abs() < TOL && (b.width - 50.0).abs() < TOL && (b.thickness - 18.0).abs() < TOL, "{b:?}");
    assert!((b.axes[0].dot(r[0]).abs() - 1.0).abs() < TOL, "length should run along the 100 mm side: {b:?}");
}

/// The same corners along the world axes give the world box, 100·cos 30° + 50·sin 30° by
/// 100·sin 30° + 50·cos 30° by 18: wrong axes still enclose the part.
#[test]
fn box_along_world_axes_encloses() {
    let world = [Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)];
    let Some(b) = box_along(&corners(100.0, 50.0, 18.0, rot_z(30.0)), world) else { panic!("no box") };
    let (s, c) = 30f64.to_radians().sin_cos();
    assert!((b.length - (100.0 * c + 50.0 * s)).abs() < 1e-9, "{b:?}");
    assert!((b.width - (100.0 * s + 50.0 * c)).abs() < 1e-9, "{b:?}");
    assert!((b.thickness - 18.0).abs() < 1e-9, "{b:?}");
}

/// Sizes come out longest first whatever order the axes are given in.
#[test]
fn sizes_are_sorted_longest_first() {
    let world = [Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)];
    let Some(b) = box_along(&corners(18.0, 591.6, 878.4, [world[1], world[2], world[0]]), world) else { panic!("no box") };
    assert!((b.length - 878.4).abs() < TOL && (b.width - 591.6).abs() < TOL && (b.thickness - 18.0).abs() < TOL, "{b:?}");
}

/// No points, non-finite points or axes that are not orthonormal are refused.
#[test]
fn bad_inputs_are_refused() {
    let world = [Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)];
    assert!(box_along(&[], world).is_none());
    assert!(box_along(&[Vec3::new(f64::NAN, 0.0, 0.0)], world).is_none());
    let skew = [Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)];
    assert!(box_along(&corners(1.0, 1.0, 1.0, world), skew).is_none());
}

/// Seen along Z, a box turned 30° in plan is found at its own size by its hull's edges.
#[test]
fn tightest_box_finds_a_turned_box() {
    let r = rot_z(30.0);
    let up = [Vec3::new(0.0, 0.0, 1.0)];
    let Some((b, used)) = tightest_box(&corners(100.0, 50.0, 18.0, r), &up) else { panic!("no box") };
    assert_eq!(used, 0);
    assert!((b.length - 100.0).abs() < TOL && (b.width - 50.0).abs() < TOL && (b.thickness - 18.0).abs() < TOL, "{b:?}");
}

/// Of several directions, the one giving the smallest box wins; a looser one never does.
#[test]
fn tightest_box_keeps_the_smallest() {
    let r = rot_z(30.0);
    let tilted = Vec3::new(1.0, 1.0, 1.0);
    let Some((b, used)) = tightest_box(&corners(100.0, 50.0, 18.0, r), &[tilted, Vec3::new(0.0, 0.0, 1.0)]) else { panic!("no box") };
    assert_eq!(used, 1, "{b:?}");
    assert!((b.volume() - 100.0 * 50.0 * 18.0).abs() < 1e-6, "{b:?}");
}

/// No points or no usable direction gives no box.
#[test]
fn tightest_box_needs_points_and_a_direction() {
    let pts = corners(1.0, 1.0, 1.0, rot_z(0.0));
    assert!(tightest_box(&[], &[Vec3::new(0.0, 0.0, 1.0)]).is_none());
    assert!(tightest_box(&pts, &[Vec3::ZERO]).is_none());
    assert!(tightest_box(&pts, &[]).is_none());
}
