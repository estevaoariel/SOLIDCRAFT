//! Point clouds: scanned points (a PLY or XYZ file) kept in the design as a reference to model
//! from (#5151). They are not bodies: nothing is built from them, they are drawn as points.
//! The points travel inside the design file (a binary part of the container, base64 in plain
//! JSON), so it reopens without the original file. They are single precision, relative to the
//! cloud's own `origin` (double precision), so a scan far from the design's origin keeps its
//! detail.

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use solidcraft_geom::Vec3;

use crate::canvas::{base64_decode, base64_encode};
use crate::{DocError, IDENTITY, Mat, Result};

/// Most points one cloud may hold.
pub const MAX_POINTS: usize = 20_000_000;
/// Largest point coordinate (mm) from the cloud's origin, and largest origin coordinate.
pub const MAX_COORD: f64 = 1e9;
/// Point sizes on screen (pixels).
pub const POINT_SIZE: std::ops::RangeInclusive<f64> = 0.5..=16.0;
/// Length units a point cloud file may be in, and millimetres per unit.
pub const UNITS: [(&str, f64); 5] = [("mm", 1.0), ("cm", 10.0), ("m", 1000.0), ("in", 25.4), ("ft", 304.8)];

/// Millimetres per `unit` (one of [`UNITS`], any case).
pub fn unit_scale(unit: &str) -> Option<f64> {
    UNITS.iter().find(|(u, _)| u.eq_ignore_ascii_case(unit)).map(|(_, k)| *k)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointCloud {
    pub id: u64,
    pub name: String,
    /// The file it was read from (its name only).
    #[serde(default)]
    pub file: String,
    /// Where the points' own frame sits in the cloud's placement (mm).
    pub origin: Vec3,
    /// The points from `origin` (mm).
    pub points: Points,
    /// The file's colour of each point, when it has them.
    #[serde(default, skip_serializing_if = "Colors::is_empty")]
    pub colors: Colors,
    /// Points the file had (more than `points` when the cloud was thinned as it came in).
    #[serde(default)]
    pub source_points: u64,
    /// The file's length unit (one of [`UNITS`]): the points were scaled from it to millimetres.
    #[serde(default = "mm")]
    pub units: String,
    /// Where the cloud sits in the design: a rigid move of its points.
    #[serde(default = "identity", skip_serializing_if = "is_identity")]
    pub transform: Mat,
    /// Size of a point on screen (pixels).
    #[serde(default = "two")]
    pub point_size: f64,
    /// One colour for every point (straight sRGB, 0..1) instead of the file's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    #[serde(default = "yes")]
    pub visible: bool,
}

fn identity() -> Mat {
    IDENTITY
}
fn is_identity(m: &Mat) -> bool {
    *m == IDENTITY
}
fn mm() -> String {
    "mm".into()
}
fn two() -> f64 {
    2.0
}
fn yes() -> bool {
    true
}

impl PointCloud {
    pub fn len(&self) -> usize {
        self.points.len()
    }
    pub fn is_empty(&self) -> bool {
        self.points.len() == 0
    }
    /// The `i`th point in the design (mm).
    pub fn world_point(&self, i: usize) -> Option<Vec3> {
        let p = self.points.get(i)?;
        let local = self.origin + Vec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
        Some(crate::apply_point(&self.transform, local))
    }
    /// The corners of the box around the points, in the cloud's placement (mm), or `None` for
    /// an empty cloud.
    pub fn local_bounds(&self) -> Option<(Vec3, Vec3)> {
        let (lo, hi) = self.points.bounds()?;
        let v = |p: [f32; 3]| self.origin + Vec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
        Some((v(lo), v(hi)))
    }
    /// The corners of the box around the points, placed in the design (mm).
    pub fn world_corners(&self) -> Option<[Vec3; 8]> {
        let (lo, hi) = self.local_bounds()?;
        Some(std::array::from_fn(|i| {
            let q = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
            crate::apply_point(&self.transform, q)
        }))
    }
    /// The cloud is whole: as many colours as points, finite coordinates within range, a size
    /// on screen in range and a rigid placement.
    pub fn check(&self) -> Result<()> {
        let bad = |what: &str| Err(DocError::Invalid(format!("point cloud `{}`: {what}", self.name)));
        if self.points.len() > MAX_POINTS {
            return bad("too many points");
        }
        if !self.colors.is_empty() && self.colors.len() != self.points.len() {
            return bad("its colours do not match its points");
        }
        if ![self.origin.x, self.origin.y, self.origin.z].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) {
            return bad("its origin is out of range");
        }
        if self.points.values().iter().any(|v| !v.is_finite() || f64::from(v.abs()) > MAX_COORD) {
            return bad("a point is out of range");
        }
        if unit_scale(&self.units).is_none() {
            return bad("its units are not mm, cm, m, in or ft");
        }
        if !(self.point_size.is_finite() && POINT_SIZE.contains(&self.point_size)) {
            return bad("its point size is out of range");
        }
        if self.color.is_some_and(|c| c.iter().any(|v| !(0.0..=1.0).contains(v))) {
            return bad("its colour is out of range");
        }
        if !rigid(&self.transform) {
            return bad("its placement is not a rigid move");
        }
        Ok(())
    }
}

