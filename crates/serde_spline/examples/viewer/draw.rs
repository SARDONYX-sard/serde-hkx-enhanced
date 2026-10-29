//! Draws the posed skeleton as lines between bones.

use eframe::egui;
use rig_math::Mat4;
use serde_spline::hkx::{Animation, Skeleton};

use crate::camera::Camera;

/// Frame shown at `time` seconds; playback loops over the clip.
pub fn frame_at(animation: &Animation, time: f64) -> usize {
    let last = animation.frames.len().saturating_sub(1);
    let duration = f64::from(animation.duration);
    if last == 0 || duration <= 0.0 {
        return 0;
    }
    let t = time.rem_euclid(duration) / duration;
    ((t * last as f64).round() as usize).min(last)
}

/// World position of every bone. Parents always precede their children in a
/// Havok skeleton, so one forward pass is enough.
fn world_positions(local: &[havok_types::QsTransform], skeleton: &Skeleton) -> Vec<[f64; 3]> {
    let mut world: Vec<Mat4> = Vec::with_capacity(local.len());
    for (transform, bone) in local.iter().zip(&skeleton.bones) {
        let local = Mat4::from_qs_transform(transform);
        let parent = usize::try_from(bone.parent_index).ok().and_then(|p| world.get(p));
        world.push(parent.map_or(local, |parent| *parent * local));
    }
    world.iter().map(Mat4::get_translation).collect()
}

pub fn draw_animation(
    ui: &egui::Ui,
    animation: &Animation,
    skeleton: &Skeleton,
    time: f64,
    camera: &Camera,
    rect: egui::Rect,
) {
    let Some(frame) = animation.frames.get(frame_at(animation, time)) else {
        return;
    };
    let world = world_positions(&frame.transforms, skeleton);
    let project = |p: [f64; 3]| camera.project((p[0] as f32, p[1] as f32, p[2] as f32), rect);

    let painter = ui.painter();
    for (index, bone) in skeleton.bones.iter().enumerate() {
        let Ok(parent) = usize::try_from(bone.parent_index) else {
            continue; // root: nothing to connect to
        };
        // Skip segments with an endpoint behind the camera.
        let (Some(a), Some(b)) = (project(world[parent]), project(world[index])) else {
            continue;
        };
        painter.line_segment([a, b], egui::Stroke::new(3.0, egui::Color32::WHITE));
        painter.circle_filled(b, 4.0, egui::Color32::LIGHT_BLUE);
    }
}
