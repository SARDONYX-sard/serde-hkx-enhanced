//! Havok animation viewer: loads a skeleton + animation (hkx/xml) and plays it.
//!
//! ```sh
//! cargo run -p serde_spline --example viewer
//! ```
mod camera;
mod draw;
mod widgets;

use std::path::Path;

use eframe::egui;
use serde_spline::hkx::{Animation, Skeleton};

use crate::camera::{Camera, CameraPreset, TurnDirection, bounding_sphere};
use crate::draw::{draw_animation, frame_at};
use crate::widgets::{axis_gizmo_ui, timeline_ui};

type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct App {
    anim_path: String,
    skeleton_path: String,

    #[serde(skip, default)]
    clip: Option<Animation>,
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
                            let (center, radius) =
                                bounding_sphere(skeleton.bones.iter().map(|bone| {
                                    let t = &bone.reference_pose.transition;
                                    (t.x, t.y, t.z)
                                }));

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
            if timeline_ui(
                ui,
                &mut self.time,
                f64::from(clip.duration),
                clip.num_frames as usize,
            ) {
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

            let frame = frame_at(clip, self.time);
            self.error = Some(format!(
                "time={} frame={} pos={:?}",
                self.time, frame, clip.frames[frame].transforms[0].transition
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

fn load_animation(anim_path: &str, skeleton_path: &str) -> Result<(Animation, Skeleton), AnyError> {
    let skeleton_path = Path::new(skeleton_path);
    let skeleton = Skeleton::from_bytes(&std::fs::read(skeleton_path)?, skeleton_path)?;

    let anim_path = Path::new(anim_path);
    let animation = Animation::from_bytes(&skeleton, &std::fs::read(anim_path)?, anim_path)?;
    Ok((animation, skeleton))
}

fn main() -> Result<(), AnyError> {
    let options = eframe::NativeOptions::default();

    Ok(eframe::run_native(
        "Havok Animation Viewer",
        options,
        Box::new(|_| {
            // Restores the paths and camera of the previous session.
            let app: App = std::fs::read_to_string("logs/config.json")
                .ok()
                .and_then(|s| sonic_rs::from_str(&s).ok())
                .unwrap_or_default();
            Ok(Box::new(app))
        }),
    )?)
}
