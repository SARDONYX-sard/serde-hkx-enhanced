//! FBX scene graph: models, animation stacks and curve evaluation.
//!
//! What is read (see [`crate::ser`] for the meaning of each object):
//!
//! | node | used for |
//! |---|---|
//! | `Objects > Model` | bones by name (`"Pelvis\0\x01Model"` / `"Model::Pelvis"`) and their `Properties70` |
//! | `Objects > AnimationStack` | clips (`LocalStart` / `LocalStop`) |
//! | `Objects > AnimationCurveNode` | `d|X` / `d|Y` / `d|Z` defaults |
//! | `Objects > AnimationCurve` | `KeyTime` (i64 KTime) / `KeyValueFloat` |
//! | `Connections` | layer -> stack, curve node -> layer, curve node -> bone property, curve -> channel |
//! | `Definitions > ObjectType "Model" > PropertyTemplate` | default `Properties70` values of every Model |
//!
//! A property missing from a Model's `Properties70` takes the template value;
//! missing from both, the FBX default (zero offsets, unit scale).
//!
//! Local node transform (FBX SDK convention), each term a 4x4 matrix from a
//! Model property:
//!
//! ```text
//! T * Roff * Rp * Rpre * R * Rpost^-1 * Rp^-1 * Soff * Sp * S * Sp^-1
//!
//! T     Lcl Translation        Rpre   PreRotation  (XYZ Euler degrees)
//! Roff  RotationOffset         R      Lcl Rotation (Euler degrees, order = RotationOrder)
//! Rp    RotationPivot          Rpost  PostRotation (XYZ Euler degrees)
//! Soff  ScalingOffset          S      Lcl Scaling
//! Sp    ScalingPivot
//! ```
//!
//! Files from Blender or this crate only use `T`, `R` and `S`; Maya/3ds Max
//! files commonly use pre-rotation and pivots.

use std::collections::HashMap;

use super::parse::{Element, Value};
use crate::Error;
use crate::math::Mat4;

/// FBX time units per second.
pub const KTIME_PER_SECOND: f64 = 46_186_158_000.0;

/// `Properties70` as `name -> values` (the values after `type, label, flags`).
#[derive(Debug, Clone, Default)]
struct Properties(HashMap<String, Vec<Value>>);

impl Properties {
    fn from_element(element: Option<&Element>) -> Self {
        let mut map = HashMap::new();
        if let Some(element) = element.and_then(|e| e.child("Properties70")) {
            for p in element.children_named("P") {
                if let Some(name) = p.prop(0).and_then(Value::as_str) {
                    map.insert(name.to_owned(), p.props.get(4..).unwrap_or_default().to_vec());
                }
            }
        }
        Self(map)
    }

    fn get(&self, name: &str) -> Option<&[Value]> {
        self.0.get(name).map(Vec::as_slice)
    }
}

/// An object property with a fallback to the `Definitions` template.
struct PropertyLookup<'a> {
    own: &'a Properties,
    template: &'a Properties,
}

impl PropertyLookup<'_> {
    fn values(&self, name: &str) -> Option<&[Value]> {
        self.own.get(name).or_else(|| self.template.get(name))
    }

    fn vec3(&self, name: &str, default: [f64; 3]) -> [f64; 3] {
        self.values(name)
            .filter(|v| v.len() >= 3)
            .map_or(default, |v| core::array::from_fn(|i| v[i].as_f64().unwrap_or(default[i])))
    }

    fn int(&self, name: &str, default: i64) -> i64 {
        self.values(name)
            .and_then(|v| v.first())
            .and_then(Value::as_i64)
            .unwrap_or(default)
    }
}

#[derive(Debug)]
pub struct Model {
    pub name: String,
    props: Properties,
    /// `(property name, curve node id)`, e.g. `("Lcl Rotation", 123)`.
    animated: Vec<(String, i64)>,
}

#[derive(Debug, Default)]
struct CurveNode {
    defaults: [Option<f64>; 3],
    curves: [Option<i64>; 3],
}

#[derive(Debug)]
struct Curve {
    times: Vec<i64>,
    values: Vec<f64>,
    default: f64,
}

impl Curve {
    /// Linear (or constant) evaluation; cubic keys are treated as linear.
    ///
    /// Before the first key the first value is held, after the last key the
    /// last value (FBX "constant" extrapolation). Sampling at the key times
    /// (as Havok exports and Blender bakes per frame) is exact either way.
    fn evaluate(&self, time: i64) -> f64 {
        let (Some(&first), Some(&last)) = (self.times.first(), self.times.last()) else {
            return self.default;
        };
        if time <= first {
            return self.values[0];
        }
        if time >= last {
            return self.values[self.values.len() - 1];
        }

        let next = self.times.partition_point(|&t| t <= time);
        let (t0, t1) = (self.times[next - 1], self.times[next]);
        let (v0, v1) = (self.values[next - 1], self.values[next]);
        let u = (time - t0) as f64 / (t1 - t0) as f64;
        (v1 - v0).mul_add(u, v0)
    }
}

