//! The floor grid: a 4-connected lattice of 1 m cells (ARCHITECTURE §3).
//!
//! Cells are stored row-major and addressed by a dense [`CellIdx`], so every
//! per-cell table is a plain `Vec` (ADR 0002: no hashed containers). The
//! origin is the north-west corner; `y` grows southwards, matching screen
//! coordinates in the viewer.

use crate::ids::{ChuteId, StationId};

/// Dense row-major index of a cell: `y * width + x`.
pub type CellIdx = u32;

/// A cell coordinate. `x` grows east, `y` grows south.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    /// Column, 0 at the west wall.
    pub x: u16,
    /// Row, 0 at the north wall.
    pub y: u16,
}

impl Pos {
    /// A position from coordinates.
    #[must_use]
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }

    /// Manhattan distance to `other`.
    #[must_use]
    pub const fn manhattan(self, other: Self) -> u32 {
        self.x.abs_diff(other.x) as u32 + self.y.abs_diff(other.y) as u32
    }
}

/// What occupies a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// Open floor; robots drive here.
    Free,
    /// Impassable boundary.
    Wall,
    /// A hole a robot drops parcels into from an adjacent free cell. Not
    /// traversable.
    Chute(ChuteId),
    /// An induction (loading) cell. Only the robot loading there may enter it,
    /// so stations never become through-traffic choke points.
    Induction(StationId),
}

impl Cell {
    /// Stable one-byte code for hashing and for the viewer's layout buffer.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Free => 0,
            Self::Wall => 1,
            Self::Chute(_) => 2,
            Self::Induction(_) => 3,
        }
    }
}

/// The four move directions in the fixed order used for every tie-break:
/// north, east, south, west (ADR 0002 §4).
pub const DIRS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// A rectangular grid of [`Cell`]s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl Grid {
    /// A grid of the given size filled with `fill`.
    #[must_use]
    pub fn new(width: u16, height: u16, fill: Cell) -> Self {
        Self {
            width,
            height,
            cells: vec![fill; usize::from(width) * usize::from(height)],
        }
    }

    /// Width in cells.
    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Height in cells.
    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }

    /// Number of cells.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// True for a zero-sized grid.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// All cells, row-major.
    #[must_use]
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The index of `pos`.
    #[must_use]
    pub fn idx(&self, pos: Pos) -> CellIdx {
        u32::from(pos.y) * u32::from(self.width) + u32::from(pos.x)
    }

    /// The position of `idx`.
    #[must_use]
    pub fn pos(&self, idx: CellIdx) -> Pos {
        let w = u32::from(self.width);
        // Both quotients fit in u16 because idx < width * height.
        #[allow(clippy::cast_possible_truncation)]
        Pos::new((idx % w) as u16, (idx / w) as u16)
    }

    /// The cell at `idx`.
    #[must_use]
    pub fn cell(&self, idx: CellIdx) -> Cell {
        self.cells[idx as usize]
    }

    /// The cell at `pos`.
    #[must_use]
    pub fn at(&self, pos: Pos) -> Cell {
        self.cell(self.idx(pos))
    }

    /// Overwrite the cell at `pos`.
    pub fn set(&mut self, pos: Pos, cell: Cell) {
        let i = self.idx(pos) as usize;
        self.cells[i] = cell;
    }

    /// True if robots may drive through `idx` (open floor only; induction
    /// cells are entered only as a destination, see [`Cell::Induction`]).
    #[must_use]
    pub fn is_free(&self, idx: CellIdx) -> bool {
        matches!(self.cell(idx), Cell::Free)
    }

    /// In-bounds neighbours of `idx`, in [`DIRS`] order.
    pub fn neighbours(&self, idx: CellIdx) -> impl Iterator<Item = CellIdx> + '_ {
        let p = self.pos(idx);
        DIRS.iter().filter_map(move |&(dx, dy)| {
            let x = i32::from(p.x) + dx;
            let y = i32::from(p.y) + dy;
            if x < 0 || y < 0 || x >= i32::from(self.width) || y >= i32::from(self.height) {
                return None;
            }
            // x and y are in 0..u16::MAX here.
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Some(self.idx(Pos::new(x as u16, y as u16)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idx_and_pos_round_trip() {
        let g = Grid::new(7, 5, Cell::Free);
        for i in 0..35 {
            assert_eq!(g.idx(g.pos(i)), i);
        }
    }

    #[test]
    fn corner_has_two_neighbours_in_fixed_order() {
        let g = Grid::new(4, 4, Cell::Free);
        let n: Vec<_> = g.neighbours(0).collect();
        // East then south.
        assert_eq!(n, vec![1, 4]);
    }
}
