//! FBX document tree and its binary/ASCII encoders.
//!
//! An FBX file is a tree of nodes. Every node has a name, a list of typed
//! properties and child nodes. Both the binary and ASCII formats are plain
//! encodings of that tree, so the scene is built once as [`Node`]s and then
//! written in either format.
//!
//! Binary layout (version < 7500, 32-bit offsets; 7500+ uses u64 for the
//! three node header fields and a 25-byte null record):
//!
//! ```text
//! header : "Kaydara FBX Binary  \0\x1a\0" (23 bytes) + u32 version (7400)
//! node   : u32 end_offset   absolute file offset just past this node
//!          u32 num_props    number of properties
//!          u32 props_len    byte length of the property list
//!          u8  name_len     then `name_len` bytes of name (no NUL)
//!          props...         `num_props` typed values (below)
//!          children...      nested nodes
//!          [null record]    13 zero bytes, present when the node has children
//!                           (the FBX SDK also writes it for nodes without properties)
//! root   : node... | null record | footer
//! ```
//!
//! Property encoding: a one-byte type code, then the value (little endian).
//!
//! | code | type | payload |
//! |---|---|---|
//! | `C` | bool | 1 byte (0 / 1) |
//! | `Y` | i16 | 2 bytes |
//! | `I` | i32 | 4 bytes |
//! | `L` | i64 | 8 bytes (object ids, KTime) |
//! | `F` | f32 | 4 bytes |
//! | `D` | f64 | 8 bytes |
//! | `S` | string | u32 length + bytes (not NUL-terminated) |
//! | `R` | raw bytes | u32 length + bytes (embedded files) |
//! | `b` `i` `l` `f` `d` | arrays of bool / i32 / i64 / f32 / f64 | u32 count, u32 encoding (0 = raw, 1 = zlib), u32 byte length, data |
//!
//! In ASCII files the same tree is written as text:
//!
//! ```text
//! Name: 1, "text", 2.5 {          ; properties separated by commas
//!     Child: "x"
//!     Array: *3 { a: 1,2,3 }      ; arrays: *<count> { a: values }
//! }
//! ```
//!
//! ref: <https://code.blender.org/2013/08/fbx-binary-file-format-specification/>

use std::fmt::Write as _;

/// FBX version written by this module.
///
/// 7400 is the last version with 32-bit node offsets and is read by every
/// importer that matters (Blender, Maya, Unity, ufbx).
pub const FBX_VERSION: u32 = 7400;

/// Separator between an object name and its class in binary files.
///
/// Binary: `"Pelvis\0\x01Model"`, ASCII: `"Model::Pelvis"`. The class is the
/// FBX SDK class (`Model`, `Geometry`, `NodeAttribute`, `AnimCurve`, ...);
/// the name may be empty (`"\0\x01AnimCurve"`).
const NAME_CLASS_SEPARATOR: &str = "\0\x01";

/// A typed FBX property value.
#[derive(Debug, Clone)]
pub enum Property {
    /// `C`
    Bool(bool),
    /// `I`
    I32(i32),
    /// `L`
    I64(i64),
    /// `D`
    F64(f64),
    /// `S`
    String(String),
    /// `R`
    Raw(Vec<u8>),
    /// `i`
    I32Array(Vec<i32>),
    /// `l`
    I64Array(Vec<i64>),
    /// `f`
    F32Array(Vec<f32>),
    /// `d`
    F64Array(Vec<f64>),
}

impl From<bool> for Property {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i32> for Property {
    fn from(value: i32) -> Self {
        Self::I32(value)
    }
}

impl From<i64> for Property {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}

impl From<f64> for Property {
    fn from(value: f64) -> Self {
        Self::F64(value)
    }
}

impl From<&str> for Property {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for Property {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

/// One FBX node.
#[derive(Debug, Clone, Default)]
pub struct Node {
    pub name: &'static str,
    pub props: Vec<Property>,
    pub children: Vec<Self>,
}

impl Node {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            props: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Adds a property.
    #[must_use]
    pub fn prop(mut self, value: impl Into<Property>) -> Self {
        self.props.push(value.into());
        self
    }

