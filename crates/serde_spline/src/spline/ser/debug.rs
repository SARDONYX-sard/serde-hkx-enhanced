use crate::spline::math::TransformMask;
use crate::spline::math::{
    SplineTrackQuat, SplineTrackVector, TransformSplineBlock, TransformTrack,
};

#[derive(Debug)]
pub(crate) struct SerializeDebugTrack {
    pub position_offset: usize,
    pub position_size: usize,
    pub rotation_offset: usize,
    pub rotation_size: usize,
    pub scale_offset: usize,
    pub scale_size: usize,
}

const fn track_kind_string(track: &TransformTrack) -> (&'static str, &'static str, &'static str) {
    let position = match &track.position {
        SplineTrackVector::Static(_) => "Static",
        SplineTrackVector::Dynamic(_) => "Dynamic",
    };

    let rotation = match &track.rotation {
        SplineTrackQuat::Identity => "Identity",
        SplineTrackQuat::Static(_) => "Static",
        SplineTrackQuat::Dynamic(_) => "Dynamic",
    };

    let scale = match &track.scale {
        SplineTrackVector::Static(_) => "Static",
        SplineTrackVector::Dynamic(_) => "Dynamic",
    };

    (position, rotation, scale)
}

fn format_track_debug(
    index: usize,
    mask: &TransformMask,
    track: &TransformTrack,
    debug: &SerializeDebugTrack,
) -> String {
    use crate::spline::math::TransformType;

    let (position_kind, rotation_kind, scale_kind) = track_kind_string(track);

    let position_quantization = mask
        .position_quantization_type()
        .map_or_else(|_| "Invalid".to_owned(), |value| format!("{value:?}"));

    let rotation_quantization = mask
        .rotation_quantization_type()
        .map_or_else(|_| "Invalid".to_owned(), |value| format!("{value:?}"));

    let scale_quantization = mask
        .scale_quantization_type()
        .map_or_else(|_| "Invalid".to_owned(), |value| format!("{value:?}"));

    format!(
        "\n\
║ Track {index:<4}
║
║   MASK
║     Position : {:?} / {:<10}
║     Rotation : {:?} / {:<10}
║     Scale    : {:?} / {:<10}
║
║   DATA
║     Position : {position_kind:<8}
║     Rotation : {rotation_kind:<8}
║     Scale    : {scale_kind:<8}
║
║   SERIALIZED
║     Position : 0x{position_offset:08x}..0x{position_end:08x} ({position_size:>6} bytes)
║     Rotation : 0x{rotation_offset:08x}..0x{rotation_end:08x} ({rotation_size:>6} bytes)
║     Scale    : 0x{scale_offset:08x}..0x{scale_end:08x} ({scale_size:>6} bytes)
║                                                                ",
        mask.sub_track_type(TransformType::PosX),
        position_quantization,
        mask.sub_track_type(TransformType::Rotation),
        rotation_quantization,
        mask.sub_track_type(TransformType::ScaleX),
        scale_quantization,
        position_end = debug.position_offset + debug.position_size,
        rotation_end = debug.rotation_offset + debug.rotation_size,
        scale_end = debug.scale_offset + debug.scale_size,
        position_offset = debug.position_offset,
        rotation_offset = debug.rotation_offset,
        scale_offset = debug.scale_offset,
        position_size = debug.position_size,
        rotation_size = debug.rotation_size,
        scale_size = debug.scale_size,
    )
}

pub(crate) fn log_serialized_block(
    block: &TransformSplineBlock,
    block_index: usize,
    block_start: usize,
    block_end: usize,
    tracks: &[SerializeDebugTrack],
) {
    let mut output = String::new();

    output.push_str(&format!(
        "\n\
╔════════════════════════════════════════════════════════════════
║ Spline Serializer
╠════════════════════════════════════════════════════════════════
║ Block {block_index:<4}
║   Range : 0x{block_start:08x}..0x{block_end:08x}
║   Size  : {:>6} bytes
║   Tracks: {:>6}
║                                                                ",
        block_end - block_start,
        block.tracks.len(),
    ));

    for (index, ((mask, track), debug)) in block
        .masks
        .iter()
        .zip(&block.tracks)
        .zip(tracks)
        .enumerate()
    {
        output.push_str(&format_track_debug(index, mask, track, debug));
    }

    output.push_str(
        "\n\
╚════════════════════════════════════════════════════════════════\n",
    );

    tracing::debug!("{output}");
}
