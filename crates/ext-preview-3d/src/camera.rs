//! The orbit camera: a yaw and a pitch around the model's centre and a
//! zoom factor on the distance that fits the model, driven by the pane's
//! gestures (orbit by drag, zoom by wheel, reset by double-click). The
//! model's Z axis points up, as printers and slicers have it.

use crate::math::{self, Mat4, Vec3};

/// The yaw of the default view, in radians: 35° to the right of the front.
pub const DEFAULT_YAW: f32 = 0.610_865;
/// The pitch of the default view, in radians: 25° above the ground.
pub const DEFAULT_PITCH: f32 = 0.436_332;
/// Radians of orbit per point of drag.
pub const ORBIT_RATE: f32 = 0.008;
/// Natural logarithm of the zoom factor per point of wheel.
pub const ZOOM_RATE: f32 = 0.002;
/// The smallest zoom factor: the model at a twentieth of its fitted size.
pub const MIN_ZOOM: f32 = 0.05;
/// The largest zoom factor.
pub const MAX_ZOOM: f32 = 40.0;
/// The vertical field of view, in radians: 40°.
pub const FOV_Y: f32 = 0.698_132;
/// The pitch never reaches the pole, in radians: 89°.
const MAX_PITCH: f32 = 1.553_343;
/// The radius a model is given for the camera when it has none.
const MIN_RADIUS: f32 = 0.5;
/// How much room the fitted view leaves around the bounding sphere.
const MARGIN: f32 = 1.1;

/// Where the model is looked at from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Angle around the up axis, in radians; 0 looks at the front.
    pub yaw: f32,
    /// Angle above the ground, in radians.
    pub pitch: f32,
    /// Factor on the distance that fits the model: 2 is twice as close.
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: DEFAULT_YAW,
            pitch: DEFAULT_PITCH,
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// Orbits by a drag of `dx`, `dy` points: dragging right turns the
    /// model to the right, dragging up shows more of its top.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * ORBIT_RATE;
        if self.yaw > std::f32::consts::PI {
            self.yaw -= std::f32::consts::TAU;
        } else if self.yaw < -std::f32::consts::PI {
            self.yaw += std::f32::consts::TAU;
        }
        self.pitch = (self.pitch - dy * ORBIT_RATE).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Zooms by a wheel movement of `dy` points: scrolling up comes
    /// closer.
    pub fn zoom_by(&mut self, dy: f32) {
        self.zoom = (self.zoom * (dy * ZOOM_RATE).exp()).clamp(MIN_ZOOM, MAX_ZOOM);
    }

    /// Back to the default view.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The unit vector from the model's centre to the eye.
    #[must_use]
    pub fn direction(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [sy * cp, -cy * cp, sp]
    }

    /// How far the eye is from the centre of a model of bounding radius
    /// `radius`.
    #[must_use]
    pub fn distance(&self, radius: f32) -> f32 {
        radius.max(MIN_RADIUS) * MARGIN / (FOV_Y / 2.0).sin() / self.zoom
    }

    /// Where the eye is for a model centred at `centre` with bounding
    /// radius `radius`.
    #[must_use]
    pub fn eye(&self, centre: Vec3, radius: f32) -> Vec3 {
        math::add(centre, math::scale(self.direction(), self.distance(radius)))
    }

    /// The view-projection matrix for a frame of aspect ratio `aspect`
    /// (width over height), with the far plane past `reach`, how far the
    /// scene extends from the centre (the model and its grid).
    #[must_use]
    pub fn view_projection(&self, centre: Vec3, radius: f32, aspect: f32, reach: f32) -> Mat4 {
        let distance = self.distance(radius);
        let eye = self.eye(centre, radius);
        let near = (distance - reach).max(distance * 0.01);
        let far = distance + reach.max(radius.max(MIN_RADIUS)) * 2.0;
        let view = Mat4::look_at(eye, centre, [0.0, 0.0, 1.0]);
        Mat4::perspective(FOV_Y, aspect.max(0.01), near, far) * view
    }

    /// The direction towards the key light: over the viewer's left
    /// shoulder, so the shading follows the view.
    #[must_use]
    pub fn light(&self) -> Vec3 {
        let dir = self.direction();
        let forward = math::scale(dir, -1.0);
        let right = math::normalize(math::cross(forward, [0.0, 0.0, 1.0]));
        let up = math::cross(right, forward);
        math::normalize(math::add(
            math::add(math::scale(dir, 0.6), math::scale(up, 0.7)),
            math::scale(right, -0.4),
        ))
    }
}
