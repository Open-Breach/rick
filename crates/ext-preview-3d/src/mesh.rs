//! The mesh as the renderer takes it: three vertices per triangle, each
//! with a packed colour, its bounding box in millimetres, and the
//! decimation that brings a model under the triangle budget.
//!
//! Decimation is vertex clustering on a uniform grid over the bounding
//! box: every corner snaps to the centre of its cell and a triangle whose
//! corners fall into fewer than three cells goes. The finest grid whose
//! result fits the budget is found by bisection on the grid resolution,
//! each step one pass over the triangles and no allocation.

use crate::math::Vec3;

/// A vertex as the GPU sees it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// The position in millimetres.
    pub position: Vec3,
    /// The colour, RGBA bytes packed little-endian (see [`pack`]).
    pub color: u32,
}

/// The neutral material: a light grey, as sRGB bytes.
pub const NEUTRAL: [u8; 4] = [0xB8, 0xB8, 0xBC, 0xFF];

/// The finest clustering grid, cells per longest side.
pub const MAX_GRID: u32 = 4096;

/// Packs RGBA bytes the way the shader unpacks them.
#[must_use]
pub fn pack(rgba: [u8; 4]) -> u32 {
    u32::from_le_bytes(rgba)
}

/// An axis-aligned bounding box; empty until a point is included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// The smallest coordinates.
    pub min: Vec3,
    /// The largest coordinates.
    pub max: Vec3,
}

impl Default for Aabb {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Aabb {
    /// The box of nothing.
    pub const EMPTY: Self = Self {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };

    /// Grows the box to hold `p`.
    pub fn include(&mut self, p: Vec3) {
        for (axis, value) in p.iter().enumerate() {
            self.min[axis] = self.min[axis].min(*value);
            self.max[axis] = self.max[axis].max(*value);
        }
    }

    /// Whether no point was included.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min[0] > self.max[0]
    }

    /// Width, depth and height; zero when empty.
    #[must_use]
    pub fn size(&self) -> Vec3 {
        if self.is_empty() {
            [0.0; 3]
        } else {
            crate::math::sub(self.max, self.min)
        }
    }

    /// The middle; the origin when empty.
    #[must_use]
    pub fn centre(&self) -> Vec3 {
        if self.is_empty() {
            [0.0; 3]
        } else {
            crate::math::scale(crate::math::add(self.min, self.max), 0.5)
        }
    }

    /// Half the diagonal: the radius of the bounding sphere.
    #[must_use]
    pub fn radius(&self) -> f32 {
        crate::math::length(self.size()) / 2.0
    }
}

/// Triangles with a colour each, and their bounds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    /// Three per triangle, in order.
    pub vertices: Vec<Vertex>,
    /// The box around every vertex.
    pub bounds: Aabb,
}

impl Mesh {
    /// An empty mesh with room for `triangles`.
    #[must_use]
    pub fn with_capacity(triangles: usize) -> Self {
        Self {
            vertices: Vec::with_capacity(triangles.saturating_mul(3)),
            bounds: Aabb::EMPTY,
        }
    }

    /// Adds one triangle.
    pub fn push(&mut self, corners: [Vec3; 3], color: u32) {
        for position in corners {
            self.bounds.include(position);
            self.vertices.push(Vertex { position, color });
        }
    }

    /// The number of triangles.
    #[must_use]
    pub fn triangles(&self) -> usize {
        self.vertices.len() / 3
    }

    /// Multiplies every coordinate by `factor`, for a unit conversion.
    pub fn scale(&mut self, factor: f32) {
        if (factor - 1.0).abs() < f32::EPSILON {
            return;
        }
        let mut bounds = Aabb::EMPTY;
        for vertex in &mut self.vertices {
            vertex.position = crate::math::scale(vertex.position, factor);
            bounds.include(vertex.position);
        }
        self.bounds = bounds;
    }

    /// A mesh of at most `budget` triangles standing in for this one, or
    /// `None` when this one already fits.
    #[must_use]
    pub fn decimate(&self, budget: usize) -> Option<Self> {
        if self.triangles() <= budget {
            return None;
        }
        // The finest grid whose clustering fits; one cell keeps nothing.
        let (mut lo, mut hi) = (1u32, MAX_GRID);
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            if self.kept(mid) <= budget {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let mut clustered = self.cluster(lo);
        // Clusterings of different grids do not nest, so a coarser grid
        // can in rare cases keep more; step down until it fits.
        while clustered.triangles() > budget && lo > 1 {
            lo /= 2;
            clustered = self.cluster(lo);
        }
        Some(clustered)
    }

    /// The grid's cell size and origin: `grid` cells along the longest
    /// side, square cells.
    fn cell_size(&self, grid: u32) -> f32 {
        let size = self.bounds.size();
        let longest = size[0].max(size[1]).max(size[2]);
        // The grid resolution is at most 4096, exactly representable.
        #[allow(clippy::cast_precision_loss)]
        let cells = grid.max(1) as f32;
        if longest > 0.0 { longest / cells } else { 1.0 }
    }

    /// The cell of `p`, as one index.
    fn cell(&self, p: Vec3, size: f32, grid: u32) -> u64 {
        let mut key = 0u64;
        for (axis, value) in p.iter().enumerate() {
            let offset = (value - self.bounds.min[axis]) / size;
            // Clamped into the grid before the cast, so it is exact.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let index = offset.floor().clamp(0.0, f32::from(u16::MAX)) as u64;
            key = key * u64::from(MAX_GRID + 1) + index.min(u64::from(grid.max(1) - 1));
        }
        key
    }

    /// How many triangles survive clustering on `grid`.
    fn kept(&self, grid: u32) -> usize {
        let size = self.cell_size(grid);
        self.vertices
            .chunks_exact(3)
            .filter(|corners| {
                let a = self.cell(corners[0].position, size, grid);
                let b = self.cell(corners[1].position, size, grid);
                let c = self.cell(corners[2].position, size, grid);
                a != b && b != c && a != c
            })
            .count()
    }

    /// The mesh clustered on `grid`: corners at their cells' centres,
    /// collapsed triangles dropped.
    fn cluster(&self, grid: u32) -> Self {
        let size = self.cell_size(grid);
        let snap = |p: Vec3| -> Vec3 {
            let mut out = p;
            for (axis, value) in out.iter_mut().enumerate() {
                let offset = (p[axis] - self.bounds.min[axis]) / size;
                // Clamped into the grid before the cast, so it is exact.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let index = offset.floor().clamp(0.0, f32::from(u16::MAX)) as u32;
                // At most 4096, exactly representable.
                #[allow(clippy::cast_precision_loss)]
                let index = index.min(grid.max(1) - 1) as f32;
                *value = self.bounds.min[axis] + (index + 0.5) * size;
            }
            out
        };
        let mut out = Self::with_capacity(self.kept(grid));
        for corners in self.vertices.chunks_exact(3) {
            let a = self.cell(corners[0].position, size, grid);
            let b = self.cell(corners[1].position, size, grid);
            let c = self.cell(corners[2].position, size, grid);
            if a != b && b != c && a != c {
                out.push(
                    [
                        snap(corners[0].position),
                        snap(corners[1].position),
                        snap(corners[2].position),
                    ],
                    corners[0].color,
                );
            }
        }
        out
    }
}
