//! Export skeleton animation data as FBX.
//!
//! The scene is written by this crate without any C library:
//!
//! - every Havok bone becomes a `Model` of type `LimbNode` with a `Skeleton`
//!   node attribute, so Blender imports the hierarchy as an armature;
//! - every bone gets `Lcl Translation`/`Lcl Rotation`/`Lcl Scaling` curves
//!   keyed at each sampled Havok frame;
//! - the scene frame rate is written as `TimeMode` (falling back to
//!   `CustomFrameRate`) so that importers place each key on an integer frame.
//!
//! Importers interpolate between keys themselves, and the FBX rotation curve
//! is Euler. If keys fall between frames (e.g. 30fps keys in a 24fps scene),
//! the displayed pose is an Euler blend, not the Havok pose.
mod mesh;
mod node;

use std::collections::HashMap;

use havok_types::QsTransform;
use serde_spline::hkx::{Animation, Skeleton};

pub use self::mesh::{Cluster, Mesh, Texture, TextureSlot};
use self::node::{Node, Properties70, Property, object_name};
use crate::error::Error;
use crate::math::Mat4;

/// FBX time units per second.
const KTIME_PER_SECOND: f64 = 46_186_158_000.0;

/// Configuration for exporting a Havok animation as FBX.
#[derive(Debug)]
pub struct Config {
    /// Scene frame rate.
    ///
    /// When `None`, the frame rate of the Havok animation
    /// (`(num_frames - 1) / duration`) is used, which places every key on an
    /// integer frame.
    pub fps: Option<f32>,

    /// Target fbx format.
    pub format: Format,

    /// Name of the animation stack (Blender: action name suffix).
    pub take_name: String,

    /// Meshes skinned to the skeleton (e.g. from a NIF, see [`crate::nif`]).
    pub meshes: Vec<Mesh>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fps: None,
            format: Format::FbxBin,
            take_name: "Take 001".to_owned(),
            meshes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Format {
    FbxBin,
    FbxAscii,
}

/// Exports a skeleton and sampled animation as an FBX file.
///
/// # Errors
///
/// - If the animation track count does not match the number of skeleton bones.
/// - If the declared frame count does not match the number of sampled frames.
/// - If a sampled frame does not contain one transform for every skeleton bone.
/// - If a bone has an invalid parent index.
pub fn to_fbx(
    animation: &Animation,
    skeleton: &Skeleton,
    config: Config,
) -> Result<Vec<u8>, Error> {
    let Config {
        fps,
        format,
        take_name,
        meshes,
    } = config;

    validate_animation(skeleton, animation)?;

    let anim_fps = animation_frame_rate(animation);
    let scene_fps = fps.map_or(anim_fps, |fps| snap_frame_rate(f64::from(fps)));

    let mut scene = SceneBuilder::default();
    let models = scene.add_skeleton(skeleton)?;

    if !meshes.is_empty() {
        let by_name = skeleton
            .bones
            .iter()
            .zip(&models)
            .map(|(bone, &id)| (bone.name.to_lowercase(), id))
            .collect();
        let rest_world = rest_world_matrices(skeleton, &models);
        let _missing = scene.add_meshes(&meshes, &by_name, &rest_world);
        #[cfg(feature = "tracing")]
        if !_missing.is_empty() {
            tracing::debug!(missing = ?_missing, "mesh bones missing from the skeleton; their weights are dropped");
        }
    }

    let stop = scene.add_animation(&models, animation, anim_fps, &take_name);

    let nodes = [
        header_extension(),
        Node::new("FileId").prop(Property::Raw(FILE_ID.to_vec())),
        Node::new("CreationTime").prop("1970-01-01 10:00:00:000"),
        Node::new("Creator").prop(CREATOR),
        global_settings(scene_fps, stop),
        scene.definitions(),
        scene.objects,
        scene.connections,
        takes(&take_name, stop),
    ];

    Ok(match format {
        Format::FbxBin => node::to_binary(&nodes),
        Format::FbxAscii => node::to_ascii(&nodes).into_bytes(),
    })
}

const CREATOR: &str = concat!("serde_fbx ", env!("CARGO_PKG_VERSION"));

/// Same fixed id as Blender's exporter. Only its presence matters.
const FILE_ID: [u8; 16] = [
    0x28, 0xb3, 0x2a, 0xeb, 0xb6, 0x24, 0xcc, 0xc2, 0xbf, 0xc8, 0xb0, 0x2a, 0xa9, 0x2b, 0xfc, 0xf1,
];

/// Objects and connections collected while building the scene.
struct SceneBuilder {
    objects: Node,
    connections: Node,
    next_id: i64,
    models: usize,
    node_attributes: usize,
    geometries: usize,
    deformers: usize,
    poses: usize,
    materials: usize,
    textures: usize,
    videos: usize,
    curve_nodes: usize,
    curves: usize,
}

impl Default for SceneBuilder {
    fn default() -> Self {
        Self {
            objects: Node::new("Objects"),
            connections: Node::new("Connections"),
            next_id: 1_000_000,
            models: 0,
            node_attributes: 0,
            geometries: 0,
            deformers: 0,
            poses: 0,
            materials: 0,
            textures: 0,
            videos: 0,
            curve_nodes: 0,
            curves: 0,
        }
    }
}

impl SceneBuilder {
    const fn new_id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    /// `C: "OO", child, parent`
    fn connect(&mut self, child: i64, parent: i64) {
        self.connections
            .push(Node::new("C").prop("OO").prop(child).prop(parent));
    }

