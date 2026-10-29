//! FBX document tree, encoded by [`super::binary`] or [`super::ascii`].
//!
//! An FBX file is a tree of nodes `Name: prop, prop, ... { children }`. The
//! scene is built once as [`Node`]s and then written in either format.
//!
//! Properties borrow (`Cow`) whatever outlives the export call: key times
//! shared by thousands of curves, cluster weights, embedded texture files.
//! Only names created on the fly (`"Pelvis\0\x01Model"`) are owned.

use std::borrow::Cow;

/// Separator between an object name and its class in binary files.
///
/// Binary: `"Pelvis\0\x01Model"`, ASCII: `"Model::Pelvis"`. The class is the
/// FBX SDK class (`Model`, `Geometry`, `NodeAttribute`, `AnimCurve`, ...);
/// the name may be empty (`"\0\x01AnimCurve"`).
pub const NAME_CLASS_SEPARATOR: &str = "\0\x01";

/// Builds `"<name>\0\x01<class>"`, the object name format of binary FBX.
pub fn object_name(name: &str, class: &str) -> String {
    format!("{name}{NAME_CLASS_SEPARATOR}{class}")
}

/// A typed FBX property value; the variant decides the binary type code.
#[derive(Debug, Clone)]
pub enum Property<'a> {
    /// `C`
    Bool(bool),
    /// `I`
    I32(i32),
    /// `L`: object ids and KTime values.
    I64(i64),
    /// `D`
    F64(f64),
    /// `S`
    String(Cow<'a, str>),
    /// `R`: embedded files (`Video.Content`) and `FileId`.
    Raw(Cow<'a, [u8]>),
    /// `i`
    I32Array(Cow<'a, [i32]>),
    /// `l`
    I64Array(Cow<'a, [i64]>),
    /// `f`
    F32Array(Cow<'a, [f32]>),
    /// `d`
    F64Array(Cow<'a, [f64]>),
}

macro_rules! impl_from {
    ($($ty:ty => $variant:ident),* $(,)?) => {$(
        impl From<$ty> for Property<'_> {
            fn from(value: $ty) -> Self {
                Self::$variant(value)
            }
        }
    )*};
}
impl_from!(bool => Bool, i32 => I32, i64 => I64, f64 => F64);

macro_rules! impl_from_cow {
    ($($owned:ty, $borrowed:ty => $variant:ident),* $(,)?) => {$(
        impl From<$owned> for Property<'_> {
            fn from(value: $owned) -> Self {
                Self::$variant(Cow::Owned(value))
            }
        }
        impl<'a> From<&'a $borrowed> for Property<'a> {
            fn from(value: &'a $borrowed) -> Self {
                Self::$variant(Cow::Borrowed(value))
            }
        }
    )*};
}
impl_from_cow!(
    String, str => String,
    Vec<i32>, [i32] => I32Array,
    Vec<i64>, [i64] => I64Array,
    Vec<f32>, [f32] => F32Array,
    Vec<f64>, [f64] => F64Array,
);

impl Property<'_> {
    pub const fn is_array(&self) -> bool {
        matches!(
            self,
            Self::I32Array(_) | Self::I64Array(_) | Self::F32Array(_) | Self::F64Array(_)
        )
    }
}

/// One FBX node.
#[derive(Debug, Clone, Default)]
pub struct Node<'a> {
    /// Node names are always fixed FBX keywords, hence `'static`.
    pub name: &'static str,
    pub props: Vec<Property<'a>>,
    pub children: Vec<Self>,
}

impl<'a> Node<'a> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            props: Vec::new(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn prop(mut self, value: impl Into<Property<'a>>) -> Self {
        self.props.push(value.into());
        self
    }

    #[must_use]
    pub fn child(mut self, child: Self) -> Self {
        self.children.push(child);
        self
    }

    /// Adds `name: value` as a child node, the common "field" shape.
    #[must_use]
    pub fn value(self, name: &'static str, value: impl Into<Property<'a>>) -> Self {
        self.child(Self::new(name).prop(value))
    }

    pub fn push(&mut self, child: Self) {
        self.children.push(child);
    }
}

/// `Properties70` builder.
///
/// Each entry is `P: name, type, label, flags, values...`:
///
/// | method | type, label | values |
/// |---|---|---|
/// | [`int`](Self::int) | `"int", "Integer"` | `I` |
/// | [`enumeration`](Self::enumeration) | `"enum", ""` | `I` (meaning depends on the property) |
/// | [`double`](Self::double) | `"double", "Number"` | `D` |
/// | [`ktime`](Self::ktime) | `"KTime", "Time"` | `L` (KTime units) |
/// | [`string`](Self::string) | `"KString", ""` | `S` |
/// | [`color`](Self::color) | `"ColorRGB", "Color"` | `D D D` (0..=1) |
/// | [`number`](Self::number) | `"Number", ""` + flag `A` | `D` (animatable) |
/// | [`lcl`](Self::lcl) | `"Lcl Translation"` etc. + flag `A` | `D D D` |
///
/// `flags`: `A` = animatable, `U` = user defined, `H` = hidden; usually empty.
#[derive(Debug, Default)]
pub struct Properties70<'a>(Node<'a>);

impl<'a> Properties70<'a> {
    pub const fn new() -> Self {
        Self(Node::new("Properties70"))
    }

    /// Arbitrary entry: `P: name, ty, label, flags, values...`.
    #[must_use]
    pub fn entry(
        mut self,
        name: &'a str,
        ty: &'a str,
        label: &'a str,
        flags: &'a str,
        values: impl IntoIterator<Item = Property<'a>>,
    ) -> Self {
        let mut p = Node::new("P").prop(name).prop(ty).prop(label).prop(flags);
        p.props.extend(values);
        self.0.push(p);
        self
    }

    #[must_use]
    pub fn int(self, name: &'a str, value: i32) -> Self {
        self.entry(name, "int", "Integer", "", [value.into()])
    }

    #[must_use]
    pub fn enumeration(self, name: &'a str, value: i32) -> Self {
        self.entry(name, "enum", "", "", [value.into()])
    }

    #[must_use]
    pub fn double(self, name: &'a str, value: f64) -> Self {
        self.entry(name, "double", "Number", "", [value.into()])
    }

    #[must_use]
    pub fn ktime(self, name: &'a str, value: i64) -> Self {
        self.entry(name, "KTime", "Time", "", [value.into()])
    }

    #[must_use]
    pub fn string(self, name: &'a str, value: &'a str) -> Self {
        self.entry(name, "KString", "", "", [value.into()])
    }

    #[must_use]
    pub fn color(self, name: &'a str, value: [f64; 3]) -> Self {
        self.entry(name, "ColorRGB", "Color", "", value.map(Property::F64))
    }

    /// Animatable number, e.g. `d|X` of an `AnimationCurveNode`.
    #[must_use]
    pub fn number(self, name: &'a str, value: f64) -> Self {
        self.entry(name, "Number", "", "A", [value.into()])
    }

    /// `Lcl Translation` / `Lcl Rotation` / `Lcl Scaling`: the type name is
    /// the property name itself.
    #[must_use]
    pub fn lcl(self, name: &'a str, value: [f64; 3]) -> Self {
        self.entry(name, name, "", "A", value.map(Property::F64))
    }

    pub fn build(self) -> Node<'a> {
        self.0
    }
}
