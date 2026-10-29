//! Animation stack, layer, curve nodes and curves.

use serde_spline::hkx::Animation;

use super::node::{Node, Properties70, object_name};
use super::scene::{SceneBuilder, transform_to_trs};

/// FBX time units ("KTime") per second.
///
/// Every time value in FBX (`KeyTime`, `LocalStart`, `TimeSpanStop`, ...) is an
/// `i64` in these units: 1 frame at 30 fps = `46_186_158_000 / 30` = `1_539_538_600`.
pub const KTIME_PER_SECOND: f64 = 46_186_158_000.0;

impl<'a> SceneBuilder<'a> {
    /// Creates one animation stack keyed at every Havok frame.
    ///
    /// Animation objects and how they connect:
    ///
    /// ```text
    /// AnimationStack "AnimStack::<take>"      one clip (Blender: one action)
    ///   LocalStart / LocalStop / ReferenceStart / ReferenceStop: KTime range
    /// └─ AnimationLayer "AnimLayer::BaseLayer"  (C: "OO", layer, stack)
    ///    └─ AnimationCurveNode "T" / "R" / "S"   (C: "OO", node, layer)
    ///         animates a bone property         (C: "OP", node, bone, "Lcl Translation" | "Lcl Rotation" | "Lcl Scaling")
    ///         P: "d|X" / "d|Y" / "d|Z" = value used when a channel has no curve
    ///       └─ AnimationCurve per channel        (C: "OP", curve, node, "d|X" | "d|Y" | "d|Z")
    /// ```
    ///
    /// Units of the curve values: translation in file units (Havok units
    /// here), rotation in XYZ Euler degrees, scale as a factor.
    ///
    /// `times`: KTime of every frame, see [`key_times`].
    pub(super) fn add_animation(&mut self, models: &[i64], animation: &Animation, times: &'a [i64], take_name: &str) {
        let stop = times.last().copied().unwrap_or(0);

        let stack = self.new_id();
        let layer = self.new_id();
        self.objects.push(
            Node::new("AnimationStack")
                .prop(stack)
                .prop(object_name(take_name, "AnimStack"))
                .prop("")
                .child(
                    Properties70::new()
                        .ktime("LocalStart", 0)
                        .ktime("LocalStop", stop)
                        .ktime("ReferenceStart", 0)
                        .ktime("ReferenceStop", stop)
                        .build(),
                ),
        );
        self.objects.push(
            Node::new("AnimationLayer")
                .prop(layer)
                .prop(object_name("BaseLayer", "AnimLayer"))
                .prop(""),
        );
        self.connect(layer, stack);

        for (bone_index, &model) in models.iter().enumerate() {
            // [frame][t/r/s][x/y/z]
            let mut prev_euler = None;
            let samples: Vec<[[f64; 3]; 3]> = animation
                .frames
                .iter()
                .map(|frame| {
                    let trs = transform_to_trs(&frame.transforms[bone_index], prev_euler);
                    prev_euler = Some(trs[1]);
                    trs
                })
                .collect();

            for (channel, (name, property)) in [
                ("T", "Lcl Translation"),
                ("R", "Lcl Rotation"),
                ("S", "Lcl Scaling"),
            ]
            .into_iter()
            .enumerate()
            {
                let curve_node = self.new_id();
                let default = samples.first().map_or([0.0; 3], |first| first[channel]);

                self.objects.push(
                    Node::new("AnimationCurveNode")
                        .prop(curve_node)
                        .prop(object_name(name, "AnimCurveNode"))
                        .prop("")
                        .child(
                            Properties70::new()
                                .number("d|X", default[0])
                                .number("d|Y", default[1])
                                .number("d|Z", default[2])
                                .build(),
                        ),
                );
                self.curve_nodes += 1;
                self.connect(curve_node, layer);
                self.connect_prop(curve_node, model, property);

                for (axis, axis_name) in ["d|X", "d|Y", "d|Z"].into_iter().enumerate() {
                    let curve = self.new_id();
                    let values = samples
                        .iter()
                        .map(|sample| sample[channel][axis] as f32)
                        .collect();
                    self.objects.push(anim_curve(curve, default[axis], times, values));
                    self.curves += 1;
                    self.connect_prop(curve, curve_node, axis_name);
                }
            }
        }
    }
}

/// Linear `AnimationCurve`.
///
/// ```text
/// AnimationCurve: <id>, "AnimCurve::", "" {
///     Default: 0                       ; value before the first key (unused when keys exist)
///     KeyVer: 4009                     ; key format version
///     KeyTime: *46 { a: 0, 1539538600, 3079077200, ... }   ; i64 KTime per key
///     KeyValueFloat: *46 { a: ... }    ; f32 value per key
///     KeyAttrFlags: *1 { a: 4 }        ; interpolation of each attribute group
///     KeyAttrDataFloat: *4 { a: 0, 0, <bits 218434821>, 0 }  ; tangent data (unused for linear)
///     KeyAttrRefCount: *1 { a: 46 }    ; number of keys using each attribute
/// }
/// ```
///
/// Keys are grouped into "attributes": `KeyAttrRefCount[i]` consecutive keys
/// use `KeyAttrFlags[i]` and `KeyAttrDataFloat[4*i..4*i+4]`. One attribute
/// covering every key is enough here.
///
/// `KeyAttrFlags` bits: `0x2` constant, `0x4` linear, `0x8` cubic; cubic keys
/// add tangent mode bits. `KeyAttrDataFloat` holds right/left slope and weights
/// for cubic keys; for linear keys the FBX SDK (and Blender) write
/// `0, 0, 218434821 (as f32 bits), 0`.
fn anim_curve<'a>(id: i64, default: f64, times: &'a [i64], values: Vec<f32>) -> Node<'a> {
    /// `eInterpolationLinear`
    const INTERPOLATION_LINEAR: i32 = 1 << 2;
    /// What the FBX SDK writes for linear keys (no tangents).
    static LINEAR_KEY_ATTR_DATA: [f32; 4] = [0.0, 0.0, f32::from_bits(218_434_821), 0.0];

    let count = times.len() as i32;
    Node::new("AnimationCurve")
        .prop(id)
        .prop(object_name("", "AnimCurve"))
        .prop("")
        .value("Default", default)
        .value("KeyVer", 4009)
        .value("KeyTime", times)
        .value("KeyValueFloat", values)
        // One attribute shared by all keys.
        .value("KeyAttrFlags", &[INTERPOLATION_LINEAR][..])
        .value("KeyAttrDataFloat", &LINEAR_KEY_ATTR_DATA[..])
        .value("KeyAttrRefCount", vec![count])
}

/// KTime of every frame at `fps`.
pub(super) fn key_times(frame_count: usize, fps: f64) -> Vec<i64> {
    (0..frame_count)
        .map(|frame| (frame as f64 * KTIME_PER_SECOND / fps).round() as i64)
        .collect()
}

/// Frame rate of the sampled Havok frames.
pub(super) fn animation_frame_rate(animation: &Animation) -> f64 {
    let intervals = animation.frames.len().saturating_sub(1);
    if intervals == 0 || !animation.duration.is_finite() || animation.duration <= 0.0 {
        return 30.0;
    }
    snap_frame_rate(intervals as f64 / f64::from(animation.duration))
}

/// Removes f32 noise such as `29.99998` so standard rates map to a `TimeMode`.
pub(super) fn snap_frame_rate(fps: f64) -> f64 {
    let rounded = fps.round();
    if (fps - rounded).abs() < 1.0e-3 {
        rounded
    } else {
        fps
    }
}

