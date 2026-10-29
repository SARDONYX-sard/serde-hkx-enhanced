//! Small egui widgets: axis gizmo and timeline scrubber.

use eframe::egui;

use crate::camera::{Camera, CameraPreset, dot, sub};

/// Draws a small Blender-style axis gizmo in the given corner rect and
/// returns the preset the user clicked, if any.
pub fn axis_gizmo_ui(ui: &egui::Ui, rect: egui::Rect, camera: &Camera) -> Option<CameraPreset> {
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
pub fn timeline_ui(ui: &mut egui::Ui, time: &mut f64, duration: f64, num_frames: usize) -> bool {
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
            let x = t.mul_add(rect.width(), rect.left());

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
        let x = frac.mul_add(rect.width(), rect.left());

        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(2.0, egui::Color32::LIGHT_BLUE),
        );
        painter.circle_filled(egui::pos2(x, rect.top()), 4.0, egui::Color32::LIGHT_BLUE);
    }

    scrubbing
}