/// A rotation and a translation within range (no scale, shear or mirror).
pub fn rigid(m: &Mat) -> bool {
    let column = |i: usize| Vec3::new(m[i][0], m[i][1], m[i][2]);
    let (x, y, z) = (column(0), column(1), column(2));
    m.iter().flatten().all(|v| v.is_finite())
        && m[3][..3].iter().all(|v| v.abs() <= MAX_COORD)
        && [x, y, z].iter().all(|axis| (axis.dot(*axis) - 1.0).abs() < 1e-6)
        && x.dot(y).abs() < 1e-6
        && x.dot(z).abs() < 1e-6
        && y.dot(z).abs() < 1e-6
        && x.cross(y).dot(z) > 0.0
        && [m[0][3], m[1][3], m[2][3]].iter().all(|v| v.abs() < 1e-9)
        && (m[3][3] - 1.0).abs() < 1e-9
}

/// Corners of the box around some points.
type Corners = Option<([f32; 3], [f32; 3])>;

/// x, y, z per point (single precision, little-endian when stored). Shared, so undo states and
/// copies of the design don't copy the points; the box around them is worked out once.
#[derive(Clone, Debug, Default)]
pub struct Points {
    values: Arc<[f32]>,
    bounds: Arc<std::sync::OnceLock<Corners>>,
}

impl Points {
    pub fn new(values: Arc<[f32]>) -> Points {
        Points { values, bounds: Default::default() }
    }
    /// x, y, z per point.
    pub fn values(&self) -> &Arc<[f32]> {
        &self.values
    }
    pub fn len(&self) -> usize {
        self.values.len() / 3
    }
    pub fn is_empty(&self) -> bool {
        self.values.len() < 3
    }
    pub fn get(&self, i: usize) -> Option<[f32; 3]> {
        let s = self.values.get(i.checked_mul(3)?..i.checked_mul(3)?.checked_add(3)?)?;
        Some([*s.first()?, *s.get(1)?, *s.get(2)?])
    }
    pub fn iter(&self) -> impl Iterator<Item = [f32; 3]> + '_ {
        self.values.as_chunks::<3>().0.iter().copied()
    }
    /// The lowest and highest x, y and z, or `None` without points.
    pub fn bounds(&self) -> Corners {
        *self.bounds.get_or_init(|| {
            let mut it = self.iter();
            let first = it.next()?;
            Some(it.fold((first, first), |(mut lo, mut hi), p| {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
                (lo, hi)
            }))
        })
    }
    /// The points as stored: little-endian f32, x, y, z per point.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.values.len() * 4);
        for v in self.values.iter() {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
    /// The points from stored bytes; `None` unless whole points of finite coordinates.
    pub fn from_bytes(b: &[u8]) -> Option<Points> {
        if !b.len().is_multiple_of(12) || b.len() / 12 > MAX_POINTS {
            return None;
        }
        let v: Vec<f32> = b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        v.iter().all(|x| x.is_finite()).then(|| Points::new(v.into()))
    }
}

/// The same points: the same shared storage, or equal values.
impl PartialEq for Points {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.values, &other.values) || self.values == other.values
    }
}

impl Serialize for Points {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&base64_encode(&self.to_bytes()))
    }
}

impl<'de> Deserialize<'de> for Points {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.len() > (MAX_POINTS * 12).div_ceil(3) * 4 + 16 {
            return Err(serde::de::Error::custom("too many points"));
        }
        let b = base64_decode(&s).ok_or_else(|| serde::de::Error::custom("points are not base64"))?;
        Points::from_bytes(&b).ok_or_else(|| serde::de::Error::custom("points are not whole, finite x, y, z"))
    }
}

