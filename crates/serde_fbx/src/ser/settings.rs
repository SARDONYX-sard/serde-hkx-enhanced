//! Top-level nodes describing the file: header, global settings, takes.

use super::binary::FBX_VERSION;
use super::node::{Node, Properties70};

pub(super) const CREATOR: &str = concat!("serde_fbx ", env!("CARGO_PKG_VERSION"));

/// Same fixed id as Blender's exporter. Only its presence matters.
pub(super) const FILE_ID: [u8; 16] = [
    0x28, 0xb3, 0x2a, 0xeb, 0xb6, 0x24, 0xcc, 0xc2, 0xbf, 0xc8, 0xb0, 0x2a, 0xa9, 0x2b, 0xfc, 0xf1,
];


/// `FBXHeaderExtension`: file format information.
///
/// - `FBXHeaderVersion`: version of this header block (1003 since FBX 2006);
/// - `FBXVersion`: same as the version in the binary header (7400 = FBX 2014/2015);
/// - `EncryptionType`: 0 = not encrypted;
/// - `Creator`: free text, the program that wrote the file.
pub(super) fn header_extension() -> Node<'static> {
    Node::new("FBXHeaderExtension")
        .value("FBXHeaderVersion", 1003)
        .value("FBXVersion", FBX_VERSION as i32)
        .value("EncryptionType", 0)
        .value("Creator", CREATOR)
}

/// Same orientation and unit as nif_tool: Z up, -Y front, 1 Havok unit = 10cm.
///
/// Axes (`*Axis`: 0 = X, 1 = Y, 2 = Z; `*AxisSign`: 1 or -1):
/// - `UpAxis` 2 / sign 1: +Z is up (Skyrim);
/// - `FrontAxis` 1 / sign -1: the model faces -Y;
/// - `CoordAxis` 0 / sign 1: +X is right (right-handed system).
///
/// Importers convert these axes to their own (Blender: Z up, -Y front, so no
/// conversion happens). `Original*` record the axes/unit of the authoring tool.
///
/// Unit: `UnitScaleFactor` = centimeters per file unit. 10 means 1 Havok unit
/// = 10 cm; Blender (meters) then imports 1 unit as 0.1 m.
///
/// Time:
/// - `TimeMode`: frame rate preset, see [`time_mode`];
/// - `TimeProtocol`: 2 = display as frames (0 = SMPTE timecode, 1 = frame count);
/// - `TimeSpanStart` / `TimeSpanStop`: scene time range in KTime;
/// - `CustomFrameRate`: frame rate when `TimeMode` is 14 (custom).
///
/// `AmbientColor`, `DefaultCamera`, `SnapOnFrameMode`, `CurrentTimeMarker`
/// are defaults written by every exporter; importers ignore them.
pub(super) fn global_settings(fps: f64, stop: i64) -> Node<'static> {
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

/// `Takes`: legacy list of animation clips (pre-7.0 files). Modern importers
/// use `AnimationStack`; it is written for compatibility with the FBX SDK.
///
/// ```text
/// Takes: {
///     Current: "getup"
///     Take: "getup" { FileName: "getup.tak"  LocalTime: 0, <stop>  ReferenceTime: 0, <stop> }
/// }
/// ```
pub(super) fn takes(take_name: &str, stop: i64) -> Node<'_> {
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
/// | value | frame rate | value | frame rate |
/// |---|---|---|---|
/// | 0 | default (30) | 10 | 25 (PAL) |
/// | 1 | 120 | 11 | 24 (cinema) |
/// | 2 | 100 | 12 | 1000 |
/// | 3 | 60 | 13 | 23.976 |
/// | 4 | 50 | 14 | custom: `CustomFrameRate` |
/// | 5 | 48 | 15 | 96 |
/// | 6 | 30 | 16 | 72 |
/// | 7 | 30 (drop frame) | 17 | 59.94 |
/// | 8, 9 | 29.97 (NTSC) | 18 | 119.88 |
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

