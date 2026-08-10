use eframe::egui;

use havok_types::{QsTransform, Quaternion, Vector4};
use serde_spline::spline::alt::{
    de::de_spline_from_hkx_or_xml,
    skeleton::{AnimationClip, Skeleton},
};

/// A simple orbit ("arcball-style") camera: the eye sits on a sphere of
/// `distance` radius around `target`, driven by `yaw`/`pitch`. Good enough
/// for previewing a skeleton without pulling in a full math crate.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Camera {
    target: (f32, f32, f32),
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

const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

impl Camera {
    fn frame_bounds(&mut self, center: (f32, f32, f32), radius: f32) {
        self.target = center;
        self.distance = radius.max(1.0) / (self.fov_y * 0.5).sin() * 1.2;
    }

    /// Single source of truth for the camera basis. `right` is derived
    /// from `yaw` alone — never from `cross(forward, world_up)` — so it
    /// stays well-defined even when looking straight down/up, where
    /// `forward` becomes parallel to world-up and that cross product
    /// would collapse to (near) zero.
    #[expect(clippy::type_complexity)]
    fn axes(&self) -> ((f32, f32, f32), (f32, f32, f32), (f32, f32, f32)) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();

        let right = (cy, 0.0, -sy); // always horizontal
        let forward = (-cp * sy, -sp, -cp * cy); // derived from yaw & pitch
        let up = cross(right, forward); // never used to derive `right`

