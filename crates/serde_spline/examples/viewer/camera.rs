//! Orbit camera used to look at the skeleton.

use eframe::egui;


/// A simple orbit ("arcball-style") camera: the eye sits on a sphere of
/// `distance` radius around `target`, driven by `yaw`/`pitch`. Good enough
/// for previewing a skeleton without pulling in a full math crate.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    pub target: (f32, f32, f32),
    /// Rotation around world Y. 0 = looking down +Z (Front).
    yaw: f32,
    /// Rotation around the camera's local X. Clamped just short of the
    /// poles so `up` never degenerates. This is the ONLY thing controlling
    /// up/down tilt, and it is recomputed from scratch every frame —
    /// nothing is ever accumulated into a general orientation, so roll is
    /// structurally impossible.
    pitch: f32,
    distance: f32,
    fov_y: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: (0.0, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.3,
            distance: 200.0,
            fov_y: 60_f32.to_radians(),
        }
    }
}

pub const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

impl Camera {
    pub fn frame_bounds(&mut self, center: (f32, f32, f32), radius: f32) {
        self.target = center;
        self.distance = radius.max(1.0) / (self.fov_y * 0.5).sin() * 1.2;
    }

    /// Single source of truth for the camera basis. `right` is derived
    /// from `yaw` alone — never from `cross(forward, world_up)` — so it
    /// stays well-defined even when looking straight down/up, where
    /// `forward` becomes parallel to world-up and that cross product
    /// would collapse to (near) zero.
    #[expect(clippy::type_complexity)]
    pub fn axes(&self) -> ((f32, f32, f32), (f32, f32, f32), (f32, f32, f32)) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();

        let right = (cy, 0.0, -sy); // always horizontal
        let forward = (-cp * sy, -sp, -cp * cy); // derived from yaw & pitch
        let up = cross(right, forward); // never used to derive `right`

        (forward, right, up)
    }

    pub fn eye(&self) -> (f32, f32, f32) {
        let (forward, _, _) = self.axes();
        (
            forward.0.mul_add(-self.distance, self.target.0),
            forward.1.mul_add(-self.distance, self.target.1),
            forward.2.mul_add(-self.distance, self.target.2),
        )
    }

    pub fn orbit(&mut self, delta: egui::Vec2) {
        const SENSITIVITY: f32 = 0.005;
        self.yaw = delta.x.mul_add(-SENSITIVITY, self.yaw);
        self.pitch = delta
            .y
            .mul_add(SENSITIVITY, self.pitch)
            .clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    pub fn pan(&mut self, delta: egui::Vec2) {
        let (_, right, up) = self.axes();
        let scale = self.distance * 0.0015;
        self.target.0 = delta
            .y
            .mul_add(-up.0, delta.x * right.0)
            .mul_add(-scale, self.target.0);
        self.target.1 = delta
            .y
            .mul_add(-up.1, delta.x * right.1)
            .mul_add(-scale, self.target.1);
        self.target.2 = delta
            .y
            .mul_add(-up.2, delta.x * right.2)
            .mul_add(-scale, self.target.2);
    }

    pub fn zoom(&mut self, delta: f32) {
        self.distance = (self.distance * delta.mul_add(-0.001, 1.0)).clamp(1.0, 100_000.0);
    }

    pub fn apply_preset(&mut self, preset: CameraPreset) {
        use std::f32::consts::{FRAC_PI_2, PI};
        let (yaw, pitch) = match preset {
            CameraPreset::Front => (0.0, 0.0),
            CameraPreset::Back => (PI, 0.0),
            CameraPreset::Right => (FRAC_PI_2, 0.0),
            CameraPreset::Left => (-FRAC_PI_2, 0.0),
            CameraPreset::Top => (0.0, PITCH_LIMIT),
            CameraPreset::Bottom => (0.0, -PITCH_LIMIT),
        };
        self.yaw = yaw;
        self.pitch = pitch;
    }

    pub fn project(&self, world: (f32, f32, f32), rect: egui::Rect) -> Option<egui::Pos2> {
        let eye = self.eye();
        let (forward, right, up) = self.axes(); // same basis as orbit/pan — no divergence

        let rel = sub(world, eye);
        let vx = dot(rel, right);
        let vy = dot(rel, up);
        let vz = dot(rel, forward);

        if vz <= 0.01 {
            return None;
        }

        let aspect = rect.width() / rect.height().max(1.0);
        let f = 1.0 / (self.fov_y * 0.5).tan();

        Some(egui::pos2(
            ((vx * f / aspect) / vz * rect.width()).mul_add(0.5, rect.center().x),
            ((vy * f) / vz * rect.height()).mul_add(-0.5, rect.center().y),
        ))
    }
}

impl Camera {
    /// Rotate the view by a fixed step, relative to wherever it's
    /// currently pointed — the same idea as Blender's Ctrl+Numpad 4/6/8/2,
    /// as opposed to Numpad 1/3/7 which jump to an absolute world axis.
    pub fn turn(&mut self, direction: TurnDirection) {
        const STEP: f32 = std::f32::consts::FRAC_PI_2; // 90°; use FRAC_PI_4 for 45° steps

        match direction {
            TurnDirection::Left => self.yaw += STEP,
            TurnDirection::Right => self.yaw -= STEP,
            TurnDirection::Up => self.pitch = (self.pitch + STEP).clamp(-PITCH_LIMIT, PITCH_LIMIT),
            TurnDirection::Down => {
                self.pitch = (self.pitch - STEP).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TurnDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CameraPreset {
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}

pub fn sub(a: (f32, f32, f32), b: (f32, f32, f32)) -> (f32, f32, f32) {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}

pub fn dot(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    a.2.mul_add(b.2, a.1.mul_add(b.1, a.0 * b.0))
}

fn cross(a: (f32, f32, f32), b: (f32, f32, f32)) -> (f32, f32, f32) {
    (
        a.2.mul_add(-b.1, a.1 * b.2),
        a.0.mul_add(-b.2, a.2 * b.0),
        a.1.mul_add(-b.0, a.0 * b.1),
    )
}

/// Center + radius of the smallest axis-aligned bounding sphere around the
/// given points (rough approximation, but plenty for camera framing).
pub fn bounding_sphere(points: impl Iterator<Item = (f32, f32, f32)>) -> ((f32, f32, f32), f32) {
    let (mut min, mut max) = (
        (f32::MAX, f32::MAX, f32::MAX),
        (f32::MIN, f32::MIN, f32::MIN),
    );

    for p in points {
        min = (min.0.min(p.0), min.1.min(p.1), min.2.min(p.2));
        max = (max.0.max(p.0), max.1.max(p.1), max.2.max(p.2));
    }

    let center = (
        (min.0 + max.0) * 0.5,
        (min.1 + max.1) * 0.5,
        (min.2 + max.2) * 0.5,
    );
    let radius = dot(sub(max, center), sub(max, center)).sqrt();

    (center, radius)
}