    /// `C: "OP", child, parent, property`
    fn connect_prop(&mut self, child: i64, parent: i64, property: &str) {
        self.connections.push(
            Node::new("C")
                .prop("OP")
                .prop(child)
                .prop(parent)
                .prop(property),
        );
    }

    /// Creates one `LimbNode` model per bone and returns their ids.
    fn add_skeleton(&mut self, skeleton: &Skeleton) -> Result<Vec<i64>, Error> {
        let mut models = Vec::with_capacity(skeleton.bones.len());

        for bone in &skeleton.bones {
            let model = self.new_id();
            let attribute = self.new_id();
            let [t, r, s] = transform_to_trs(&bone.reference_pose, None);

            self.objects.push(
                Node::new("NodeAttribute")
                    .prop(attribute)
                    .prop(object_name(&bone.name, "NodeAttribute"))
                    .prop("LimbNode")
                    .child(Properties70::new().double("Size", 1.0).build())
                    .value("TypeFlags", "Skeleton"),
            );

            self.objects.push(
                Node::new("Model")
                    .prop(model)
                    .prop(object_name(&bone.name, "Model"))
                    .prop("LimbNode")
                    .value("Version", 232)
                    .child(model_properties(t, r, s).build())
                    .value("MultiLayer", 0)
                    .value("MultiTake", 0)
                    .value("Shading", Property::Bool(true))
                    .value("Culling", "CullingOff"),
            );

            self.connect(attribute, model);
            models.push(model);
        }
        self.models += models.len();
        self.node_attributes += models.len();

        for (index, bone) in skeleton.bones.iter().enumerate() {
            let parent = match usize::try_from(bone.parent_index) {
                Ok(parent) => *models.get(parent).ok_or_else(|| Error::ExportFbx {
                    message: format!(
                        "bone {index} has invalid parent index {}",
                        bone.parent_index
                    ),
                })?,
                Err(_) => 0, // scene root
            };
            self.connect(models[index], parent);
        }

        Ok(models)
    }

    /// Creates one animation stack keyed at every Havok frame.
    ///
    /// Returns the stop time in KTime.
    fn add_animation(
        &mut self,
        models: &[i64],
        animation: &Animation,
        fps: f64,
        take_name: &str,
    ) -> i64 {
        let times: Vec<i64> = (0..animation.frames.len())
            .map(|frame| (frame as f64 * KTIME_PER_SECOND / fps).round() as i64)
            .collect();
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
                    self.objects
                        .push(anim_curve(curve, default[axis], times.clone(), values));
                    self.curves += 1;
                    self.connect_prop(curve, curve_node, axis_name);
                }
            }
        }

        stop
    }

    fn definitions(&self) -> Node {
        let types = [
            ("GlobalSettings", 1),
            ("Model", self.models),
            ("NodeAttribute", self.node_attributes),
            ("Geometry", self.geometries),
            ("Deformer", self.deformers),
            ("Pose", self.poses),
            ("Material", self.materials),
            ("Texture", self.textures),
            ("Video", self.videos),
            ("AnimationStack", 1),
            ("AnimationLayer", 1),
            ("AnimationCurveNode", self.curve_nodes),
            ("AnimationCurve", self.curves),
        ];
        let total: usize = types.iter().map(|(_, count)| count).sum();

        let mut definitions = Node::new("Definitions")
            .value("Version", 100)
            .value("Count", total as i32);
        for (name, count) in types.into_iter().filter(|(_, count)| *count > 0) {
            definitions.push(
                Node::new("ObjectType")
                    .prop(name)
                    .value("Count", count as i32),
            );
        }
        definitions
    }
}