        (forward, right, up)
    }

    fn eye(&self) -> (f32, f32, f32) {
        let (forward, _, _) = self.axes();
        (
            forward.0.mul_add(-self.distance, self.target.0),
            forward.1.mul_add(-self.distance, self.target.1),
            forward.2.mul_add(-self.distance, self.target.2),
        )
    }

    fn orbit(&mut self, delta: egui::Vec2) {
        const SENSITIVITY: f32 = 0.005;
        self.yaw = delta.x.mul_add(-SENSITIVITY, self.yaw);
        self.pitch = delta
            .y
            .mul_add(SENSITIVITY, self.pitch)
            .clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    fn pan(&mut self, delta: egui::Vec2) {
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

    fn zoom(&mut self, delta: f32) {
        self.distance = (self.distance * delta.mul_add(-0.001, 1.0)).clamp(1.0, 100_000.0);
    }

    fn apply_preset(&mut self, preset: CameraPreset) {
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

    fn project(&self, world: (f32, f32, f32), rect: egui::Rect) -> Option<egui::Pos2> {
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
    fn turn(&mut self, direction: TurnDirection) {
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
enum TurnDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
enum CameraPreset {
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}

fn sub(a: (f32, f32, f32), b: (f32, f32, f32)) -> (f32, f32, f32) {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}

fn dot(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    a.2.mul_add(b.2, a.1.mul_add(b.1, a.0 * b.0))
}

fn cross(a: (f32, f32, f32), b: (f32, f32, f32)) -> (f32, f32, f32) {
    (
        a.2.mul_add(-b.1, a.1 * b.2),
        a.0.mul_add(-b.2, a.2 * b.0),
        a.1.mul_add(-b.0, a.0 * b.1),
    )
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct App {
    anim_path: String,
    skeleton_path: String,

    #[serde(skip, default)]
    clip: Option<AnimationClip>,
    #[serde(skip, default)]
    skeleton: Option<Skeleton>,

    camera: Camera,

    time: f64,
    playing: bool,

    #[serde(skip, default)]
    error: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            anim_path: String::new(),
            skeleton_path: String::new(),

            clip: None,
            skeleton: None,

            camera: Camera::default(),

            time: 0.0,
            playing: false,

            error: None,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("loader").show(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Animation:");

                    ui.add(
                        egui::TextEdit::singleline(&mut self.anim_path)
                            .desired_width(ui.available_width()),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Skeleton:");

                    ui.add(
                        egui::TextEdit::singleline(&mut self.skeleton_path)
                            .desired_width(ui.available_width()),
                    );
                });

                if ui.button("Load").clicked() {
                    self.error = None;

                    match load_animation(&self.anim_path, &self.skeleton_path) {
                        Ok((clip, skeleton)) => {
                            let (center, radius) = bounding_sphere(
                                skeleton
                                    .reference_pose
                                    .iter()
                                    .map(|t| (t.transition.x, t.transition.y, t.transition.z)),
                            );

                            self.camera.frame_bounds(center, radius);
                            self.clip = Some(clip);
                            self.skeleton = Some(skeleton);
                            self.time = 0.0;
                            self.playing = true;
                        }

                        Err(err) => {
                            self.error = Some(err.to_string());
                        }
                    }
                }

                ui.horizontal(|ui| {
                    ui.label("View:");

                    if ui.button("Front").clicked() {
                        self.camera.apply_preset(CameraPreset::Front);
                    }
                    if ui.button("Back").clicked() {
                        self.camera.apply_preset(CameraPreset::Back);
                    }
                    if ui.button("Right").clicked() {
                        self.camera.apply_preset(CameraPreset::Right);
                    }
                    if ui.button("Left").clicked() {
                        self.camera.apply_preset(CameraPreset::Left);
                    }
                    if ui.button("Top").clicked() {
                        self.camera.apply_preset(CameraPreset::Top);
                    }
                    if ui.button("Bottom").clicked() {
                        self.camera.apply_preset(CameraPreset::Bottom);
                    }

                    ui.separator();

                    // Relative — rotates by a fixed step from the current view, exactly
                    // like Blender's Ctrl+Numpad orbit.
                    if ui.button("◀ Turn Left").clicked() {
                        self.camera.turn(TurnDirection::Left);
                    }
                    if ui.button("Turn Right ▶").clicked() {
                        self.camera.turn(TurnDirection::Right);
                    }
                    if ui.button("▲ Turn Up").clicked() {
                        self.camera.turn(TurnDirection::Up);
                    }
                    if ui.button("▼ Turn Down").clicked() {
                        self.camera.turn(TurnDirection::Down);
                    }
                });

                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
            });
        });

        egui::Panel::bottom("timeline").show(ui, |ui| {
            let Some(clip) = &self.clip else { return };
            if timeline_ui(ui, &mut self.time, clip.duration as f64, clip.num_frames) {
                self.playing = false;
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let (Some(clip), Some(skeleton)) = (&self.clip, &self.skeleton) else {
                return;
            };

            if self.playing {
                self.time += ui.input(|i| i.stable_dt) as f64;

                ui.request_repaint();
            }

            // camera -------------------------------------------------
            let rect = ui.available_rect_before_wrap();
            let response = ui.interact(rect, ui.id().with("camera"), egui::Sense::click_and_drag());

            // Left-drag: orbit. Right/middle-drag: pan. Scroll: zoom.
            if response.dragged_by(egui::PointerButton::Primary) {
                self.camera.orbit(response.drag_delta());
            } else if response.dragged_by(egui::PointerButton::Secondary)
                || response.dragged_by(egui::PointerButton::Middle)
            {
                self.camera.pan(response.drag_delta());
            }

            if response.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    self.camera.zoom(scroll);
                }
            }

            let gizmo_rect = egui::Rect::from_min_size(
                rect.right_top() + egui::vec2(-70.0, 10.0),
                egui::vec2(60.0, 60.0),
            );
            if let Some(preset) = axis_gizmo_ui(ui, gizmo_rect, &self.camera) {
                self.camera.apply_preset(preset);
            }
            // -------------------------------------------------

            let frame = clip.frame_at(self.time);
            self.error = Some(format!(
                "time={} frame={} pos={:?}",
                self.time, frame, clip.frames[frame][0].transition
            ));

            draw_animation(ui, clip, skeleton, self.time, &self.camera, rect);
        });
    }

    #[expect(clippy::unwrap_used)]
    fn on_exit(&mut self) {
        std::fs::create_dir_all("logs").unwrap();
        std::fs::write(
            "logs/config.json",
            sonic_rs::to_string_pretty(self).unwrap(),
        )
        .unwrap();
        std::fs::write(
            "logs/clip.json",
            sonic_rs::to_string_pretty(&self.clip).unwrap(),
        )
        .unwrap();
        std::fs::write(
            "logs/skeleton.json",
            sonic_rs::to_string_pretty(&self.skeleton).unwrap(),
        )
        .unwrap();
    }
}

