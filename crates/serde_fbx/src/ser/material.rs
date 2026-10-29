//! Material, texture and video objects of a mesh.

use std::collections::HashMap;

use super::mesh::Mesh;
use super::node::{Node, Properties70, Property, object_name};
use super::scene::SceneBuilder;

impl<'a> SceneBuilder<'a> {
    /// `Material <- Texture <- Video`, connected to `model`.
    ///
    /// ``text
    /// Material: <id>, "Material::<mesh>", "" {
    ///     Version: 102
    ///     ShadingModel: "phong"                ; "lambert" or "phong"
    ///     Properties70: {
    ///         P: "DiffuseColor", "Color", "", "A", 1, 1, 1    ; multiplied with the diffuse texture
    ///         P: "DiffuseFactor", "Number", "", "A", 1
    ///     }
    /// }
    /// Texture: <id>, "Texture::highlandcow.dds", "" {
    ///     Type: "TextureVideoClip"   Version: 202
    ///     TextureName: "Texture::highlandcow.dds"
    ///     Properties70: { P: "UVSet", "KString", "", "", "UVMap" }  ; LayerElementUV name to use
    ///     Media: "Video::highlandcow.dds"      ; name of the Video object
    ///     FileName / RelativeFilename: "textures/actors/cow/highlandcow.dds"
    ///     ModelUVTranslation: 0, 0   ModelUVScaling: 1, 1   ; UV offset / tiling
    ///     Texture_Alpha_Source: "None"   Cropping: 0, 0, 0, 0
    /// }
    /// Video: <id>, "Video::highlandcow.dds", "Clip" {
    ///     Type: "Clip"
    ///     Properties70: { P: "Path", "KString", "XRefUrl", "", "<path>" }
    ///     Filename / RelativeFilename: "<path>"
    ///     Content: <raw file bytes>            ; optional embedded image
    /// }
    /// C: "OO", <material>, <mesh model>
    /// C: "OP", <texture>, <material>, "DiffuseColor"   ; or "NormalMap"
    /// C: "OO", <video>, <texture>
    /// ``
    ///
    /// `RelativeFilename` is relative to the FBX file and is what Blender
    /// tries first; `FileName` is the fallback (usually absolute).
    pub(super) fn add_material(
        &mut self,
        model: i64,
        mesh: &'a Mesh,
        videos: &mut HashMap<String, i64>,
    ) {
        let material = self.new_id();
        self.objects.push(
            Node::new("Material")
                .prop(material)
                .prop(object_name(&mesh.name, "Material"))
                .prop("")
                .value("Version", 102)
                .value("ShadingModel", "phong")
                .value("MultiLayer", 0)
                .child(
                    Properties70::new()
                        .string("ShadingModel", "phong")
                        .entry(
                            "DiffuseColor",
                            "Color",
                            "",
                            "A",
                            [1.0, 1.0, 1.0].map(Property::F64),
                        )
                        .entry("DiffuseFactor", "Number", "", "A", [Property::F64(1.0)])
                        .build(),
                ),
        );
        self.materials += 1;
        self.connect(material, model);

        for texture in &mesh.textures {
            let name = texture
                .path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&texture.path)
                .to_owned();

            let id = self.new_id();
            self.objects.push(
                Node::new("Texture")
                    .prop(id)
                    .prop(object_name(&name, "Texture"))
                    .prop("")
                    .value("Type", "TextureVideoClip")
                    .value("Version", 202)
                    .value("TextureName", object_name(&name, "Texture"))
                    .child(
                        Properties70::new()
                            .string("UVSet", "UVMap")
                            .entry("UseMaterial", "bool", "", "", [Property::I32(1)])
                            .build(),
                    )
                    .value("Media", object_name(&name, "Video"))
                    .value("FileName", texture.path.as_str())
                    .value("RelativeFilename", texture.path.as_str())
                    .child(Node::new("ModelUVTranslation").prop(0.0).prop(0.0))
                    .child(Node::new("ModelUVScaling").prop(1.0).prop(1.0))
                    .value("Texture_Alpha_Source", "None")
                    .child(
                        Node::new("Cropping")
                            .prop(0i32)
                            .prop(0i32)
                            .prop(0i32)
                            .prop(0i32),
                    ),
            );
            self.textures += 1;
            self.connect_prop(id, material, texture.slot.property());

            let video = match videos.get(&texture.path) {
                Some(&video) => video,
                None => {
                    let video = self.new_id();
                    let mut node = Node::new("Video")
                        .prop(video)
                        .prop(object_name(&name, "Video"))
                        .prop("Clip")
                        .value("Type", "Clip")
                        .child(
                            Properties70::new()
                                .entry(
                                    "Path",
                                    "KString",
                                    "XRefUrl",
                                    "",
                                    [Property::from(texture.path.as_str())],
                                )
                                .build(),
                        )
                        .value("UseMipMap", 0)
                        .value("Filename", texture.path.as_str())
                        .value("RelativeFilename", texture.path.as_str());
                    if let Some(content) = &texture.content {
                        node.push(
                            Node::new("Content").prop(Property::Raw(content.as_slice().into())),
                        );
                    }
                    self.objects.push(node);
                    self.videos += 1;
                    videos.insert(texture.path.clone(), video);
                    video
                }
            };
            self.connect(video, id);
        }
    }
}