/// Linear `AnimationCurve`.
fn anim_curve(id: i64, default: f64, times: Vec<i64>, values: Vec<f32>) -> Node {
    /// `eInterpolationLinear`
    const INTERPOLATION_LINEAR: i32 = 1 << 2;

    let count = times.len() as i32;
    Node::new("AnimationCurve")
        .prop(id)
        .prop(object_name("", "AnimCurve"))
        .prop("")
        .value("Default", default)
        .value("KeyVer", 4009)
        .value("KeyTime", Property::I64Array(times))
        .value("KeyValueFloat", Property::F32Array(values))
        // One attribute shared by all keys.
        .value(
            "KeyAttrFlags",
            Property::I32Array(vec![INTERPOLATION_LINEAR]),
        )
        .value(
            "KeyAttrDataFloat",
            Property::F32Array(vec![0.0, 0.0, f32::from_bits(218_434_821), 0.0]),
        )
        .value("KeyAttrRefCount", Property::I32Array(vec![count]))
}

fn header_extension() -> Node {
    Node::new("FBXHeaderExtension")
        .value("FBXHeaderVersion", 1003)
        .value("FBXVersion", node::FBX_VERSION as i32)
        .value("EncryptionType", 0)
        .value("Creator", CREATOR)
}

/// Same orientation and unit as nif_tool: Z up, -Y front, 1 Havok unit = 10cm.
fn global_settings(fps: f64, stop: i64) -> Node {
    Node::new("GlobalSettings").value("Version", 1000).child(
        Properties70::new()
            .int("UpAxis", 2)
            .int("UpAxisSign", 1)
            .int("FrontAxis", 1)
            .int("FrontAxisSign", -1)
            .int("CoordAxis", 0)
            .int("CoordAxisSign", 1)
            .int("OriginalUpAxis", 2)
            .int("OriginalUpAxisSign", 1)
            .double("UnitScaleFactor", 10.0)
            .double("OriginalUnitScaleFactor", 10.0)
            .color("AmbientColor", [0.0; 3])
            .string("DefaultCamera", "Producer Perspective")
            .enumeration("TimeMode", time_mode(fps))
            .enumeration("TimeProtocol", 2)
            .enumeration("SnapOnFrameMode", 0)
            .ktime("TimeSpanStart", 0)
            .ktime("TimeSpanStop", stop)
            .double("CustomFrameRate", fps)
            .int("CurrentTimeMarker", -1)
            .build(),
    )
}

fn takes(take_name: &str, stop: i64) -> Node {
    Node::new("Takes").value("Current", take_name).child(
        Node::new("Take")
            .prop(take_name)
            .value("FileName", format!("{take_name}.tak"))
            .child(Node::new("LocalTime").prop(0i64).prop(stop))
            .child(Node::new("ReferenceTime").prop(0i64).prop(stop)),
    )
}

/// FBX `TimeMode` for a frame rate.
///
/// Importers such as Blender only read `CustomFrameRate` when the mode is
/// `eCustom` (14).
fn time_mode(fps: f64) -> i32 {
    const MODES: [(f64, i32); 11] = [
        (120.0, 1),
        (100.0, 2),
        (60.0, 3),
        (50.0, 4),
        (48.0, 5),
        (30.0, 6),
        (25.0, 10),
        (24.0, 11),
        (1000.0, 12),
        (96.0, 15),
        (72.0, 16),
    ];

    MODES
        .iter()
        .find(|(rate, _)| (rate - fps).abs() < 1.0e-6)
        .map_or(14, |&(_, mode)| mode)
}

/// Frame rate of the sampled Havok frames.
fn animation_frame_rate(animation: &Animation) -> f64 {
    let intervals = animation.frames.len().saturating_sub(1);
    if intervals == 0 || !animation.duration.is_finite() || animation.duration <= 0.0 {
        return 30.0;
    }
    snap_frame_rate(intervals as f64 / f64::from(animation.duration))
}

/// Removes f32 noise such as `29.99998` so standard rates map to a `TimeMode`.
fn snap_frame_rate(fps: f64) -> f64 {
    let rounded = fps.round();
    if (fps - rounded).abs() < 1.0e-3 {
        rounded
    } else {
        fps
    }
}

/// World matrix of every skeleton model in its reference pose.
fn rest_world_matrices(skeleton: &Skeleton, models: &[i64]) -> HashMap<i64, Mat4> {
    let mut world: Vec<Mat4> = Vec::with_capacity(skeleton.bones.len());
    for bone in &skeleton.bones {
        let local = Mat4::from_qs_transform(&bone.reference_pose);
        let parent = usize::try_from(bone.parent_index)
            .ok()
            .and_then(|p| world.get(p).copied());
        world.push(parent.map_or(local, |parent| parent * local));
    }
    models.iter().copied().zip(world).collect()
}