/// Draws a small Blender-style axis gizmo in the given corner rect and
/// returns the preset the user clicked, if any.
fn axis_gizmo_ui(ui: &egui::Ui, rect: egui::Rect, camera: &Camera) -> Option<CameraPreset> {
    let painter = ui.painter();
    let center = rect.center();
    let radius = rect.width().min(rect.height()).mul_add(0.5, -14.0);

    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
    );

    type Axises<'a> = [((f32, f32, f32), &'a str, egui::Color32, CameraPreset); 6];
    let axises: Axises = [
        (
            (1.0, 0.0, 0.0),
            "X",
            egui::Color32::from_rgb(220, 60, 60),
            CameraPreset::Right,
        ),
        (
            (-1.0, 0.0, 0.0),
            "-X",
            egui::Color32::from_rgb(150, 40, 40),
            CameraPreset::Left,
        ),
        (
            (0.0, 1.0, 0.0),
            "Y",
            egui::Color32::from_rgb(60, 200, 60),
            CameraPreset::Top,
        ),
        (
            (0.0, -1.0, 0.0),
            "-Y",
            egui::Color32::from_rgb(40, 130, 40),
            CameraPreset::Bottom,
        ),
        (
            (0.0, 0.0, 1.0),
            "Z",
            egui::Color32::from_rgb(60, 120, 220),
            CameraPreset::Front,
        ),
        (
            (0.0, 0.0, -1.0),
            "-Z",
            egui::Color32::from_rgb(40, 80, 150),
            CameraPreset::Back,
        ),
    ];

    let (_, cam_right, cam_up) = camera.axes();
    let mut clicked = None;

    // Sort back-to-front by depth so the near knob draws on top.
    let mut entries: Vec<_> = axises
        .iter()
        .map(|(dir, label, color, preset)| {
            let depth = dot(*dir, sub(camera.target, camera.eye()));
            (
                dot(*dir, cam_right),
                dot(*dir, cam_up),
                depth,
                label,
                color,
                preset,
            )
        })
        .collect();
    #[expect(clippy::unwrap_used)]
    entries.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());

    for (sx, sy, depth, label, color, preset) in entries {
        let pos = egui::pos2(sx.mul_add(radius, center.x), sy.mul_add(-radius, center.y));
        let knob_radius = if depth > 0.0 { 9.0 } else { 6.0 };
        let fill = if depth > 0.0 {
            *color
        } else {
            egui::Color32::from_gray(50)
        };

        painter.circle_filled(pos, knob_radius, fill);
        painter.text(
            pos,
            egui::Align2::CENTER_CENTER,
            *label,
            egui::FontId::proportional(9.0),
            egui::Color32::WHITE,
        );

        if ui.rect_contains_pointer(egui::Rect::from_center_size(
            pos,
            egui::vec2(knob_radius * 2.0, knob_radius * 2.0),
        )) && ui.input(|i| i.pointer.any_click())
        {
            clicked = Some(*preset);
        }
    }

    clicked
}

/// Draws a Blender-style scrubber: a horizontal bar spanning the clip's
/// duration, with per-frame tick marks and a draggable playhead. Clicking
/// or dragging anywhere on the bar seeks `time` to that position.
/// Returns `true` while the user is actively interacting with it, so the
/// caller can pause playback for the duration of the drag.
fn timeline_ui(ui: &mut egui::Ui, time: &mut f64, duration: f64, num_frames: usize) -> bool {
    let desired_height = 32.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), desired_height),
        egui::Sense::click_and_drag(),
    );

    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30));

    if duration > 0.0 && num_frames > 1 {
        for frame in 0..num_frames {
            let t = frame as f32 / (num_frames - 1) as f32;
            let x = rect.left() + t * rect.width();

            painter.line_segment(
                [
                    egui::pos2(x, rect.bottom() - 6.0),
                    egui::pos2(x, rect.bottom()),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(90)),
            );
        }
    }

    let scrubbing = response.dragged() || response.clicked();

    if scrubbing && let Some(pos) = response.interact_pointer_pos() {
        let frac = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        *time = frac as f64 * duration;
    }

    if duration > 0.0 {
        let frac = (*time / duration).clamp(0.0, 1.0) as f32;
        let x = rect.left() + frac * rect.width();

        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(2.0, egui::Color32::LIGHT_BLUE),
        );
        painter.circle_filled(egui::pos2(x, rect.top()), 4.0, egui::Color32::LIGHT_BLUE);
    }

    scrubbing
}

fn load_animation(
    anim_path: &str,
    skeleton_path: &str,
) -> Result<(AnimationClip, Skeleton), AnyError> {
    let anim_path = std::path::Path::new(anim_path);

    let skeleton_path = std::path::Path::new(skeleton_path);

    let anim_bytes = std::fs::read(anim_path)?;

    let skeleton_bytes = std::fs::read(skeleton_path)?;

    Ok(de_spline_from_hkx_or_xml(
        &anim_bytes,
        anim_path,
        &skeleton_bytes,
        skeleton_path,
    )?)
}

