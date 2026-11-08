//! The little linear algebra the camera needs: three-vectors and
//! column-major 4 × 4 matrices laid out as WGSL reads them.

/// A point or a direction.
pub type Vec3 = [f32; 3];

/// `a + b`.
#[must_use]
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// `a - b`.
#[must_use]
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `a * s`.
#[must_use]
pub fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// The dot product.
#[must_use]
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The cross product.
#[must_use]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The length.
#[must_use]
pub fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

/// The unit vector along `a`; the zero vector stays zero.
#[must_use]
pub fn normalize(a: Vec3) -> Vec3 {
    let len = length(a);
    if len > f32::EPSILON {
        scale(a, 1.0 / len)
    } else {
        a
    }
}

/// A column-major 4 × 4 matrix: `cols[column][row]`, the order WGSL and
/// wgpu uniform buffers take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    /// The four columns.
    pub cols: [[f32; 4]; 4],
}

impl Mat4 {
    /// The identity.
    pub const IDENTITY: Self = Self {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    /// A right-handed perspective projection with the vertical field of
    /// view `fov_y` in radians, mapping depth to wgpu's `0..1`.
    #[must_use]
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        let f = 1.0 / (fov_y / 2.0).tan();
        let range = near - far;
        Self {
            cols: [
                [f / aspect, 0.0, 0.0, 0.0],
                [0.0, f, 0.0, 0.0],
                [0.0, 0.0, far / range, -1.0],
                [0.0, 0.0, near * far / range, 0.0],
            ],
        }
    }

    /// A right-handed view matrix looking from `eye` at `target` with
    /// `up` upwards.
    #[must_use]
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let f = normalize(sub(target, eye));
        let s = normalize(cross(f, up));
        let u = cross(s, f);
        Self {
            cols: [
                [s[0], u[0], -f[0], 0.0],
                [s[1], u[1], -f[1], 0.0],
                [s[2], u[2], -f[2], 0.0],
                [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
            ],
        }
    }

    /// The point `p` transformed, as homogeneous coordinates.
    #[must_use]
    pub fn transform(self, p: Vec3) -> [f32; 4] {
        let v = [p[0], p[1], p[2], 1.0];
        let mut out = [0.0; 4];
        for (r, cell) in out.iter_mut().enumerate() {
            *cell = (0..4).map(|k| self.cols[k][r] * v[k]).sum();
        }
        out
    }

    /// The sixteen numbers in column-major order.
    #[must_use]
    pub fn to_array(self) -> [f32; 16] {
        let mut out = [0.0; 16];
        for (i, cell) in out.iter_mut().enumerate() {
            *cell = self.cols[i / 4][i % 4];
        }
        out
    }
}

impl std::ops::Mul for Mat4 {
    type Output = Self;

    /// `self * rhs`: applies `rhs` first, then `self`.
    fn mul(self, rhs: Self) -> Self {
        let mut cols = [[0.0; 4]; 4];
        for (c, col) in cols.iter_mut().enumerate() {
            for (r, cell) in col.iter_mut().enumerate() {
                *cell = (0..4).map(|k| self.cols[k][r] * rhs.cols[c][k]).sum();
            }
        }
        Self { cols }
    }
}
