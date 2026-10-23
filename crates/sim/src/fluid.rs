//! The fluid region of a study: the fluid cells reachable from a seed (an inlet's cells) through
//! face-neighbours. For an internal flow (a duct) the openings are closed off by `blocked`
//! cells; a region that still reaches the grid's edge leaks, so the walls have a gap or are too
//! thin for the cell size.

use crate::{Cell, Grid, Result, SimError};

#[derive(Clone, Debug)]
pub struct Region {
    /// Per grid cell: in the region.
    pub cells: Vec<bool>,
    /// How many cells are in it.
    pub count: usize,
    /// It reaches a cell on the grid's outer layer.
    pub touches_edge: bool,
}

/// Flood the fluid cells from `seed`, not entering cells for which `blocked(i, j, k)` is true.
pub fn fluid_region(grid: &Grid, seed: [usize; 3], blocked: impl Fn(usize, usize, usize) -> bool) -> Result<Region> {
    let [nx, ny, nz] = grid.dims;
    let start = grid.index(seed[0], seed[1], seed[2]).ok_or(SimError::BadSeed)?;
    if grid.cells.get(start) != Some(&Cell::Fluid) || blocked(seed[0], seed[1], seed[2]) {
        return Err(SimError::BadSeed);
    }
    let mut cells = vec![false; grid.len()];
    let mut stack = vec![start];
    let (mut count, mut touches_edge) = (0, false);
    if let Some(c) = cells.get_mut(start) {
        *c = true;
    }
    while let Some(at) = stack.pop() {
        count += 1;
        let [i, j, k] = grid.coords(at);
        touches_edge |= i == 0 || j == 0 || k == 0 || i + 1 == nx || j + 1 == ny || k + 1 == nz;
        let near = [
            (i.checked_sub(1), Some(j), Some(k)),
            (Some(i + 1), Some(j), Some(k)),
            (Some(i), j.checked_sub(1), Some(k)),
            (Some(i), Some(j + 1), Some(k)),
            (Some(i), Some(j), k.checked_sub(1)),
            (Some(i), Some(j), Some(k + 1)),
        ];
        for (a, b, c) in near {
            let (Some(a), Some(b), Some(c)) = (a, b, c) else { continue };
            let Some(n) = grid.index(a, b, c) else { continue };
            let fluid = grid.cells.get(n) == Some(&Cell::Fluid);
            if let Some(seen) = cells.get_mut(n)
                && !*seen
                && fluid
                && !blocked(a, b, c)
            {
                *seen = true;
                stack.push(n);
            }
        }
    }
    Ok(Region { cells, count, touches_edge })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxelize;
    use crate::voxelize::tests::square_duct;

    #[test]
    fn a_duct_bore_is_found_between_its_openings_and_leaks_without_them() {
        let m = square_duct();
        let mut g = Grid::around(m.bounds(), 2.0, 2, 1_000_000).unwrap();
        voxelize(&m, &mut g).unwrap();
        let cell = |v: f64, o: f64| ((v - o) / 2.0).floor() as usize;
        let seed = [cell(20.0, g.origin.x), cell(10.0, g.origin.y), cell(10.0, g.origin.z)];
        // Open ends: the bore joins the air around the duct.
        let open = fluid_region(&g, seed, |_, _, _| false).unwrap();
        assert!(open.touches_edge);
        // The two openings closed off at the duct's end planes: just the bore, 6 × 6 × 20 cells.
        let (x0, x1) = (cell(0.0, g.origin.x), cell(40.0, g.origin.x));
        let bore = fluid_region(&g, seed, |i, _, _| i < x0 || i >= x1).unwrap();
        assert!(!bore.touches_edge);
        assert_eq!(bore.count, 720);
    }

    #[test]
    fn a_seed_in_the_wall_or_off_the_grid_is_refused() {
        let m = square_duct();
        let mut g = Grid::around(m.bounds(), 2.0, 2, 1_000_000).unwrap();
        voxelize(&m, &mut g).unwrap();
        assert_eq!(fluid_region(&g, [999, 0, 0], |_, _, _| false).err(), Some(SimError::BadSeed));
        let wall = [12, 3, 8];
        assert_eq!(g.get(wall[0], wall[1], wall[2]), Some(Cell::Solid));
        assert_eq!(fluid_region(&g, wall, |_, _, _| false).err(), Some(SimError::BadSeed));
    }
}