#[derive(Debug)]
pub struct AnimStack {
    pub name: String,
    layers: Vec<i64>,
    pub local_start: Option<i64>,
    pub local_stop: Option<i64>,
}

#[derive(Debug, Default)]
pub struct Scene {
    pub models: HashMap<i64, Model>,
    pub stacks: Vec<AnimStack>,
    /// Object id of each entry in `stacks`.
    stack_ids: Vec<i64>,
    model_template: Properties,
    curve_nodes: HashMap<i64, CurveNode>,
    curves: HashMap<i64, Curve>,
    /// curve node id -> layer id
    curve_node_layer: HashMap<i64, i64>,
}

/// `"Pelvis\0\x01Model"` (binary) or `"Model::Pelvis"` (ASCII) -> `"Pelvis"`.
fn object_name(raw: &str) -> String {
    if let Some((name, _class)) = raw.split_once("\0\x01") {
        name.to_owned()
    } else if let Some((_class, name)) = raw.split_once("::") {
        name.to_owned()
    } else {
        raw.to_owned()
    }
}

impl Scene {
    /// Builds the scene from parsed top-level nodes.
    ///
    /// # Errors
    /// If `Objects` or `Connections` is missing.
    pub fn new(nodes: &[Element]) -> Result<Self, Error> {
        let find = |name: &str| nodes.iter().find(|node| node.name == name);
        let objects = find("Objects").ok_or_else(|| Error::LoadFbx {
            message: "no 'Objects' section".to_owned(),
        })?;
        let connections = find("Connections").ok_or_else(|| Error::LoadFbx {
            message: "no 'Connections' section".to_owned(),
        })?;

        let mut scene = Self {
            model_template: model_template(find("Definitions")),
            ..Default::default()
        };

        for object in &objects.children {
            let Some(id) = object.prop(0).and_then(Value::as_i64) else {
                continue;
            };
            let name = object
                .prop(1)
                .and_then(Value::as_str)
                .map(object_name)
                .unwrap_or_default();

            match object.name.as_str() {
                "Model" => {
                    scene.models.insert(
                        id,
                        Model {
                            name,
                            props: Properties::from_element(Some(object)),
                            animated: Vec::new(),
                        },
                    );
                }
                "AnimationStack" => {
                    let props = Properties::from_element(Some(object));
                    let ktime = |name: &str| props.get(name).and_then(|v| v.first()).and_then(Value::as_i64);
                    scene.stacks.push(AnimStack {
                        name,
                        layers: Vec::new(),
                        local_start: ktime("LocalStart"),
                        local_stop: ktime("LocalStop"),
                    });
                    scene.stack_ids.push(id);
                }
                "AnimationCurveNode" => {
                    let props = Properties::from_element(Some(object));
                    let mut node = CurveNode::default();
                    for (i, channel) in ["d|X", "d|Y", "d|Z"].into_iter().enumerate() {
                        node.defaults[i] = props.get(channel).and_then(|v| v.first()).and_then(Value::as_f64);
                    }
                    scene.curve_nodes.insert(id, node);
                }
                "AnimationCurve" => {
                    let array = |name: &str| object.child(name).and_then(|e| e.prop(0));
                    let times = array("KeyTime").and_then(Value::to_i64_vec).unwrap_or_default();
                    let values = array("KeyValueFloat").and_then(Value::to_f64_vec).unwrap_or_default();
                    let len = times.len().min(values.len());
                    scene.curves.insert(
                        id,
                        Curve {
                            times: times[..len].to_vec(),
                            values: values[..len].to_vec(),
                            default: array("Default").and_then(Value::as_f64).unwrap_or(0.0),
                        },
                    );
                }
                _ => {}
            }
        }

        scene.connect(connections);
        Ok(scene)
    }

    fn connect(&mut self, connections: &Element) {
        for c in connections.children_named("C") {
            let kind = c.prop(0).and_then(Value::as_str).unwrap_or_default();
            let (Some(child), Some(parent)) = (
                c.prop(1).and_then(Value::as_i64),
                c.prop(2).and_then(Value::as_i64),
            ) else {
                continue;
            };
            let property = c.prop(3).and_then(Value::as_str);

            match (kind, property) {
                // curve -> curve node channel
                ("OP", Some(channel)) if self.curves.contains_key(&child) => {
                    if let Some(node) = self.curve_nodes.get_mut(&parent) {
                        let axis = match channel {
                            "d|X" => 0,
                            "d|Y" => 1,
                            "d|Z" => 2,
                            _ => continue,
                        };
                        node.curves[axis] = Some(child);
                    }
                }
                // curve node -> model property
                ("OP", Some(property)) if self.curve_nodes.contains_key(&child) => {
                    if let Some(model) = self.models.get_mut(&parent) {
                        model.animated.push((property.to_owned(), child));
                    }
                }
                ("OO", _) => {
                    if self.curve_nodes.contains_key(&child) {
                        self.curve_node_layer.insert(child, parent);
                    } else if let Some(index) = self.stack_ids.iter().position(|&id| id == parent) {
                        self.stacks[index].layers.push(child);
                    }
                }
                _ => {}
            }
        }
    }

