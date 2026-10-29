//! `Geometry` objects: polygon data and layer elements.

use super::mesh::Mesh;
use super::node::{Node, Properties70, object_name};

/// `Geometry` of type `Mesh`: the polygon data.
///
/// ``text
/// Geometry: <id>, "Geometry::<mesh>", "Mesh" {
///     GeometryVersion: 124
///     Vertices: *9 { a: x0,y0,z0, x1,y1,z1, x2,y2,z2 }   ; f64, mesh-local space
///     PolygonVertexIndex: *3 { a: 0, 1, -3 }           ; see below
///     LayerElementMaterial: 0 { ... }   ; material per polygon
///     LayerElementNormal: 0 { ... }     ; normals
///     LayerElementUV: 0 { ... }         ; texture coordinates
///     Layer: 0 {                        ; which elements form layer 0
///         Version: 100
///         LayerElement: { Type: "LayerElementNormal"  TypedIndex: 0 }
///         ...
///     }
/// }
/// ``
///
/// `PolygonVertexIndex` lists the vertex indices of every polygon in order;
/// the last index of a polygon is stored as `-(index + 1)` (bitwise NOT) to
/// mark its end. Triangle `(0, 1, 2)` is `0, 1, -3`.
///
/// Every `LayerElement*` has the same shape:
///
/// - `MappingInformationType`: what each value belongs to
///   - `"ByVertice"` (sic): one per vertex (`Vertices` order)
///   - `"ByPolygonVertex"`: one per entry of `PolygonVertexIndex` (corner)
///   - `"ByPolygon"`: one per polygon
///   - `"AllSame"`: one value for the whole mesh
/// - `ReferenceInformationType`:
///   - `"Direct"`: the values are listed in mapping order
///   - `"IndexToDirect"`: values are listed once, and an `...Index` array
///     maps each mapping slot to a value
pub(super) fn geometry_node(id: i64, mesh: &Mesh) -> Node<'static> {
    let vertices: Vec<f64> = mesh
        .vertices
        .iter()
        .flat_map(|v| v.map(f64::from))
        .collect();
    // The last index of each polygon is stored as `-(index + 1)`.
    let polygon_indices: Vec<i32> = mesh
        .triangles
        .iter()
        .flat_map(|&[a, b, c]| [a as i32, b as i32, -(c as i32) - 1])
        .collect();

    let mut geometry = Node::new("Geometry")
        .prop(id)
        .prop(object_name(&mesh.name, "Geometry"))
        .prop("Mesh")
        .child(Properties70::new().build())
        .value("GeometryVersion", 124)
        .value("Vertices", vertices)
        .value("PolygonVertexIndex", polygon_indices);

    let mut layer = Node::new("Layer").prop(0i32).value("Version", 100);

    // Every polygon uses material 0 (the first material connected to the model).
    if !mesh.textures.is_empty() {
        geometry.push(
            Node::new("LayerElementMaterial")
                .prop(0i32)
                .value("Version", 101)
                .value("Name", "")
                .value("MappingInformationType", "AllSame")
                .value("ReferenceInformationType", "IndexToDirect")
                .value("Materials", &[0i32][..]),
        );
        layer.push(layer_element("LayerElementMaterial"));
    }

    // One normal (x, y, z) per vertex, in `Vertices` order.
    if mesh.normals.len() == mesh.vertices.len() && !mesh.normals.is_empty() {
        geometry.push(
            Node::new("LayerElementNormal")
                .prop(0i32)
                .value("Version", 101)
                .value("Name", "")
                .value("MappingInformationType", "ByVertice")
                .value("ReferenceInformationType", "Direct")
                .value(
                    "Normals",
                    mesh.normals
                        .iter()
                        .flat_map(|n| n.map(f64::from))
                        .collect::<Vec<f64>>(),
                ),
        );
        layer.push(layer_element("LayerElementNormal"));
    }

    // UVs (u, v), v pointing up (0 = bottom of the image). One UV per vertex in
    // `UV`, and `UVIndex` (one per polygon corner) points into it; the same
    // indices as `PolygonVertexIndex` without the end markers.
    // `Name` is the UV map name referenced by `Texture.UVSet`.
    if mesh.uvs.len() == mesh.vertices.len() && !mesh.uvs.is_empty() {
        let uv_index: Vec<i32> = mesh
            .triangles
            .iter()
            .flat_map(|t| t.map(|i| i as i32))
            .collect();
        geometry.push(
            Node::new("LayerElementUV")
                .prop(0i32)
                .value("Version", 101)
                .value("Name", "UVMap")
                .value("MappingInformationType", "ByPolygonVertex")
                .value("ReferenceInformationType", "IndexToDirect")
                .value(
                    "UV",
                    mesh.uvs
                        .iter()
                        .flat_map(|uv| uv.map(f64::from))
                        .collect::<Vec<f64>>(),
                )
                .value("UVIndex", uv_index),
        );
        layer.push(layer_element("LayerElementUV"));
    }

    geometry.child(layer)
}

fn layer_element(element: &'static str) -> Node<'static> {
    Node::new("LayerElement")
        .value("Type", element)
        .value("TypedIndex", 0)
}
