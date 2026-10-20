//! Layout generator: turns a [`FloorConfig`] into a concrete floor.
//!
//! The generated floor follows the shape of real robotic sort floors
//! (Libiao, Geek+): induction stations along one edge and a field of chute
//! holes in a regular lattice, with aisles between them.
//!
//! ```text
//!   x: 0 1 2 3 4 5 6 7 ...
//!   # # # # # # # # # #      y = 0       wall
//!   # . . . . . . . . #      y = 1, 2    perimeter aisle (2 cells)
//!   # . . C . . C . . #      y = 3       chute every 3rd cell in x and y,
//!   # . . . . . . . . #                  leaving 2-cell aisles between holes
//!   # . . C . . C . . #
//!   # . . . . . . . . #      south_margin free rows (staging area)
//!   # . I . . . I . . #      y = H - 2   induction stations
//!   # # # # # # # # # #      y = H - 1   wall
//! ```
//!
//! Two-cell aisles mean every chute has four free neighbours and two robots
//! can pass side by side, which CP1's one-way lane guidance (ADR 0003) relies
//! on.

use crate::config::{ConfigError, FloorConfig};
use crate::grid::{Cell, CellIdx, Grid, Pos};
use crate::ids::{ChuteId, StationId};

/// Distance value for cells that cannot reach the source set.
pub const UNREACHABLE: u16 = u16::MAX;

/// Spacing of the chute lattice: one hole, then a two-cell aisle.
pub const CHUTE_PITCH: u16 = 3;

/// Minimum distance between induction stations, so each induction cell keeps
/// free side neighbours for robots arriving and leaving.
pub const STATION_PITCH: u16 = 3;

/// A generated floor: the grid plus the cell of every station and chute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The cell grid.
    pub grid: Grid,
    /// Induction cell of each station, indexed by [`StationId`].
    pub stations: Vec<CellIdx>,
    /// Hole cell of each chute, indexed by [`ChuteId`].
    pub chutes: Vec<CellIdx>,
}

/// Generate the floor for `floor`.
///
/// # Errors
/// [`ConfigError::FloorTooSmall`], [`ConfigError::BadStationCount`],
/// [`ConfigError::NoChutes`] or [`ConfigError::Unreachable`].
pub fn generate(floor: &FloorConfig) -> Result<Layout, ConfigError> {
    let (w, h) = (floor.width, floor.height);
    if w < 10 || h < 10 {
        return Err(ConfigError::FloorTooSmall {
            width: w,
            height: h,
        });
    }
    let mut grid = Grid::new(w, h, Cell::Free);
    for x in 0..w {
        grid.set(Pos::new(x, 0), Cell::Wall);
        grid.set(Pos::new(x, h - 1), Cell::Wall);
    }
    for y in 0..h {
        grid.set(Pos::new(0, y), Cell::Wall);
        grid.set(Pos::new(w - 1, y), Cell::Wall);
    }

    // Stations: centred in equal segments of the interior south row.
    let station_row = h - 2;
    let interior = w - 2;
    let max_stations = interior / STATION_PITCH;
    if floor.stations == 0 || floor.stations > max_stations {
        return Err(ConfigError::BadStationCount {
            stations: floor.stations,
            max: max_stations,
        });
    }
    let n = u32::from(floor.stations);
    let mut stations = Vec::with_capacity(usize::from(floor.stations));
    for i in 0..floor.stations {
        // Centre of segment i of n: 1 + interior * (2i + 1) / (2n).
        let x = 1 + u32::from(interior) * (2 * u32::from(i) + 1) / (2 * n);
        // x <= interior < u16::MAX.
        #[allow(clippy::cast_possible_truncation)]
        let pos = Pos::new(x as u16, station_row);
        grid.set(pos, Cell::Induction(StationId(i)));
        stations.push(grid.idx(pos));
    }

    // Chute lattice: rows from 3 down to the staging margin, columns from 3 to
    // w - 4, so the outermost holes keep a two-cell aisle to the walls.
    let mut chutes = Vec::new();
    let last_chute_row = i32::from(station_row) - 1 - i32::from(floor.south_margin);
    let mut y = CHUTE_PITCH;
    while i32::from(y) <= last_chute_row {
        let mut x = CHUTE_PITCH;
        while x + 3 < w {
            let id = u16::try_from(chutes.len()).map_err(|_| ConfigError::NoChutes)?;
            let pos = Pos::new(x, y);
            grid.set(pos, Cell::Chute(ChuteId(id)));
            chutes.push(grid.idx(pos));
            x += CHUTE_PITCH;
        }
        y += CHUTE_PITCH;
    }
    if chutes.is_empty() {
        return Err(ConfigError::NoChutes);
    }

    let layout = Layout {
        grid,
        stations,
        chutes,
    };
    layout
        .check_invariants()
        .map_err(ConfigError::Unreachable)?;
    Ok(layout)
}