    /// Time range covered by the curves animating `models` in `stack`.
    pub fn curve_time_range(&self, stack: &AnimStack, models: &[i64]) -> Option<(i64, i64)> {
        let mut range: Option<(i64, i64)> = None;
        for curve in self.stack_curves(stack, models) {
            if let (Some(&first), Some(&last)) = (curve.times.first(), curve.times.last()) {
                range = Some(range.map_or((first, last), |(a, b)| (a.min(first), b.max(last))));
            }
        }
        range
    }

    fn stack_curves<'a>(&'a self, stack: &'a AnimStack, models: &'a [i64]) -> impl Iterator<Item = &'a Curve> {
        models
            .iter()
            .filter_map(|id| self.models.get(id))
            .flat_map(|model| model.animated.iter())
            .filter(|(_, node)| self.in_stack(stack, *node))
            .filter_map(|(_, node)| self.curve_nodes.get(node))
            .flat_map(|node| node.curves.iter().flatten())
            .filter_map(|curve| self.curves.get(curve))
    }

    fn in_stack(&self, stack: &AnimStack, curve_node: i64) -> bool {
        self.curve_node_layer
            .get(&curve_node)
            .is_none_or(|layer| stack.layers.contains(layer))
    }

    /// Local transform of `model` at `time` (KTime) in `stack`.
    pub fn local_transform(&self, model_id: i64, stack: Option<&AnimStack>, time: i64) -> Mat4 {
        let Some(model) = self.models.get(&model_id) else {
            return Mat4::IDENTITY;
        };
        let props = PropertyLookup {
            own: &model.props,
            template: &self.model_template,
        };

        let mut translation = props.vec3("Lcl Translation", [0.0; 3]);
        let mut rotation = props.vec3("Lcl Rotation", [0.0; 3]);
        let mut scaling = props.vec3("Lcl Scaling", [1.0; 3]);

        if let Some(stack) = stack {
            for (property, node_id) in &model.animated {
                if !self.in_stack(stack, *node_id) {
                    continue;
                }
                let target = match property.as_str() {
                    "Lcl Translation" => &mut translation,
                    "Lcl Rotation" => &mut rotation,
                    "Lcl Scaling" => &mut scaling,
                    _ => continue,
                };
                let Some(node) = self.curve_nodes.get(node_id) else {
                    continue;
                };
                for ((value, curve), default) in target.iter_mut().zip(node.curves).zip(node.defaults) {
                    if let Some(curve) = curve.and_then(|id| self.curves.get(&id)) {
                        *value = curve.evaluate(time);
                    } else if let Some(default) = default {
                        *value = default;
                    }
                }
            }
        }

        let order = props.int("RotationOrder", 0);
        let rotation_offset = Mat4::translation(props.vec3("RotationOffset", [0.0; 3]));
        let rotation_pivot = Mat4::translation(props.vec3("RotationPivot", [0.0; 3]));
        let pre = Mat4::from_euler_degrees(props.vec3("PreRotation", [0.0; 3]), 0);
        let post = Mat4::from_euler_degrees(props.vec3("PostRotation", [0.0; 3]), 0);
        let scaling_offset = Mat4::translation(props.vec3("ScalingOffset", [0.0; 3]));
        let scaling_pivot = Mat4::translation(props.vec3("ScalingPivot", [0.0; 3]));

        Mat4::translation(translation)
            * rotation_offset
            * rotation_pivot
            * pre
            * Mat4::from_euler_degrees(rotation, order)
            * post.inverse()
            * rotation_pivot.inverse()
            * scaling_offset
            * scaling_pivot
            * Mat4::scale(scaling)
            * scaling_pivot.inverse()
    }
}

/// `Definitions > ObjectType "Model" > PropertyTemplate > Properties70`.
fn model_template(definitions: Option<&Element>) -> Properties {
    definitions
        .into_iter()
        .flat_map(|d| d.children_named("ObjectType"))
        .find(|t| t.prop(0).and_then(Value::as_str) == Some("Model"))
        .and_then(|t| t.child("PropertyTemplate"))
        .map(|t| Properties::from_element(Some(t)))
        .unwrap_or_default()
}
