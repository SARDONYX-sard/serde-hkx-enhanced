//! Prints a summary of a NIF scene.
//!
//! ```sh
//! cargo run -p serde_nif --example nif_dump -- path/to/mesh.nif
//! ```
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: nif_dump <file.nif>")?;
    let bytes = std::fs::read(path)?;
    let scene = serde_nif::from_bytes(&bytes)?;

    println!("nodes: {}", scene.nodes.len());
    for (index, node) in scene.nodes.iter().enumerate() {
        let world = scene.world_transform(index);
        println!(
            "  [{index}] {:?} parent={:?} world_t={:?}",
            node.name, node.parent, world.translation
        );
    }

    for shape in &scene.shapes {
        println!(
            "shape {:?}: {} vertices, {} triangles, normals={}, uvs={}, textures={:?}",
            shape.name,
            shape.vertices.len(),
            shape.triangles.len(),
            shape.normals.len(),
            shape.uvs.len(),
            shape.textures
        );
        if let Some(skin) = &shape.skin {
            let weighted = skin.weights.iter().filter(|w| !w.is_empty()).count();
            let max_index = shape.triangles.iter().flatten().max();
            let sum_err = skin
                .weights
                .iter()
                .map(|w| (w.iter().map(|(_, w)| w).sum::<f32>() - 1.0).abs())
                .fold(0.0f32, f32::max);
            println!(
                "  skin: {} bones, {weighted} weighted vertices, max weight-sum error {sum_err}, max tri index {max_index:?}",
                skin.bones.len()
            );
            println!("  global_to_skin {:?}", skin.global_to_skin);
            let shape_world = scene.world_transform(shape.node);
            let skin_to_global = skin.global_to_skin.inverse();
            for (bone, t) in skin.bones.iter().zip(&skin.skin_to_bone) {
                let via_skin = skin_to_global.compose(&t.inverse()).translation;
                let via_shape = shape_world.compose(&t.inverse()).translation;
                println!(
                    "  bind {bone:?} via_global_to_skin={via_skin:?} via_shape_world={via_shape:?}"
                );
            }
        }
    }
    Ok(())
}
