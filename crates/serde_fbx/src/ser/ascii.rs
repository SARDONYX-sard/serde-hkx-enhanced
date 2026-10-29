//! ASCII FBX encoder.
//!
//! ```text
//! Name: 1, "text", 2.5 {          ; properties separated by commas
//!     Child: "x"
//!     Array: *3 { a: 1,2,3 }      ; arrays: *<count> { a: values }
//! }
//! ```

use std::fmt::{Display, Write as _};

use super::binary::FBX_VERSION;
use super::node::{NAME_CLASS_SEPARATOR, Node, Property};

/// Encodes top-level nodes as an ASCII FBX file.
pub fn to_ascii(nodes: &[Node<'_>]) -> String {
    let mut out = String::with_capacity(1024 * 1024);
    let (major, minor) = (FBX_VERSION / 1000, FBX_VERSION % 1000 / 100);
    let _ = writeln!(out, "; FBX {major}.{minor}.0 project file");
    out.push_str("; ----------------------------------------------------\n\n");

    for node in nodes {
        write_node(&mut out, node, 0);
    }
    out
}

// `write!` into a `String` cannot fail, hence the ignored results below.
fn write_node(out: &mut String, node: &Node<'_>, depth: usize) {
    let indent = "\t".repeat(depth);
    let _ = write!(out, "{indent}{}: ", node.name);

    // Arrays use a dedicated form and are never mixed with other properties.
    if let [array] = node.props.as_slice()
        && array.is_array()
    {
        let (len, values) = match array {
            Property::I32Array(v) => (v.len(), join(v.iter())),
            Property::I64Array(v) => (v.len(), join(v.iter())),
            Property::F32Array(v) => (v.len(), join(v.iter())),
            Property::F64Array(v) => (v.len(), join(v.iter())),
            _ => unreachable!("checked by is_array"),
        };
        let _ = writeln!(out, "*{len} {{\n{indent}\ta: {values}\n{indent}}}");
        return;
    }

    for (index, prop) in node.props.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        write_prop(out, prop);
    }

    if node.children.is_empty() && !node.props.is_empty() {
        out.push('\n');
        return;
    }

    out.push_str(" {\n");
    for child in &node.children {
        write_node(out, child, depth + 1);
    }
    let _ = writeln!(out, "{indent}}}");
}

fn write_prop(out: &mut String, prop: &Property<'_>) {
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
            out.push('"');
            match value.split_once(NAME_CLASS_SEPARATOR) {
                Some((name, class)) => {
                    out.push_str(&escape(class));
                    out.push_str("::");
                    out.push_str(&escape(name));
                }
                None => out.push_str(&escape(value)),
            }
            out.push('"');
        }
        Property::Raw(value) => {
            out.push('"');
            for byte in value.iter() {
                let _ = write!(out, "{byte:02x}");
            }
            out.push('"');
        }
        Property::I32Array(_)
        | Property::I64Array(_)
        | Property::F32Array(_)
        | Property::F64Array(_) => {
            unreachable!("array properties are written by write_node")
        }
    }
}

fn escape(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('"') {
        text.replace('"', "&quot;").into()
    } else {
        text.into()
    }
}

fn join<T: Display>(values: impl Iterator<Item = T>) -> String {
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
    use crate::ser::node::object_name;

    #[test]
    fn object_name_is_class_colon_name() {
        let nodes = [Node::new("Model").prop(object_name("Pelvis", "Model"))];
        assert!(to_ascii(&nodes).contains("Model: \"Model::Pelvis\""));
    }
}