/// Converts a Havok transform to FBX translation, XYZ Euler degrees, and scale.
///
/// `prev_euler` keeps consecutive keys on the closest equivalent Euler angles.
fn transform_to_trs(transform: &QsTransform, prev_euler: Option<[f64; 3]>) -> [[f64; 3]; 3] {
    let t = &transform.transition;
    let s = &transform.scale;

    let q = &transform.quaternion;
    let euler = quat_to_euler([q.x, q.y, q.z, q.scaler].map(f64::from));
    let euler = prev_euler.map_or(euler, |prev| compatible_euler(euler, prev));

    [
        [f64::from(t.x), f64::from(t.y), f64::from(t.z)],
        euler,
        [f64::from(s.x), f64::from(s.y), f64::from(s.z)],
    ]
}

/// `Properties70` of a `Model` with the given local transform.
fn model_properties(t: [f64; 3], r: [f64; 3], s: [f64; 3]) -> Properties70 {
    Properties70::new()
        .enumeration("RotationOrder", 0) // eEulerXYZ
        .enumeration("InheritType", 1) // eInheritRSrs
        .int("DefaultAttributeIndex", 0)
        .lcl("Lcl Translation", t)
        .lcl("Lcl Rotation", r)
        .lcl("Lcl Scaling", s)
}

/// Converts a quaternion `(x, y, z, w)` to FBX `eEulerXYZ` angles in degrees.
///
/// `eEulerXYZ` applies X first, i.e. `R = Rz * Ry * Rx`.
fn quat_to_euler([mut x, mut y, mut z, mut w]: [f64; 4]) -> [f64; 3] {
    let length = w.mul_add(w, z.mul_add(z, y.mul_add(y, x * x))).sqrt();
    if length > f64::EPSILON {
        x /= length;
        y /= length;
        z /= length;
        w /= length;
    } else {
        return [0.0; 3];
    }

    let roll = (2.0 * y.mul_add(z, w * x)).atan2(2.0f64.mul_add(-y.mul_add(y, x * x), 1.0));
    let sin_y = 2.0 * z.mul_add(-x, w * y);
    let pitch = sin_y.clamp(-1.0, 1.0).asin();
    let yaw = (2.0 * x.mul_add(y, w * z)).atan2(2.0f64.mul_add(-z.mul_add(z, y * y), 1.0));

    [roll.to_degrees(), pitch.to_degrees(), yaw.to_degrees()]
}

/// Picks the Euler representation of the same rotation closest to `prev`.
///
/// Curves are interpolated per component, so a jump such as 179° -> -179°
/// spins the bone almost a full turn between two keys. Both XYZ solutions
/// `(x, y, z)` and `(x + 180, 180 - y, z + 180)` are unwrapped by multiples of
/// 360° toward `prev`, and the nearer one is used.
fn compatible_euler(euler: [f64; 3], prev: [f64; 3]) -> [f64; 3] {
    fn unwrap(e: [f64; 3], prev: [f64; 3]) -> [f64; 3] {
        core::array::from_fn(|i| ((prev[i] - e[i]) / 360.0).round().mul_add(360.0, e[i]))
    }

    fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
        (0..3).map(|i| (a[i] - b[i]).abs()).sum()
    }

    let direct = unwrap(euler, prev);
    let flipped = unwrap([euler[0] + 180.0, 180.0 - euler[1], euler[2] + 180.0], prev);

    if distance(flipped, prev) < distance(direct, prev) {
        flipped
    } else {
        direct
    }
}

fn validate_animation(skeleton: &Skeleton, animation: &Animation) -> Result<(), Error> {
    let bone_count = skeleton.bones.len();

    if animation.num_tracks as usize > skeleton.bones.len() {
        return Err(Error::InvalidTrackCount {
            actual: animation.num_tracks as usize,
            maximum: skeleton.bones.len(),
        });
    }

    if animation.num_frames as usize != animation.frames.len() {
        return Err(Error::EncoderFrameCountMismatch {
            expected: animation.num_frames as usize,
            actual: animation.frames.len(),
        });
    }

    for (frame_index, frame) in animation.frames.iter().enumerate() {
        if frame.transforms.len() != bone_count {
            return Err(Error::EncoderTransformCountMismatch {
                frame_index,
                expected: bone_count,
                actual: frame.transforms.len(),
            });
        }
    }

    Ok(())
}