impl Layout {
    /// Free cells adjacent to a chute: where a robot stands to drop into it.
    #[must_use]
    pub fn chute_access(&self, chute: ChuteId) -> Vec<CellIdx> {
        let hole = self.chutes[chute.index()];
        self.grid
            .neighbours(hole)
            .filter(|&c| self.grid.is_free(c))
            .collect()
    }

    /// Breadth-first distances (in moves) from `sources` to every cell, moving
    /// only through free cells. Sources themselves may be any cell (an
    /// induction cell is a valid source), and induction cells get a distance
    /// but are never passed through. Cells that cannot reach a source get
    /// [`UNREACHABLE`].
    ///
    /// Integer BFS keeps the result exact and platform-independent (ADR 0002).
    /// CP1 replaces it with Dijkstra over lane-weighted edges (ADR 0003).
    #[must_use]
    pub fn distance_field(&self, sources: &[CellIdx]) -> Vec<u16> {
        let mut dist = vec![UNREACHABLE; self.grid.len()];
        let mut queue = std::collections::VecDeque::new();
        for &s in sources {
            if dist[s as usize] != 0 {
                dist[s as usize] = 0;
                queue.push_back(s);
            }
        }
        while let Some(c) = queue.pop_front() {
            let d = dist[c as usize];
            for n in self.grid.neighbours(c) {
                if dist[n as usize] != UNREACHABLE {
                    continue;
                }
                match self.grid.cell(n) {
                    Cell::Free => {
                        dist[n as usize] = d.saturating_add(1);
                        queue.push_back(n);
                    }
                    // A robot on an induction cell must know its way out, but
                    // paths never run *through* a station.
                    Cell::Induction(_) => dist[n as usize] = d.saturating_add(1),
                    Cell::Wall | Cell::Chute(_) => {}
                }
            }
        }
        dist
    }

    /// Cells where robots start, in a fixed order: the staging rows nearest
    /// the stations first, every other column first, so the fleet starts
    /// spread out and close to induction. Returns `None` if fewer than `n`
    /// cells are available.
    #[must_use]
    pub fn spawn_cells(&self, n: usize) -> Option<Vec<CellIdx>> {
        let g = &self.grid;
        let mut out = Vec::with_capacity(n);
        // Skip the station row so no robot starts beside an induction cell.
        let top_row = g.height() - 3;
        for parity in [1u16, 0] {
            for y in (1..=top_row).rev() {
                for x in 1..g.width() - 1 {
                    if out.len() == n {
                        return Some(out);
                    }
                    let i = g.idx(Pos::new(x, y));
                    if x % 2 == parity && g.is_free(i) {
                        out.push(i);
                    }
                }
            }
        }
        (out.len() == n).then_some(out)
    }

    /// The number of cells [`Layout::spawn_cells`] can provide.
    #[must_use]
    pub fn spawn_capacity(&self) -> usize {
        let g = &self.grid;
        let top_row = g.height() - 3;
        (1..=top_row)
            .flat_map(|y| (1..g.width() - 1).map(move |x| Pos::new(x, y)))
            .filter(|&p| g.is_free(g.idx(p)))
            .count()
    }

    /// Check the generator's invariants:
    /// - all free cells form one connected region;
    /// - every station can be reached from that region;
    /// - every chute has at least one adjacent free cell in that region.
    ///
    /// # Errors
    /// A description of the first violated invariant.
    pub fn check_invariants(&self) -> Result<(), String> {
        let g = &self.grid;
        let Some(first_free) = (0..g.len())
            .filter_map(|i| u32::try_from(i).ok())
            .find(|&i| g.is_free(i))
        else {
            return Err("no free cells".into());
        };
        let dist = self.distance_field(&[first_free]);
        if let Some(i) =
            (0..g.len()).find(|&i| g.cells()[i] == Cell::Free && dist[i] == UNREACHABLE)
        {
            return Err(format!("free cell {i} is disconnected"));
        }
        for (s, &cell) in self.stations.iter().enumerate() {
            let reachable = g
                .neighbours(cell)
                .any(|n| g.is_free(n) && dist[n as usize] != UNREACHABLE);
            if !reachable {
                return Err(format!("station {s} has no reachable free neighbour"));
            }
        }
        for (c, &cell) in self.chutes.iter().enumerate() {
            let reachable = g
                .neighbours(cell)
                .any(|n| g.is_free(n) && dist[n as usize] != UNREACHABLE);
            if !reachable {
                return Err(format!("chute {c} has no reachable free neighbour"));
            }
        }
        Ok(())
    }
}
