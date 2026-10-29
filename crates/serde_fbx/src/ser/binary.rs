//! Binary FBX encoder.
//!
//! Layout (version < 7500, 32-bit offsets; 7500+ uses u64 for the three node
//! header fields and a 25-byte null record):
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
//! ref: <https://code.blender.org/2013/08/fbx-binary-file-format-specification/>

use super::node::{Node, Property};

/// FBX version written by this crate.
///
/// 7400 is the last version with 32-bit node offsets and is read by every
/// importer that matters (Blender, Maya, Unity, ufbx).
pub const FBX_VERSION: u32 = 7400;

const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";
const NULL_RECORD: [u8; 13] = [0; 13];
const FOOTER_ID: [u8; 16] = [
    0xfa, 0xbc, 0xab, 0x09, 0xd0, 0xc8, 0xd4, 0x66, 0xb1, 0x76, 0xfb, 0x83, 0x1c, 0xf7, 0x26, 0x7e,
];
const FOOTER_MAGIC: [u8; 16] = [
    0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b,
];

/// Encodes top-level nodes as a binary FBX file.
pub fn to_binary(nodes: &[Node<'_>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1024 * 1024);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FBX_VERSION.to_le_bytes());

    for node in nodes {
        write_node(&mut out, node);
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

fn write_node(out: &mut Vec<u8>, node: &Node<'_>) {
    // The header holds sizes only known after writing the node, so it is
    // reserved first and patched at the end.
    let header = out.len();
    out.extend_from_slice(&[0; 12]);
    out.push(node.name.len() as u8);
    out.extend_from_slice(node.name.as_bytes());

    let props_start = out.len();
    for prop in &node.props {
        write_prop(out, prop);
    }
    let props_len = out.len() - props_start;

    if !node.children.is_empty() || node.props.is_empty() {
        for child in &node.children {
            write_node(out, child);
        }
        out.extend_from_slice(&NULL_RECORD);
    }

    let end_offset = out.len() as u32;
    out[header..header + 4].copy_from_slice(&end_offset.to_le_bytes());
    out[header + 4..header + 8].copy_from_slice(&(node.props.len() as u32).to_le_bytes());
    out[header + 8..header + 12].copy_from_slice(&(props_len as u32).to_le_bytes());
}

fn write_prop(out: &mut Vec<u8>, prop: &Property<'_>) {
    fn array<T, const N: usize>(
        out: &mut Vec<u8>,
        code: u8,
        values: &[T],
        to_bytes: impl Fn(&T) -> [u8; N],
    ) {
        out.push(code);
        out.extend_from_slice(&(values.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // encoding: uncompressed
        out.extend_from_slice(&((values.len() * N) as u32).to_le_bytes());
        for value in values {
            out.extend_from_slice(&to_bytes(value));
        }
    }

    fn sized(out: &mut Vec<u8>, code: u8, bytes: &[u8]) {
        out.push(code);
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }

    match prop {
        Property::Bool(value) => out.extend_from_slice(&[b'C', u8::from(*value)]),
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
        Property::String(value) => sized(out, b'S', value.as_bytes()),
        Property::Raw(value) => sized(out, b'R', value),
        Property::I32Array(values) => array(out, b'i', values, |v| v.to_le_bytes()),
        Property::I64Array(values) => array(out, b'l', values, |v| v.to_le_bytes()),
        Property::F32Array(values) => array(out, b'f', values, |v| v.to_le_bytes()),
        Property::F64Array(values) => array(out, b'd', values, |v| v.to_le_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_offsets() {
        let nodes = [Node::new("A").prop(1i32).child(Node::new("B").prop("x"))];
        let bytes = to_binary(&nodes);

        let start = MAGIC.len() + 4;
        let end_offset = u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap()) as usize;
        // header(13) + "A"(1) + I(5) + child(13 + 1 + 6) + null record(13)
        assert_eq!(end_offset, start + 13 + 1 + 5 + 20 + 13);
        assert_eq!(&bytes[end_offset..end_offset + 13], &NULL_RECORD);
    }
}