/// r, g, b per point (bytes).
#[derive(Clone, Debug, Default)]
pub struct Colors(pub Arc<[u8]>);

impl Colors {
    pub fn len(&self) -> usize {
        self.0.len() / 3
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn get(&self, i: usize) -> Option<[u8; 3]> {
        let s = self.0.get(i.checked_mul(3)?..i.checked_mul(3)?.checked_add(3)?)?;
        Some([*s.first()?, *s.get(1)?, *s.get(2)?])
    }
    pub fn from_bytes(b: &[u8]) -> Option<Colors> {
        (b.len().is_multiple_of(3) && b.len() / 3 <= MAX_POINTS).then(|| Colors(b.into()))
    }
}

impl PartialEq for Colors {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl Serialize for Colors {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&base64_encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Colors {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.len() > (MAX_POINTS * 3).div_ceil(3) * 4 + 16 {
            return Err(serde::de::Error::custom("too many colours"));
        }
        let b = base64_decode(&s).ok_or_else(|| serde::de::Error::custom("colours are not base64"))?;
        Colors::from_bytes(&b).ok_or_else(|| serde::de::Error::custom("colours are not whole r, g, b"))
    }
}

/// Every cloud of a design is whole ([`PointCloud::check`]) and has its own id.
pub fn check_all(clouds: &[PointCloud]) -> Result<()> {
    let mut ids = std::collections::BTreeSet::new();
    for c in clouds {
        c.check()?;
        if !ids.insert(c.id) {
            return Err(DocError::Invalid(format!("two point clouds have the id {}", c.id)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud(points: Vec<f32>, colors: Vec<u8>) -> PointCloud {
        PointCloud {
            id: 7,
            name: "Scan".into(),
            file: "scan.ply".into(),
            origin: Vec3::new(1000.0, 0.0, 0.0),
            points: Points::new(points.into()),
            colors: Colors(colors.into()),
            source_points: 2,
            units: "m".into(),
            transform: IDENTITY,
            point_size: 2.0,
            color: None,
            visible: true,
        }
    }

    #[test]
    fn points_round_trip_through_json_and_stay_shared() {
        let c = cloud(vec![0.0, 1.0, 2.0, -3.5, 4.25, 1e6], vec![255, 0, 10, 1, 2, 3]);
        let json = serde_json::to_string(&c).unwrap();
        let back: PointCloud = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.points.get(1), Some([-3.5, 4.25, 1e6]));
        assert_eq!(back.colors.get(0), Some([255, 0, 10]));
        assert_eq!(back.world_point(0), Some(Vec3::new(1000.0, 1.0, 2.0)));
        assert_eq!(back.points.get(2), None);
        let copy = c.clone();
        assert!(Arc::ptr_eq(copy.points.values(), c.points.values()), "a copy shares the points");
        let (lo, hi) = c.local_bounds().unwrap();
        assert_eq!((lo, hi), (Vec3::new(996.5, 1.0, 2.0), Vec3::new(1000.0, 4.25, 1e6)));
        check_all(&[c]).unwrap();
        assert_eq!((unit_scale("M"), unit_scale("yd")), (Some(1000.0), None));
    }

    #[test]
    fn damaged_clouds_are_refused() {
        // Not whole points, not finite, not base64.
        for points in ["AAAA", "AADAfwAAwH8AAMB/", "!!!!"] {
            let json = serde_json::to_string(&cloud(vec![0.0; 3], Vec::new())).unwrap().replace("\"AAAAAAAAAAAAAAAA\"", &format!("\"{points}\""));
            assert!(serde_json::from_str::<PointCloud>(&json).is_err(), "{points}");
        }
        assert!(cloud(vec![0.0; 6], vec![1, 2, 3]).check().is_err(), "one colour for two points");
        let mut c = cloud(vec![0.0; 3], Vec::new());
        c.point_size = 100.0;
        assert!(c.check().is_err());
        let mut c = cloud(vec![0.0; 3], Vec::new());
        c.transform[0][0] = 2.0;
        assert!(c.check().is_err(), "a scale is not a placement");
        let c = cloud(vec![0.0; 3], Vec::new());
        assert!(check_all(&[c.clone(), c]).is_err(), "two clouds with one id");
    }
}