    /// Adds a child node.
    #[must_use]
    pub fn child(mut self, child: Self) -> Self {
        self.children.push(child);
        self
    }

    /// Adds `name: value` as a child node.
    #[must_use]
    pub fn value(self, name: &'static str, value: impl Into<Property>) -> Self {
        self.child(Self::new(name).prop(value))
    }

    pub fn push(&mut self, child: Self) {
        self.children.push(child);
    }
}

/// Builds `"<name>\0\x01<class>"`, the object name format of binary FBX.
pub fn object_name(name: &str, class: &str) -> String {
    format!("{name}{NAME_CLASS_SEPARATOR}{class}")
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
pub struct Properties70(Node);

impl Properties70 {
    pub const fn new() -> Self {
        Self(Node::new("Properties70"))
    }

    fn p(&mut self, name: &str, ty: &str, label: &str, flags: &str) -> &mut Node {
        self.0
            .push(Node::new("P").prop(name).prop(ty).prop(label).prop(flags));
        #[allow(clippy::unwrap_used)] // Just pushed.
        self.0.children.last_mut().unwrap()
    }

    #[must_use]
    pub fn int(mut self, name: &str, value: i32) -> Self {
        self.p(name, "int", "Integer", "").props.push(value.into());
        self
    }

    #[must_use]
    pub fn enumeration(mut self, name: &str, value: i32) -> Self {
        self.p(name, "enum", "", "").props.push(value.into());
        self
    }

    #[must_use]
    pub fn double(mut self, name: &str, value: f64) -> Self {
        self.p(name, "double", "Number", "")
            .props
            .push(value.into());
        self
    }

    #[must_use]
    pub fn ktime(mut self, name: &str, value: i64) -> Self {
        self.p(name, "KTime", "Time", "").props.push(value.into());
        self
    }

    #[must_use]
    pub fn string(mut self, name: &str, value: &str) -> Self {
        self.p(name, "KString", "", "").props.push(value.into());
        self
    }

    #[must_use]
    pub fn color(mut self, name: &str, value: [f64; 3]) -> Self {
        let p = self.p(name, "ColorRGB", "Color", "");
        p.props.extend(value.map(Property::F64));
        self
    }

    /// Animatable number, e.g. `d|X` of an `AnimationCurveNode`.
    #[must_use]
    pub fn number(mut self, name: &str, value: f64) -> Self {
        self.p(name, "Number", "", "A").props.push(value.into());
        self
    }

    /// Arbitrary entry: `P: name, ty, label, flags, values...`.
    #[must_use]
    pub fn entry(
        mut self,
        name: &str,
        ty: &str,
        label: &str,
        flags: &str,
        values: impl IntoIterator<Item = Property>,
    ) -> Self {
        self.p(name, ty, label, flags).props.extend(values);
        self
    }

    /// `Lcl Translation` / `Lcl Rotation` / `Lcl Scaling`.
    #[must_use]
    pub fn lcl(mut self, name: &str, value: [f64; 3]) -> Self {
        let p = self.p(name, name, "", "A");
        p.props.extend(value.map(Property::F64));
        self
    }

    pub fn build(self) -> Node {
        self.0
    }
}

// ---------------------------------------------------------------------------
// Binary

const BINARY_MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";
const NULL_RECORD: [u8; 13] = [0; 13];
const FOOTER_ID: [u8; 16] = [
    0xfa, 0xbc, 0xab, 0x09, 0xd0, 0xc8, 0xd4, 0x66, 0xb1, 0x76, 0xfb, 0x83, 0x1c, 0xf7, 0x26, 0x7e,
];
const FOOTER_MAGIC: [u8; 16] = [
    0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b,
];

/// Encodes top-level nodes as a binary FBX file.
pub fn to_binary(nodes: &[Node]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1024 * 1024);
    out.extend_from_slice(BINARY_MAGIC);
    out.extend_from_slice(&FBX_VERSION.to_le_bytes());

    for node in nodes {
        write_binary_node(&mut out, node);
    }
    out.extend_from_slice(&NULL_RECORD);

    // Footer. Importers do not validate it, but the FBX SDK expects it.
    out.extend_from_slice(&FOOTER_ID);
    out.extend_from_slice(&[0; 4]);
    let padding = match out.len() % 16 {
        0 => 16,
        rem => 16 - rem,
    };
    out.resize(out.len() + padding, 0);
    out.extend_from_slice(&FBX_VERSION.to_le_bytes());
    out.extend_from_slice(&[0; 120]);
    out.extend_from_slice(&FOOTER_MAGIC);

    out
}

fn write_binary_node(out: &mut Vec<u8>, node: &Node) {
    let header_pos = out.len();
    out.extend_from_slice(&[0; 12]); // end_offset, num_props, props_len: patched below
    out.push(node.name.len() as u8);
    out.extend_from_slice(node.name.as_bytes());

    let props_start = out.len();
    for prop in &node.props {
        write_binary_prop(out, prop);
    }
    let props_len = out.len() - props_start;

    // A node with children is terminated by a null record. The FBX SDK also
    // writes it for nodes without properties.
    if !node.children.is_empty() || node.props.is_empty() {
        for child in &node.children {
            write_binary_node(out, child);
        }
        out.extend_from_slice(&NULL_RECORD);
    }

    let end_offset = out.len() as u32;
    out[header_pos..header_pos + 4].copy_from_slice(&end_offset.to_le_bytes());
    out[header_pos + 4..header_pos + 8].copy_from_slice(&(node.props.len() as u32).to_le_bytes());
    out[header_pos + 8..header_pos + 12].copy_from_slice(&(props_len as u32).to_le_bytes());
}

fn write_binary_prop(out: &mut Vec<u8>, prop: &Property) {
    fn array<T, const N: usize>(
        out: &mut Vec<u8>,
        code: u8,
        values: &[T],
        to_bytes: impl Fn(&T) -> [u8; N],
    ) {
        out.push(code);
        out.extend_from_slice(&(values.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // encoding: 0 = uncompressed
        out.extend_from_slice(&((values.len() * N) as u32).to_le_bytes());
        for value in values {
            out.extend_from_slice(&to_bytes(value));
        }
    }

    match prop {
        Property::Bool(value) => {
            out.push(b'C');
            out.push(u8::from(*value));
        }
        Property::I32(value) => {
            out.push(b'I');
            out.extend_from_slice(&value.to_le_bytes());
        }
        Property::I64(value) => {
            out.push(b'L');
            out.extend_from_slice(&value.to_le_bytes());
        }
        Property::F64(value) => {
            out.push(b'D');
            out.extend_from_slice(&value.to_le_bytes());
        }
        Property::String(value) => {
            out.push(b'S');
            out.extend_from_slice(&(value.len() as u32).to_le_bytes());
            out.extend_from_slice(value.as_bytes());
        }
        Property::Raw(value) => {
            out.push(b'R');
            out.extend_from_slice(&(value.len() as u32).to_le_bytes());
            out.extend_from_slice(value);
        }
        Property::I32Array(values) => array(out, b'i', values, |v| v.to_le_bytes()),
        Property::I64Array(values) => array(out, b'l', values, |v| v.to_le_bytes()),
        Property::F32Array(values) => array(out, b'f', values, |v| v.to_le_bytes()),
        Property::F64Array(values) => array(out, b'd', values, |v| v.to_le_bytes()),
    }
}

// ---------------------------------------------------------------------------
// ASCII

/// Encodes top-level nodes as an ASCII FBX file.
pub fn to_ascii(nodes: &[Node]) -> String {
    let mut out = String::with_capacity(1024 * 1024);
    let major = FBX_VERSION / 1000;
    let minor = FBX_VERSION % 1000 / 100;
    let _ = writeln!(out, "; FBX {major}.{minor}.0 project file");
    out.push_str("; ----------------------------------------------------\n\n");

    for node in nodes {
        write_ascii_node(&mut out, node, 0);
    }

    out
}

fn write_ascii_node(out: &mut String, node: &Node, depth: usize) {
    let indent = "\t".repeat(depth);
    let _ = write!(out, "{indent}{}: ", node.name);

    // Arrays are written as `Name: *N { a: ... }` and are never mixed with
    // other properties.
    if let [
        array @ (Property::I32Array(_)
        | Property::I64Array(_)
        | Property::F32Array(_)
        | Property::F64Array(_)),
    ] = node.props.as_slice()
    {
        let (len, values) = match array {
            Property::I32Array(v) => (v.len(), join(v.iter())),
            Property::I64Array(v) => (v.len(), join(v.iter())),
            Property::F32Array(v) => (v.len(), join(v.iter())),
            Property::F64Array(v) => (v.len(), join(v.iter())),
            _ => unreachable!(),
        };
        let _ = writeln!(out, "*{len} {{\n{indent}\ta: {values}\n{indent}}}");
        return;
    }

    for (index, prop) in node.props.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        write_ascii_prop(out, prop);
    }

    if node.children.is_empty() && !node.props.is_empty() {
        out.push('\n');
        return;
    }

    out.push_str(" {\n");
    for child in &node.children {
        write_ascii_node(out, child, depth + 1);
    }
    let _ = writeln!(out, "{indent}}}");
}

fn write_ascii_prop(out: &mut String, prop: &Property) {
    match prop {
        Property::Bool(value) => out.push(if *value { 'T' } else { 'F' }),
        Property::I32(value) => {
            let _ = write!(out, "{value}");
        }
        Property::I64(value) => {
            let _ = write!(out, "{value}");
        }
        Property::F64(value) => {
            let _ = write!(out, "{value}");
        }
        Property::String(value) => {
            // Binary `name\0\x01class` is `class::name` in ASCII.
            let value = match value.split_once(NAME_CLASS_SEPARATOR) {
                Some((name, class)) => format!("{class}::{name}"),
                None => value.clone(),
            };
            let _ = write!(out, "\"{}\"", value.replace('"', "&quot;"));
        }
        Property::Raw(value) => {
            out.push('"');
            for byte in value {
                let _ = write!(out, "{byte:02x}");
            }
            out.push('"');
        }
        Property::I32Array(_)
        | Property::I64Array(_)
        | Property::F32Array(_)
        | Property::F64Array(_) => {
            unreachable!("array properties are written by write_ascii_node")
        }
    }
}

fn join<T: std::fmt::Display>(values: impl Iterator<Item = T>) -> String {
    let mut out = String::new();
    for (index, value) in values.enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{value}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_node_offsets() {
        let nodes = [Node::new("A").prop(1i32).child(Node::new("B").prop("x"))];
        let bytes = to_binary(&nodes);

        let start = BINARY_MAGIC.len() + 4;
        let end_offset = u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap());
        // header(13) + "A"(1) + I(5) + child(13 + 1 + 6) + null(13)
        assert_eq!(end_offset as usize, start + 13 + 1 + 5 + 20 + 13);
        assert_eq!(
            &bytes[end_offset as usize..end_offset as usize + 13],
            &NULL_RECORD
        );
    }

    #[test]
    fn ascii_object_name() {
        let nodes = [Node::new("Model").prop(object_name("Pelvis", "Model"))];
        assert!(to_ascii(&nodes).contains("Model: \"Model::Pelvis\""));
    }
}
