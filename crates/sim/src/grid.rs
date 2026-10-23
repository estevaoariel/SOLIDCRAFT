//! A uniform Cartesian grid of cubic cells around a body.

use solidcraft_geom::Aabb3;
use solidcraft_geom::Vec3;

use crate::{Result, SimError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Fluid,
    Solid,
}

#[derive(Clone, Debug)]
pub struct Grid {
    /// Cells along x, y and z.
    pub dims: [usize; 3],
    /// The corner of cell (0, 0, 0), in millimetres.
    pub origin: Vec3,
    /// The cells' edge length, in millimetres.
    pub spacing: f64,
    /// x fastest, then y, then z.
    pub cells: Vec<Cell>,
}

impl Grid {
    /// All-fluid cells of size `spacing` covering `bounds` plus `pad` cells on every side, refused
    /// past `max_cells` before anything is allocated.
    pub fn around(bounds: Aabb3, spacing: f64, pad: usize, max_cells: u64) -> Result<Grid> {
        if !spacing.is_finite() || spacing <= 0.0 {
            return Err(SimError::InvalidSpacing);
        }
        let size = [bounds.max.x - bounds.min.x, bounds.max.y - bounds.min.y, bounds.max.z - bounds.min.z];
        if size.iter().any(|s| !s.is_finite() || *s < 0.0) {
            return Err(SimError::EmptyMesh);
        }
        let mut dims = [0usize; 3];
        let mut cells: u64 = 1;
        for (d, s) in dims.iter_mut().zip(size) {
            let n = (s / spacing).ceil() + 2.0 * pad as f64;
            if n > max_cells as f64 {
                return Err(SimError::TooManyCells { cells: u64::MAX, max: max_cells });
            }
            *d = (n as usize).max(1);
            cells = cells.saturating_mul(*d as u64);
        }
        if cells > max_cells {
            return Err(SimError::TooManyCells { cells, max: max_cells });
        }
        let p = pad as f64 * spacing;
        let origin = Vec3 { x: bounds.min.x - p, y: bounds.min.y - p, z: bounds.min.z - p };
        Ok(Grid { dims, origin, spacing, cells: vec![Cell::Fluid; cells as usize] })
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn index(&self, i: usize, j: usize, k: usize) -> Option<usize> {
        let [nx, ny, nz] = self.dims;
        (i < nx && j < ny && k < nz).then_some(i + nx * (j + ny * k))
    }

    /// The cell coordinates of an index.
    pub fn coords(&self, index: usize) -> [usize; 3] {
        let [nx, ny, _] = self.dims;
        [index % nx, (index / nx) % ny, index / (nx * ny)]
    }

    pub fn center(&self, i: usize, j: usize, k: usize) -> Vec3 {
        let s = self.spacing;
        Vec3 { x: self.origin.x + (i as f64 + 0.5) * s, y: self.origin.y + (j as f64 + 0.5) * s, z: self.origin.z + (k as f64 + 0.5) * s }
    }

    pub fn get(&self, i: usize, j: usize, k: usize) -> Option<Cell> {
        self.cells.get(self.index(i, j, k)?).copied()
    }

    pub fn count(&self, cell: Cell) -> usize {
        self.cells.iter().filter(|c| **c == cell).count()
    }
}