/// Center + radius of the smallest axis-aligned bounding sphere around the
/// given points (rough approximation, but plenty for camera framing).
fn bounding_sphere(points: impl Iterator<Item = (f32, f32, f32)>) -> ((f32, f32, f32), f32) {
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

fn compute_world(local: &[QsTransform], parents: &[i16]) -> Vec<QsTransform> {
    let mut result = vec![QsTransform::default(); local.len()];

    for i in 0..local.len() {
        let parent = parents[i];

        result[i] = if parent < 0 {
            local[i].clone()
        } else {
            mul_qs_transform(&result[parent as usize], &local[i])
        };
    }

    result
}

fn draw_animation(
    ui: &egui::Ui,
    clip: &AnimationClip,
    skeleton: &Skeleton,
    time: f64,
    camera: &Camera,
    rect: egui::Rect,
) {
    let frame = clip.frame_at(time);
    let local = &clip.frames[frame];
    let world = compute_world(local, &skeleton.parent_indices);

    let painter = ui.painter();

    for (index, bone) in world.iter().enumerate() {
        let parent = skeleton.parent_indices[index];
        if parent < 0 {
            continue;
        }

        let parent = parent as usize;
        let a = &world[parent].transition;
        let b = &bone.transition;

        let (Some(pa), Some(pb)) = (
            camera.project((a.x, a.y, a.z), rect),
            camera.project((b.x, b.y, b.z), rect),
        ) else {
            continue; // one endpoint is behind the camera
        };

        painter.line_segment([pa, pb], egui::Stroke::new(3.0, egui::Color32::WHITE));
        painter.circle_filled(pb, 4.0, egui::Color32::LIGHT_BLUE);
    }
}

fn mul_qs_transform(parent: &QsTransform, child: &QsTransform) -> QsTransform {
    // NOTE: Havok's QsTransform carries a scale component, but Skyrim skeletal
    // clips almost never animate it, and the decoded/reference-pose scale here
    // comes through as (0, 0, 0) rather than the expected identity (1, 1, 1).
    // Folding a zero scale into the translation collapses every bone onto its
    // parent, which is exactly the "single dot in the center" symptom.
    // Bone positions therefore ignore scale entirely.
    let transition = add_vec4(
        &rotate_vector(&parent.quaternion, &child.transition),
        &parent.transition,
    );

    let quaternion = mul_quaternion(&parent.quaternion, &child.quaternion);

    // Kept for completeness only; not used when computing bone positions.
    let scale = mul_vec4(&parent.scale, &child.scale);

    QsTransform {
        transition,
        quaternion,
        scale,
    }
}

fn add_vec4(a: &Vector4, b: &Vector4) -> Vector4 {
    Vector4::new(a.x + b.x, a.y + b.y, a.z + b.z, a.w + b.w)
}

fn mul_vec4(a: &Vector4, b: &Vector4) -> Vector4 {
    Vector4::new(a.x * b.x, a.y * b.y, a.z * b.z, a.w * b.w)
}

fn mul_quaternion(a: &Quaternion, b: &Quaternion) -> Quaternion {
    Quaternion {
        x: a.z.mul_add(
            -b.y,
            a.y.mul_add(b.z, a.x.mul_add(b.scaler, a.scaler * b.x)),
        ),

        y: a.z.mul_add(
            b.x,
            a.y.mul_add(b.scaler, a.x.mul_add(-b.z, a.scaler * b.y)),
        ),

        z: a.z.mul_add(
            b.scaler,
            a.y.mul_add(-b.x, a.x.mul_add(b.y, a.scaler * b.z)),
        ),

        scaler: a.z.mul_add(
            -b.z,
            a.y.mul_add(-b.y, a.x.mul_add(-b.x, a.scaler * b.scaler)),
        ),
    }
}

fn rotate_vector(q: &Quaternion, v: &Vector4) -> Vector4 {
    let qv = Quaternion {
        x: v.x,
        y: v.y,
        z: v.z,
        scaler: 0.0,
    };

    let inv = Quaternion {
        x: -q.x,
        y: -q.y,
        z: -q.z,
        scaler: q.scaler,
    };

    let result = mul_quaternion(&mul_quaternion(q, &qv), &inv);

    Vector4 {
        x: result.x,
        y: result.y,
        z: result.z,
        w: v.w,
    }
}

type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;

fn main() -> Result<(), AnyError> {
    let options = eframe::NativeOptions::default();

    Ok(eframe::run_native(
        "Havok Animation Viewer",
        options,
        Box::new(|_| {
            let app: App = std::fs::read_to_string("logs/config.json")
                .ok()
                .and_then(|s| sonic_rs::from_str(&s).ok())
                .unwrap_or_default();
            Ok(Box::new(app))
        }),
    )?)
}
